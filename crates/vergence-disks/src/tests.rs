use super::*;

/// kepler's own GEOM picture (FreeBSD 15.1, one NVMe disk, ZFS on root; serial and UUIDs
/// replaced): the disk, its four GPT partitions with their types, the labels that name them,
/// and the one swapped on.
#[test]
fn freebsd_disks_from_geom() {
    let list = geom::disks(include_str!("../testdata/kepler-geom.xml"), false);
    assert_eq!(list.len(), 1);
    let d = &list[0];
    assert_eq!(d.name, "nda0");
    assert_eq!(d.model, "Micron 2210 MTFDHBA512QFD");
    assert_eq!(d.serial, "SERIAL0001");
    assert_eq!(d.size, 512_110_190_592);
    assert_eq!(d.bus, "NVMe");
    assert_eq!(d.kind, Some(Kind::Internal));
    assert_eq!(d.rotational, Some(false));
    assert_eq!(d.scheme, "GPT");
    let names: Vec<&str> = d.parts.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["nda0p1", "nda0p2", "nda0p3", "nda0p4"]);
    let types: Vec<&str> = d.parts.iter().map(|p| p.ptype.as_str()).collect();
    assert_eq!(types, ["EFI system", "FreeBSD boot", "FreeBSD swap", "FreeBSD ZFS"]);
    let efi = &d.parts[0];
    assert!(efi.aliases.contains(&"gpt/efiboot0".to_string()), "{:?}", efi.aliases);
    assert_eq!(efi.fs, "FAT32");
    assert_eq!(d.parts[2].role, "Swap, in use");
    assert_eq!(d.parts[2].fs, "Swap");
    assert_eq!(d.parts[3].fs, "ZFS");
    assert_eq!(d.parts[3].start, 4_728_832 * 512);
    assert_eq!(d.parts[3].size, 509_688_676_352);
    // Partitions fill the disk: no free segment but alignment's.
    assert!(d.segments().iter().all(|s| matches!(s, Segment::Part(_))), "{:?}", d.segments());
}

/// A USB stick under FreeBSD: `da` with umass attached is external; a bare FAT file system
/// (no table) is the whole disk's, its label from LABEL's msdosfs name.
#[test]
fn a_freebsd_stick_with_no_table() {
    let xml = r#"<mesh>
  <class id="1"><name>LABEL</name>
    <geom id="2"><class ref="1"/><name>da0</name>
      <consumer id="3"><provider ref="10"/></consumer>
      <provider id="4"><name>msdosfs/STICK</name><mediasize>31000000000</mediasize></provider>
    </geom>
  </class>
  <class id="5"><name>DISK</name>
    <geom id="6"><class ref="5"/><name>da0</name>
      <provider id="10"><name>da0</name><mediasize>31000000000</mediasize><sectorsize>512</sectorsize>
        <config><rotationrate>unknown</rotationrate><ident>X</ident><descr>SanDisk Ultra</descr></config>
      </provider>
    </geom>
  </class>
</mesh>"#;
    let list = geom::disks(xml, true);
    assert_eq!(list.len(), 1);
    let d = &list[0];
    assert_eq!((d.bus.as_str(), d.kind), ("USB", Some(Kind::External)));
    assert_eq!(d.rotational, None);
    let w = d.whole.as_ref().expect("a whole-disk file system");
    assert_eq!((w.fs.as_str(), w.label.as_str()), ("FAT32", "STICK"));
    assert!(geom::disks(xml, false)[0].bus == "SCSI");
}

