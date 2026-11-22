// `iso OUT LABEL`: a small ISO 9660 image (drives milestone 1, for the
// display suite's disk-image scenario): the volume `LABEL`, one file at its
// root, HELLO.TXT. Written by hand, so a check needs no xorriso or makefs:
// sixteen empty sectors, the primary volume descriptor, its terminator, the
// path tables (little- and big-endian), the root directory, the file.

use std::path::Path;

const SECTOR: usize = 2048;
const PVD: usize = 16;
const TERM: usize = 17;
const L_TABLE: usize = 18;
const M_TABLE: usize = 19;
const ROOT: usize = 20;
const FILE: usize = 21;
const SECTORS: usize = 22;
const TEXT: &[u8] = b"hello from a Vergence disk image\n";

fn both32(b: &mut Vec<u8>, v: u32) {
    b.extend(v.to_le_bytes());
    b.extend(v.to_be_bytes());
}

fn both16(b: &mut Vec<u8>, v: u16) {
    b.extend(v.to_le_bytes());
    b.extend(v.to_be_bytes());
}

/// A directory record: its extent, length, whether a directory, its name.
fn record(extent: usize, len: usize, dir: bool, id: &[u8]) -> Vec<u8> {
    let mut r = vec![0u8, 0];
    both32(&mut r, extent as u32);
    both32(&mut r, len as u32);
    // Recorded 2026-11-21 12:00:00 UTC: years since 1900, month, day, hour, minute, second, offset.
    r.extend([126, 11, 21, 12, 0, 0, 0]);
    r.push(if dir { 2 } else { 0 });
    r.extend([0, 0]);
    both16(&mut r, 1);
    r.push(id.len() as u8);
    r.extend(id);
    if r.len() % 2 == 1 {
        r.push(0);
    }
    r[0] = r.len() as u8;
    r
}

/// A string field of `n` bytes, padded with spaces.
fn field(s: &str, n: usize) -> Vec<u8> {
    let mut b: Vec<u8> = s.bytes().take(n).collect();
    b.resize(n, b' ');
    b
}

pub fn image(label: &str) -> Vec<u8> {
    let mut img = vec![0u8; SECTORS * SECTOR];
    let label: String = label.chars().map(|c| c.to_ascii_uppercase()).filter(|c| c.is_ascii_alphanumeric() || *c == '_').take(32).collect();
    // The path table: the root alone (name 0, parent 1), eight bytes and a pad.
    let table = |extent: u32, le: bool| -> Vec<u8> {
        let mut t = vec![1u8, 0];
        t.extend(if le { extent.to_le_bytes() } else { extent.to_be_bytes() });
        t.extend(if le { 1u16.to_le_bytes() } else { 1u16.to_be_bytes() });
        t.extend([0, 0]);
        t
    };
    let mut pvd = vec![1u8];
    pvd.extend(b"CD001");
    pvd.push(1);
    pvd.push(0);
    pvd.extend(field("", 32));
    pvd.extend(field(&label, 32));
    pvd.extend([0u8; 8]);
    both32(&mut pvd, SECTORS as u32);
    pvd.extend([0u8; 32]);
    both16(&mut pvd, 1);
    both16(&mut pvd, 1);
    both16(&mut pvd, SECTOR as u16);
    both32(&mut pvd, 10);
    pvd.extend((L_TABLE as u32).to_le_bytes());
    pvd.extend([0u8; 4]);
    pvd.extend((M_TABLE as u32).to_be_bytes());
    pvd.extend([0u8; 4]);
    pvd.extend(record(ROOT, SECTOR, true, &[0]));
    for n in [128, 128, 128, 128, 37, 37, 37] {
        pvd.extend(field("", n));
    }
    for _ in 0..4 {
        pvd.extend(b"0000000000000000");
        pvd.push(0);
    }
    pvd.push(1);
    assert_eq!(pvd.len(), 882, "the descriptor's fields add up");
    img[PVD * SECTOR..PVD * SECTOR + pvd.len()].copy_from_slice(&pvd);
    let term = [255u8, b'C', b'D', b'0', b'0', b'1', 1];
    img[TERM * SECTOR..TERM * SECTOR + term.len()].copy_from_slice(&term);
    for (at, le) in [(L_TABLE, true), (M_TABLE, false)] {
        let t = table(ROOT as u32, le);
        img[at * SECTOR..at * SECTOR + t.len()].copy_from_slice(&t);
    }
    let mut dir = record(ROOT, SECTOR, true, &[0]);
    dir.extend(record(ROOT, SECTOR, true, &[1]));
    dir.extend(record(FILE, TEXT.len(), false, b"HELLO.TXT;1"));
    img[ROOT * SECTOR..ROOT * SECTOR + dir.len()].copy_from_slice(&dir);
    img[FILE * SECTOR..FILE * SECTOR + TEXT.len()].copy_from_slice(TEXT);
    img
}

pub fn run(args: &[String]) -> Result<bool, Box<dyn std::error::Error>> {
    let [out, label] = args else { return Err("iso OUT LABEL".into()) };
    std::fs::write(Path::new(out), image(label))?;
    println!("{out}: an ISO 9660 image, volume {label}, with HELLO.TXT");
    Ok(true)
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_image_has_its_descriptors_where_they_belong() {
        let img = super::image("vtest");
        assert_eq!(img.len(), super::SECTORS * super::SECTOR);
        assert_eq!(&img[32769..32774], b"CD001", "what vergence-media looks for");
        assert_eq!(&img[16 * 2048 + 40..16 * 2048 + 45], b"VTEST");
        assert_eq!(img[17 * 2048], 255);
    }
}
