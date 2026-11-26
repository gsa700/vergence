//! The libraries AlbumWall knows (milestone 1: folders; Navidrome servers in
//! milestone 2). As in the Avalonia app there may be several, one on the wall
//! at a time and never merged; the picker in the bottom bar switches.
//!
//! Kept in `~/.config/vergence/albumwall/libraries`, apart from the Avalonia
//! AlbumWall's settings (question 5). A small text file of our own: a
//! comment, the library on the wall, then one library a line, its kind, name
//! and folder separated by tabs. The folder is the line's last field, so a
//! tab in a folder's name survives; a newline cannot be kept and is refused.

use std::path::{Path, PathBuf};

/// One library: a music folder (a local disk, an external drive, a share).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Library {
    /// What the picker shows: the folder's own name unless renamed.
    pub name: String,
    pub folder: PathBuf,
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
        self.list.push(Library { name, folder: folder.to_path_buf() });
        self.current = self.list.len() - 1;
        Ok(())
    }

    /// The file's text.
    pub fn to_text(&self) -> String {
        let mut s = format!("{HEADER}\ncurrent\t{}\n", self.current);
        for l in &self.list {
            s.push_str(&format!("folder\t{}\t{}\n", l.name, l.folder.to_string_lossy()));
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
            let mut parts = line.splitn(3, '\t');
            match (parts.next(), parts.next(), parts.next()) {
                (Some("current"), Some(n), None) => libs.current = n.trim().parse().unwrap_or(0),
                (Some("folder"), Some(name), Some(folder)) if !folder.is_empty() => {
                    libs.list.push(Library { name: name.to_string(), folder: PathBuf::from(folder) })
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
        std::fs::write(&tmp, self.to_text())?;
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