/// Malformed text never panics and gives what it could read.
#[test]
fn geom_reader_survives_bad_text() {
    for bad in ["", "<", "<mesh><class>", "</mesh>", "<mesh><class><name>DISK</name><geom><provider>", "<<>>&amp;"] {
        let _ = geom::disks(bad, false);
    }
    let t = geom::parse("<a id=\"1\"><b>x &amp; y</b><c ref=\"2\"/></a>");
    let a = &t.kids[0];
    assert_eq!((a.name.as_str(), a.id.as_str()), ("a", "1"));
    assert_eq!(a.kids[0].text, "x & y");
    assert_eq!(a.kids[1].r#ref, "2");
}

#[test]
fn zpool_list_and_status() {
    let pools = parse_zpool_list("zroot\t506806140928\t42632736768\t464173404160\tONLINE\n");
    assert_eq!(pools.len(), 1);
    let mut p = pools[0].clone();
    assert_eq!((p.size, p.alloc, p.free), (506_806_140_928, 42_632_736_768, 464_173_404_160));
    parse_zpool_status(include_str!("../testdata/kepler-zpool-status.txt"), &mut p);
    assert_eq!(p.members, vec![("nda0p4".to_string(), "ONLINE".to_string())]);
    assert_eq!(p.errors, "No known data errors");
    assert_eq!(p.scan, "");
    assert!(member_is("/dev/nda0p4", "nda0p4") && !member_is("nda0p4", "nda0p3"));
}

/// A mirror: its disks are members, the group line is not; a scrub's two-line scan joined.
#[test]
fn zpool_status_of_a_mirror() {
    let text = "  pool: tank\n state: DEGRADED\n  scan: scrub in progress since Sun Nov 22 10:00:00 2026\n\t1.2T / 3.4T scanned, 35% done\nconfig:\n\n\tNAME        STATE     READ WRITE CKSUM\n\ttank        DEGRADED     0     0     0\n\t  mirror-0  DEGRADED     0     0     0\n\t    ada0p3  ONLINE       0     0     0\n\t    ada1p3  FAULTED      3     0     0\n\nerrors: No known data errors\n";
    let mut p = Pool { name: "tank".into(), ..Default::default() };
    parse_zpool_status(text, &mut p);
    assert_eq!(p.members, vec![("ada0p3".into(), "ONLINE".into()), ("ada1p3".into(), "FAULTED".into())]);
    assert_eq!(p.scan, "scrub in progress since Sun Nov 22 10:00:00 2026 1.2T / 3.4T scanned, 35% done");
}

/// techbench's EFI partition as udev records it.
#[test]
fn udev_records_and_words() {
    let u = parse_udev(
        "S:disk/by-uuid/1DF4-2D13\nE:ID_FS_TYPE=vfat\nE:ID_FS_VERSION=FAT32\nE:ID_PART_ENTRY_TYPE=c12a7328-f81f-11d2-ba4b-00a0c93ec93b\nE:ID_FS_LABEL=My Stick\nG:systemd\n",
    );
    assert_eq!(u.get("ID_FS_TYPE").map(String::as_str), Some("vfat"));
    assert_eq!(u.get("ID_FS_LABEL").map(String::as_str), Some("My Stick"));
    assert_eq!(fs_word("vfat"), "FAT32");
    assert_eq!(fs_word("crypto_LUKS"), "Encrypted (LUKS)");
    assert_eq!(ptype_word(&u["ID_PART_ENTRY_TYPE"]), "EFI system");
    assert_eq!(ptype_word("0FC63DAF-8483-4772-8E79-3D69D8477DE4"), "Linux file system");
    assert_eq!(ptype_word("0x83"), "Linux file system");
    assert_eq!(ptype_word("freebsd-zfs"), "FreeBSD ZFS");
    assert_eq!(ptype_word("!12345"), "!12345");
}

/// udev's escaped names: Hubble's Windows disk (GPT names) and a stick whose label has spaces.
#[test]
fn udev_labels_unescaped() {
    let gpt = parse_udev("E:ID_PART_ENTRY_NAME=Basic\\x20data\\x20partition\n");
    assert_eq!(udev_label(&gpt), "Basic data partition");
    let stick = parse_udev("E:ID_FS_LABEL=My_Stick\nE:ID_FS_LABEL_ENC=My\\x20Stick\nE:ID_PART_ENTRY_NAME=x\n");
    assert_eq!(udev_label(&stick), "My Stick");
    let hostile = parse_udev("E:ID_FS_LABEL_ENC=a\\x1b[2Jb\\x0a\\xc3\\xa9\\xzz\n");
    assert_eq!(udev_label(&hostile), "a[2Jb\u{e9}\\xzz");
    assert_eq!(udev_label(&parse_udev("")), "");
}

#[test]
fn mount_tables() {
    let m = parse_proc_mounts("/dev/sdb1 /run/media/d/My\\040Stick vfat rw 0 0\n/dev/nvme0n1p3 / btrfs rw 0 0\n");
    assert_eq!(m[0], ("/dev/sdb1".into(), "/run/media/d/My Stick".into(), "vfat".into()));
    let fs =
        vec![vergence_sysinfo::FileSystem { mount: "/".into(), source: "/dev/nvme0n1p3".into(), total: 100, free: 40 }];
    let mut p = Part { name: "nvme0n1p3".into(), ..Default::default() };
    attach_mount(&mut p, &[("/dev/nvme0n1p3".into(), "/home".into(), "btrfs".into()), m[1].clone()], &fs);
    assert_eq!(p.mount.as_deref(), Some("/"), "the shortest of several mounts (Btrfs subvolumes)");
    assert_eq!(p.usage, Some((100, 40)));
    assert_eq!(p.fs, "Btrfs");
    let mut efi = Part { name: "nda0p1".into(), aliases: vec!["gpt/efiboot0".into()], ..Default::default() };
    attach_mount(&mut efi, &parse_mount_p("/dev/gpt/efiboot0\t/boot/efi\tmsdosfs\trw\t2 2\n"), &[]);
    assert_eq!(efi.mount.as_deref(), Some("/boot/efi"));
}

#[test]
fn the_bar_shows_real_gaps_only() {
    const M: u64 = 1 << 20;
    let d = Disk {
        size: 1000 * M,
        parts: vec![
            Part { start: 500 * M, size: 100 * M, ..Default::default() },
            Part { start: M, size: 99 * M, ..Default::default() },
        ],
        ..Default::default()
    };
    assert_eq!(
        d.segments(),
        vec![
            Segment::Part(1),
            Segment::Free { start: 100 * M, size: 400 * M },
            Segment::Part(0),
            Segment::Free { start: 600 * M, size: 400 * M },
        ]
    );
}
