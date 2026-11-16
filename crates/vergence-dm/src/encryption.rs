//! Is any of this machine's storage encrypted? (VDM review M6: automatic
//! login is never allowed with any encrypted file system, Dave's "4 a but
//! also not allowed with any encrypted FS": it would unlock at power on
//! what the encryption was there to keep.) Checked at every boot, not only
//! when automatic login is turned on. Counts: dm-crypt (LUKS or plain) on
//! Linux, GELI on FreeBSD, ZFS native encryption on either, even when a
//! TPM unlocks it.

/// What was found encrypted, said in words (empty: nothing).
pub fn found() -> Vec<String> {
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
    // FreeBSD: GELI providers appear as /dev/<name>.eli.
    for e in std::fs::read_dir("/dev").into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.ends_with(".eli") {
            out.push(format!("{name} (GELI)"));
        }
    }
    // ZFS native encryption, on either, where ZFS is there.
    for zfs in ["/sbin/zfs", "/usr/sbin/zfs", "/usr/bin/zfs"] {
        if !std::path::Path::new(zfs).is_file() {
            continue;
        }
        if let Ok(o) = std::process::Command::new(zfs).args(["get", "-H", "-o", "name,value", "encryption"]).env_clear().output() {
            out.extend(zfs_encrypted(&String::from_utf8_lossy(&o.stdout)));
        }
        break;
    }
    out
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
        assert_eq!(zfs_encrypted("tank\toff\ntank/home\taes-256-gcm\n"), vec!["tank/home (ZFS aes-256-gcm)".to_string()]);
    }
}
