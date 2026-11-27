//! The libraries AlbumWall knows (milestone 1: folders; Navidrome servers in
//! milestone 2). As in the Avalonia app there may be several, one on the wall
//! at a time and never merged; the picker in the bottom bar switches.
//!
//! Kept in `~/.config/vergence/albumwall/libraries`, apart from the Avalonia
//! AlbumWall's settings (question 5). A small text file of our own: a
//! comment, the library on the wall, then one library a line, its kind, name
//! and folder separated by tabs. The folder is the line's last field, so a
//! tab in a folder's name survives; a newline cannot be kept and is refused.
//!
//! A music server (milestone 2) is a line of its own: `server`, its name,
//! address, user, salt and token, and the server's library chosen (empty for
//! all). No password is kept (question 3), but the token signs in to that
//! server, so the file is readable by its owner only.

use std::path::{Path, PathBuf};

/// One library: a music folder (a local disk, an external drive, a share),
/// or a music server (`server`; its `folder` is then the server's key).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Library {
    /// What the picker shows: the folder's own name unless renamed.
    pub name: String,
    pub folder: PathBuf,
    pub server: Option<crate::subsonic::Login>,
}

/// Every library, and which one is on the wall.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Libraries {
    pub list: Vec<Library>,
    pub current: usize,
}

const HEADER: &str = "# AlbumWall for Vergence: its libraries (kind, name, folder; tab-separated)";

impl Libraries {
    /// The one on the wall, if there is any.
    pub fn current(&self) -> Option<&Library> {
        self.list.get(self.current)
    }

    /// A folder added and put on the wall; one already known is only put on
    /// the wall. Refused if its path holds a newline (the file could not keep it).
    pub fn add_folder(&mut self, folder: &Path) -> Result<(), String> {
        if folder.to_string_lossy().contains('\n') {
            return Err("a folder whose name holds a line break cannot be kept".into());
        }
        if let Some(i) = self.list.iter().position(|l| l.folder == folder) {
            self.current = i;
            return Ok(());
        }
        let name = folder
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| folder.to_string_lossy().into_owned())
            .replace(['\t', '\n'], " ");
        self.list.push(Library { name, folder: folder.to_path_buf(), server: None });
        self.current = self.list.len() - 1;
        Ok(())
    }

    /// A music server added and put on the wall (one already known, the same
    /// user and library, has its sign-in renewed and is put on the wall).
    pub fn add_server(&mut self, name: &str, login: crate::subsonic::Login) {
        let key = login.key();
        let name = name.replace(['\t', '\n'], " ");
        if let Some(i) = self.list.iter().position(|l| l.folder == key) {
            self.list[i].server = Some(login);
            self.current = i;
            return;
        }
        self.list.push(Library { name, folder: key, server: Some(login) });
        self.current = self.list.len() - 1;
    }

    /// The file's text.
    pub fn to_text(&self) -> String {
        let mut s = format!("{HEADER}\ncurrent\t{}\n", self.current);
        for l in &self.list {
            match &l.server {
                Some(g) => s.push_str(&format!(
                    "server\t{}\t{}\t{}\t{}\t{}\t{}\n",
                    l.name,
                    g.address,
                    g.user.replace(['\t', '\n'], " "),
                    g.salt,
                    g.token,
                    g.folder.as_deref().unwrap_or("")
                )),
                None => s.push_str(&format!("folder\t{}\t{}\n", l.name, l.folder.to_string_lossy())),
            }
        }
        s
    }

    /// Read from the file's text; lines it does not know are skipped (a
    /// later version's kinds), and `current` is kept in range.
    pub fn from_text(text: &str) -> Libraries {
        let mut libs = Libraries::default();
        for line in text.lines() {
            if line.starts_with('#') {
                continue;
            }
            if let Some(rest) = line.strip_prefix("server\t") {
                let f: Vec<&str> = rest.split('\t').collect();
                if f.len() == 6 && !f[1].is_empty() && !f[2].is_empty() && !f[4].is_empty() {
                    let login = crate::subsonic::Login {
                        address: f[1].into(),
                        user: f[2].into(),
                        salt: f[3].into(),
                        token: f[4].into(),
                        folder: Some(f[5].to_string()).filter(|x| !x.is_empty()),
                    };
                    libs.list.push(Library { name: f[0].into(), folder: login.key(), server: Some(login) });
                }
                continue;
            }
            let mut parts = line.splitn(3, '\t');
            match (parts.next(), parts.next(), parts.next()) {
                (Some("current"), Some(n), None) => libs.current = n.trim().parse().unwrap_or(0),
                (Some("folder"), Some(name), Some(folder)) if !folder.is_empty() => {
                    libs.list.push(Library { name: name.to_string(), folder: PathBuf::from(folder), server: None })
                }
                _ => {}
            }
        }
        if libs.current >= libs.list.len() {
            libs.current = 0;
        }
        libs
    }

    /// Read from `path`; none if it is missing or unreadable.
    pub fn load(path: &Path) -> Libraries {
        std::fs::read_to_string(path).map(|t| Libraries::from_text(&t)).unwrap_or_default()
    }

    /// Written to `path` whole: a new file beside it renamed over it, so a
    /// crash leaves the old list or the new one, never half of either.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("new");
        // Readable by its owner only: a server's token is in it.
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
            f.write_all(self.to_text().as_bytes())?;
        }
        std::fs::set_permissions(&tmp, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
        std::fs::rename(&tmp, path)
    }
}

