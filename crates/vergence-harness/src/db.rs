//! Reading the linuxhw/EDID database: one text file per EDID, starting with
//! "edid-decode (hex):", a blank line, then the blob as hex bytes, blocks
//! separated by blank lines, ended by a line of dashes.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Every EDID file under `root` (skipping .git and the *.md indexes), sorted
/// so runs are repeatable.
pub fn files(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') {
                continue;
            }
            let ft = entry.file_type()?;
            if ft.is_dir() {
                stack.push(path);
            } else if ft.is_file() && path.parent() != Some(root) {
                out.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// The EDID bytes of one database file, or None if it has no hex block.
pub fn read_edid(path: &Path) -> io::Result<Option<Vec<u8>>> {
    let text = fs::read_to_string(path)?;
    Ok(hex_block(&text))
}

/// Parse the hex block that follows "edid-decode (hex):".
pub fn hex_block(text: &str) -> Option<Vec<u8>> {
    let mut lines = text.lines();
    lines.find(|l| l.trim() == "edid-decode (hex):")?;
    let mut bytes = Vec::new();
    for line in lines {
        let line = line.trim();
        if line.starts_with("---") {
            break;
        }
        for tok in line.split_whitespace() {
            if tok.len() != 2 {
                return None;
            }
            bytes.push(u8::from_str_radix(tok, 16).ok()?);
        }
    }
    if bytes.is_empty() { None } else { Some(bytes) }
}

#[cfg(test)]
mod tests {
    use super::hex_block;

    #[test]
    fn reads_two_blocks() {
        let t = "edid-decode (hex):\n\n00 ff ff ff\n\n01 02\n\n----------------\n\nBlock 0\n";
        assert_eq!(hex_block(t), Some(vec![0x00, 0xff, 0xff, 0xff, 0x01, 0x02]));
    }

    #[test]
    fn no_block() {
        assert_eq!(hex_block("something else\n00 ff\n"), None);
        assert_eq!(hex_block("edid-decode (hex):\n\n----\n"), None);
        assert_eq!(hex_block("edid-decode (hex):\n\nzz ff\n"), None);
    }
}
