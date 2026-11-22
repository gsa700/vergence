//! Is any of this machine's storage encrypted? (VDM review M6: automatic
//! login is never allowed with any encrypted file system, answer 4 a in
//! docs/vdm-milestone-1.md: it would unlock at power on what the encryption
//! was there to keep.) Checked at every boot, not only
//! when automatic login is turned on. Counts: dm-crypt (LUKS or plain) on
//! Linux, GELI on FreeBSD, ZFS native encryption on either, even when a
//! TPM unlocks it; Linux eCryptfs mounts and volumes listed in /etc/crypttab
//! (open or not); GELI providers anywhere under /dev. What cannot be told
//! counts against (threat model T14: "if it cannot tell, it is not
//! offered"; security review 2026-11-16, finding 7).

/// What was found encrypted, said in words (empty: nothing), or why it
/// could not be told.
pub fn found() -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    // Linux: device-mapper devices whose uuid dm-crypt made ("CRYPT-LUKS2-...").
    for e in std::fs::read_dir("/sys/block").into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if let Ok(uuid) = std::fs::read_to_string(e.path().join("dm/uuid"))
            && uuid.starts_with("CRYPT-")
        {
            let kind = uuid.split('-').nth(1).unwrap_or("dm-crypt").to_string();
            out.push(format!("{name} ({kind})"));
        }
    }
    // FreeBSD: GELI providers appear as <name>.eli under /dev, also in its
    // gpt/, label/, gptid/ and diskid/ directories.
    let mut dirs = vec![std::path::PathBuf::from("/dev")];
    let mut depth = 0;
    while !dirs.is_empty() && depth < 3 {
        let mut next = Vec::new();
        for d in dirs {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.ends_with(".eli") {
                    out.push(format!("{} (GELI)", e.path().display()));
                } else if depth == 0 && ["gpt", "label", "gptid", "diskid", "ufs", "mapper"].contains(&name.as_str()) {
                    next.push(e.path());
                }
            }
        }
        dirs = next;
        depth += 1;
    }
    // Linux: eCryptfs mounts; and any volume /etc/crypttab names, opened at
    // boot or not (a home unlocked later by pam_mount counts too).
    if let Ok(m) = std::fs::read_to_string("/proc/mounts") {
        out.extend(
            m.lines()
                .filter(|l| l.split_whitespace().nth(2) == Some("ecryptfs"))
                .map(|l| format!("{} (eCryptfs)", l.split_whitespace().nth(1).unwrap_or("?"))),
        );
    }
    if let Ok(t) = std::fs::read_to_string("/etc/crypttab") {
        out.extend(
            t.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(|l| format!("{} (crypttab)", l.split_whitespace().next().unwrap_or("?"))),
        );
    }
    // ZFS native encryption, on either, where ZFS is there.
    for zfs in ["/sbin/zfs", "/usr/sbin/zfs", "/usr/bin/zfs"] {
        if !std::path::Path::new(zfs).is_file() {
            continue;
        }
        match std::process::Command::new(zfs).args(["get", "-H", "-o", "name,value", "encryption"]).env_clear().output()
        {
            Ok(o) if o.status.success() => out.extend(zfs_encrypted(&String::from_utf8_lossy(&o.stdout))),
            Ok(o) => {
                return Err(format!(
                    "zfs could not say whether its datasets are encrypted ({})",
                    String::from_utf8_lossy(&o.stderr).trim()
                ));
            }
            Err(e) => return Err(format!("zfs could not be asked ({e})")),
        }
        break;
    }
    out.dedup();
    Ok(out)
}

/// The datasets `zfs get -H -o name,value encryption` says are encrypted.
pub fn zfs_encrypted(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| {
            let (name, value) = l.split_once('\t')?;
            (value.trim() != "off" && value.trim() != "-").then(|| format!("{name} (ZFS {})", value.trim()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zfs_encryption_is_read() {
        assert!(zfs_encrypted("zroot\toff\nzroot/ROOT\toff\n").is_empty());
        assert_eq!(
            zfs_encrypted("tank\toff\ntank/home\taes-256-gcm\n"),
            vec!["tank/home (ZFS aes-256-gcm)".to_string()]
        );
    }
}
