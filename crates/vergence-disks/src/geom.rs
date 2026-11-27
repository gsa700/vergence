//! FreeBSD's GEOM picture (`sysctl -n kern.geom.confxml`) read into disks and partitions.
//!
//! The text is the kernel's, in a fixed shape: `<class>`es (DISK, PART, LABEL, SWAP, ...), each
//! with `<geom>`s holding `<provider>`s (what it offers: "nda0p1") and `<consumer>`s (what it sits
//! on, by `<provider ref=...>`). A small tree reader of our own for that shape, not a general XML
//! parser: elements, `id`/`ref` attributes and text; entities as the kernel writes them.
//!
//! Compiled on every system so the tests run everywhere; called on FreeBSD.

#![cfg_attr(not(target_os = "freebsd"), allow(dead_code))]

use std::collections::HashMap;

use crate::{Disk, Kind, Part, fs_word, ptype_word};

/// One element: its name, `id` or `ref` attribute, text, children.
#[derive(Debug, Default)]
pub struct El {
    pub name: String,
    pub id: String,
    pub r#ref: String,
    pub text: String,
    pub kids: Vec<El>,
}

impl El {
    fn kid(&self, name: &str) -> Option<&El> {
        self.kids.iter().find(|k| k.name == name)
    }
    fn kids<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a El> + 'a {
        self.kids.iter().filter(move |k| k.name == name)
    }
    /// A child's text, trimmed ("" when there is none).
    fn get(&self, name: &str) -> &str {
        self.kid(name).map_or("", |k| k.text.trim())
    }
    /// A child of `config`'s text.
    fn conf(&self, name: &str) -> &str {
        self.kid("config").map_or("", |c| c.get(name))
    }
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

fn attr(tag: &str, key: &str) -> String {
    let pat = format!("{key}=\"");
    tag.find(&pat)
        .and_then(|i| {
            let rest = &tag[i + pat.len()..];
            rest.find('"').map(|j| rest[..j].to_string())
        })
        .unwrap_or_default()
}