/// `~/.config/vergence/albumwall` (or under `$XDG_CONFIG_HOME`).
pub fn config_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("vergence/albumwall"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_round_trip_with_the_one_on_the_wall() {
        let mut libs = Libraries::default();
        libs.add_folder(Path::new("/music/CDs")).unwrap();
        libs.add_folder(Path::new("/mnt/nas/Music\twith a tab")).unwrap();
        libs.current = 0;
        let back = Libraries::from_text(&libs.to_text());
        assert_eq!(back, libs);
        assert_eq!(back.list[1].folder, Path::new("/mnt/nas/Music\twith a tab"));
        assert_eq!(back.list[1].name, "Music with a tab");
        assert_eq!(back.current().unwrap().name, "CDs");
    }

    #[test]
    fn a_folder_already_known_is_only_put_on_the_wall() {
        let mut libs = Libraries::default();
        libs.add_folder(Path::new("/a")).unwrap();
        libs.add_folder(Path::new("/b")).unwrap();
        libs.add_folder(Path::new("/a")).unwrap();
        assert_eq!(libs.list.len(), 2);
        assert_eq!(libs.current, 0);
    }

    #[test]
    fn unknown_lines_are_skipped_and_current_kept_in_range() {
        let libs = Libraries::from_text("# x\ncurrent\t7\nserver\tNAS\thttp://x\nfolder\tM\t/m\nfolder\tEmpty\t\n");
        assert_eq!(libs.list.len(), 1);
        assert_eq!(libs.current, 0);
    }

    #[test]
    fn a_server_round_trips_and_the_file_is_its_owners() {
        let mut libs = Libraries::default();
        libs.add_folder(Path::new("/music")).unwrap();
        let login = crate::subsonic::Login {
            address: "http://10.0.1.5:4533".into(),
            user: "dave".into(),
            salt: "c19b2d00aa11".into(),
            token: "26719a1196d2a940705a59634eb18eab".into(),
            folder: Some("2".into()),
        };
        libs.add_server("MusicServer", login.clone());
        assert_eq!(libs.current, 1);
        assert_eq!(libs.list[1].folder, Path::new("navidrome://dave@10.0.1.5:4533/folder/2"));
        let back = Libraries::from_text(&libs.to_text());
        assert_eq!(back, libs);
        // Signed in again: the same library, its sign-in renewed.
        libs.add_server("x", crate::subsonic::Login { token: "new".into(), ..login });
        assert_eq!(libs.list.len(), 2);
        assert_eq!(libs.list[1].server.as_ref().unwrap().token, "new");
        let dir = std::env::temp_dir().join(format!("albumwall-libs-mode-{}", std::process::id()));
        let path = dir.join("libraries");
        libs.save(&path).unwrap();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_line_break_in_a_folder_is_refused() {
        let mut libs = Libraries::default();
        assert!(libs.add_folder(Path::new("/bad\nname")).is_err());
        assert!(libs.list.is_empty());
    }

    #[test]
    fn saving_writes_the_whole_file_in_place() {
        let dir = std::env::temp_dir().join(format!("albumwall-libs-{}", std::process::id()));
        let path = dir.join("libraries");
        let mut libs = Libraries::default();
        libs.add_folder(Path::new("/music")).unwrap();
        libs.save(&path).unwrap();
        assert_eq!(Libraries::load(&path), libs);
        assert!(!path.with_extension("new").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