/// The tree of `text`; a root element named "" holding the top-level elements. Malformed text
/// gives what was read up to the fault (never a panic).
pub fn parse(text: &str) -> El {
    let mut stack = vec![El::default()];
    let mut rest = text;
    while let Some(lt) = rest.find('<') {
        let between = &rest[..lt];
        if let Some(top) = stack.last_mut()
            && !between.trim().is_empty()
        {
            top.text.push_str(&unescape(between));
        }
        let Some(gt) = rest[lt..].find('>') else { break };
        let tag = &rest[lt + 1..lt + gt];
        rest = &rest[lt + gt + 1..];
        if tag.starts_with('?') || tag.starts_with('!') {
            continue;
        }
        if let Some(name) = tag.strip_prefix('/') {
            // Close the innermost element of this name (and anything left open inside it).
            if let Some(pos) = stack.iter().rposition(|e| e.name == name.trim())
                && pos > 0
            {
                while stack.len() > pos {
                    let done = stack.pop().unwrap_or_default();
                    if let Some(parent) = stack.last_mut() {
                        parent.kids.push(done);
                    }
                }
            }
            continue;
        }
        let self_closing = tag.ends_with('/');
        let body = tag.trim_end_matches('/');
        let name = body.split_whitespace().next().unwrap_or("").to_string();
        let el = El { name, id: attr(body, "id"), r#ref: attr(body, "ref"), ..Default::default() };
        if self_closing {
            if let Some(top) = stack.last_mut() {
                top.kids.push(el);
            }
        } else {
            stack.push(el);
        }
    }
    while stack.len() > 1 {
        let done = stack.pop().unwrap_or_default();
        if let Some(parent) = stack.last_mut() {
            parent.kids.push(done);
        }
    }
    stack.pop().unwrap_or_default()
}

/// The disks in `xml`. `umass`: whether the USB mass-storage driver is attached (then `da`
/// disks are USB sticks and drives, not SAS).
pub fn disks(xml: &str, umass: bool) -> Vec<Disk> {
    let root = parse(xml);
    let Some(mesh) = root.kid("mesh") else { return Vec::new() };
    let class = |name: &str| mesh.kids("class").find(|c| c.get("name") == name);
    // Every provider's name by id, for consumers' refs.
    let mut provider_name: HashMap<&str, &str> = HashMap::new();
    for c in mesh.kids("class") {
        for g in c.kids("geom") {
            for p in g.kids("provider") {
                provider_name.insert(p.id.as_str(), p.get("name"));
            }
        }
    }
    // LABEL: other names for a provider ("nda0p1" is also "gpt/efiboot0"); a file system label
    // names the file system too ("msdosfs/STICK", "ufs/rootfs", "ext2fs/data").
    let mut labels: HashMap<String, Vec<String>> = HashMap::new();
    if let Some(c) = class("LABEL") {
        for g in c.kids("geom") {
            for p in g.kids("provider") {
                labels.entry(g.get("name").to_string()).or_default().push(p.get("name").to_string());
            }
        }
    }
    // SWAP: which providers are swapped on.
    let mut swap: Vec<&str> = Vec::new();
    if let Some(c) = class("SWAP") {
        for g in c.kids("geom") {
            for cons in g.kids("consumer") {
                if let Some(p) = cons.kid("provider").and_then(|p| provider_name.get(p.r#ref.as_str())) {
                    swap.push(p);
                }
            }
        }
    }
    // PART: each disk's table and partitions.
    let mut tables: HashMap<&str, &El> = HashMap::new();
    if let Some(c) = class("PART") {
        for g in c.kids("geom") {
            tables.insert(g.get("name"), g);
        }
    }
    let mut out = Vec::new();
    let Some(dclass) = class("DISK") else { return out };
    for g in dclass.kids("geom") {
        let Some(p) = g.kid("provider") else { continue };
        let name = p.get("name").to_string();
        let base: String = name.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
        let (bus, kind) = match base.as_str() {
            "nda" | "nvd" => ("NVMe", Kind::Internal),
            "ada" => ("SATA", Kind::Internal),
            "da" if umass => ("USB", Kind::External),
            "da" => ("SCSI", Kind::Internal),
            "mmcsd" | "sdda" => ("Memory card", Kind::External),
            "md" => ("Disk image", Kind::Image),
            "vtbd" => ("Virtual", Kind::Internal),
            "cd" => continue,
            _ => ("", Kind::Internal),
        };
        let rotation = p.conf("rotationrate");
        let mut d = Disk {
            model: p.conf("descr").replace('_', " "),
            serial: p.conf("ident").to_string(),
            size: p.get("mediasize").parse().unwrap_or(0),
            bus: bus.into(),
            kind: Some(kind),
            // 0 = not reported, 1 = solid state, else RPM.
            rotational: match rotation {
                "1" => Some(false),
                "0" | "" | "unknown" => (bus == "NVMe").then_some(false),
                _ => Some(true),
            },
            name: name.clone(),
            ..Default::default()
        };
        if let Some(t) = tables.get(name.as_str()) {
            d.scheme = t.conf("scheme").to_string();
            for pp in t.kids("provider") {
                let pname = pp.get("name").to_string();
                let sector: u64 = pp.get("sectorsize").parse().unwrap_or(512);
                let mut part = Part {
                    index: pp.conf("index").parse().unwrap_or(0),
                    start: pp.conf("start").parse::<u64>().unwrap_or(0) * sector,
                    size: pp.get("mediasize").parse().unwrap_or(0),
                    ptype: ptype_word(pp.conf("type")),
                    label: pp.conf("label").to_string(),
                    uuid: pp.conf("rawuuid").to_string(),
                    ..Default::default()
                };
                let alias = pp.get("alias");
                if !alias.is_empty() {
                    part.aliases.push(alias.to_string());
                }
                fill_from_labels(&mut part, labels.get(&pname));
                if swap.contains(&pname.as_str()) {
                    part.role = "Swap, in use".into();
                    part.fs = "Swap".into();
                }
                if part.fs.is_empty() {
                    part.fs = match pp.conf("type") {
                        "freebsd-swap" => "Swap".into(),
                        "freebsd-zfs" => "ZFS".into(),
                        "freebsd-ufs" => "UFS".into(),
                        "efi" => "FAT32".into(),
                        _ => String::new(),
                    };
                }
                part.name = pname;
                d.parts.push(part);
            }
            d.parts.sort_by_key(|p| p.start);
        } else {
            let mut whole = Part { name: name.clone(), size: d.size, ..Default::default() };
            fill_from_labels(&mut whole, labels.get(&name));
            if !whole.fs.is_empty() || !whole.aliases.is_empty() {
                d.whole = Some(whole);
            }
        }
        out.push(d);
    }
    out
}

/// LABEL's names for a provider: "gpt/efiboot0" (a GPT label: an alias), "msdosfs/STICK" (the
/// file system and its label), "gptid/..." and "diskid/..." (aliases).
fn fill_from_labels(part: &mut Part, names: Option<&Vec<String>>) {
    for n in names.into_iter().flatten() {
        part.aliases.push(n.clone());
        if let Some((kind, label)) = n.split_once('/') {
            let fs = match kind {
                "msdosfs" => "FAT32",
                "ufs" | "ufsid" => "UFS",
                "ext2fs" => "ext2/3/4",
                "ntfs" => "NTFS",
                "iso9660" => "ISO 9660 (disc image)",
                "exfat" => "exFAT",
                _ => continue,
            };
            part.fs = fs_word(fs);
            if kind != "ufsid" {
                part.label = label.to_string();
            }
        }
    }
}
