//! PDF's standard security handler (ISO 32000-2 §7.6): revisions 2 to 4
//! (RC4 and AES-128, keys from MD5) and 5 to 6 (AES-256, keys from SHA-2),
//! with crypt filters (V4, V5). Our own MD5, SHA-2, RC4 and AES (hash.rs,
//! aes.rs).
//!
//! Most encrypted PDFs have an empty user password: they only restrict
//! printing or copying, and open without asking. [`Security::unlock`] tries
//! a password as the user's and then as the owner's.

pub mod aes;
pub mod hash;

use crate::object::{Dict, Object};

/// The 32 bytes every password is padded with (revisions 2 to 4).
const PAD: [u8; 32] = [0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01, 0x08, 0x2E, 0x2E, 0x00, 0xB6, 0xD0, 0x68, 0x3E, 0x80, 0x2F, 0x0C, 0xA9, 0xFE, 0x64, 0x53, 0x69, 0x7A];

/// How a kind of data (strings, streams) is encrypted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    None,
    Rc4,
    Aes128,
    Aes256,
}

/// The /Encrypt dictionary as read, before any password.
#[derive(Debug, Clone)]
pub struct Security {
    revision: i64,
    /// The file key's length in bytes (revisions 2 to 4).
    length: usize,
    o: Vec<u8>,
    u: Vec<u8>,
    oe: Vec<u8>,
    ue: Vec<u8>,
    p: i32,
    id0: Vec<u8>,
    encrypt_metadata: bool,
    pub strings: Method,
    pub streams: Method,
}

/// An unlocked file: its key and how to apply it.
#[derive(Debug, Clone)]
pub struct Keys {
    key: Vec<u8>,
    pub strings: Method,
    pub streams: Method,
    pub encrypt_metadata: bool,
    /// Whether the owner's password opened it (all permissions).
    pub owner: bool,
}

impl Security {
    /// From the trailer's /Encrypt (resolved) and the first /ID string.
    /// None for a handler other than /Standard or a revision not known.
    pub fn new(enc: &Dict, id0: &[u8]) -> Option<Security> {
        if enc.name("Filter") != Some(b"Standard") {
            return None;
        }
        let v = enc.int("V").unwrap_or(0);
        let revision = enc.int("R")?;
        if !(2..=6).contains(&revision) {
            return None;
        }
        let bytes = |k: &str| enc.get(k).and_then(Object::as_bytes).map(<[u8]>::to_vec).unwrap_or_default();
        let length = match revision {
            2 => 5,
            _ => (enc.int("Length").unwrap_or(40).clamp(40, 128) / 8) as usize,
        };
        let (strings, streams) = match v {
            1 | 2 => (Method::Rc4, Method::Rc4),
            4 | 5 => {
                let cf = enc.get("CF").and_then(Object::as_dict);
                let method = |name: Option<&[u8]>| -> Method {
                    let Some(name) = name else { return Method::None };
                    if name == b"Identity" {
                        return Method::None;
                    }
                    let filter = cf.and_then(|d| d.get(std::str::from_utf8(name).unwrap_or(""))).and_then(Object::as_dict);
                    match filter.and_then(|f| f.name("CFM")) {
                        Some(b"V2") => Method::Rc4,
                        Some(b"AESV2") => Method::Aes128,
                        Some(b"AESV3") => Method::Aes256,
                        Some(b"None") => Method::None,
                        // A named filter with no method: the handler's default for its version.
                        _ if v == 5 => Method::Aes256,
                        _ => Method::Rc4,
                    }
                };
                (method(enc.name("StrF")), method(enc.name("StmF")))
            }
            _ => return None,
        };
        let encrypt_metadata = !matches!(enc.get("EncryptMetadata"), Some(Object::Bool(false)));
        Some(Security {
            revision,
            length,
            o: bytes("O"),
            u: bytes("U"),
            oe: bytes("OE"),
            ue: bytes("UE"),
            p: enc.int("P").unwrap_or(0) as i32,
            id0: id0.to_vec(),
            encrypt_metadata,
            strings,
            streams,
        })
    }

    /// The file's keys if `password` is its user or owner password.
    pub fn unlock(&self, password: &[u8]) -> Option<Keys> {
        let keys = |key: Vec<u8>, owner: bool| Keys { key, strings: self.strings, streams: self.streams, encrypt_metadata: self.encrypt_metadata, owner };
        if self.revision >= 5 {
            // UTF-8, at most 127 bytes (SASLprep left out: plain passwords are unchanged by it).
            let pw = &password[..password.len().min(127)];
            if let Some(k) = self.aes256_key(pw, false) {
                return Some(keys(k, false));
            }
            return self.aes256_key(pw, true).map(|k| keys(k, true));
        }
        if let Some(k) = self.user_key(password) {
            return Some(keys(k, false));
        }
        // As the owner's: it unlocks the user password stored in /O.
        let user = self.user_password_from_owner(password);
        self.user_key(&user).map(|k| keys(k, true))
    }

    /// Algorithm 2: the key for a user password, checked against /U.
    fn user_key(&self, password: &[u8]) -> Option<Vec<u8>> {
        let mut input = padded(password).to_vec();
        input.extend_from_slice(self.o.get(..32).unwrap_or(&self.o[..]));
        input.extend_from_slice(&self.p.to_le_bytes());
        input.extend_from_slice(&self.id0);
        if self.revision >= 4 && !self.encrypt_metadata {
            input.extend_from_slice(&[0xff; 4]);
        }
        let mut key = hash::md5(&input).to_vec();
        let n = self.length.min(16);
        if self.revision >= 3 {
            for _ in 0..50 {
                key = hash::md5(&key[..n]).to_vec();
            }
        }
        key.truncate(n);
        // Algorithm 4 and 5: what /U should hold with this key.
        let expect: Vec<u8> = if self.revision == 2 {
            rc4(&key, &PAD)
        } else {
            let mut m = PAD.to_vec();
            m.extend_from_slice(&self.id0);
            let mut x = rc4(&key, &hash::md5(&m));
            for i in 1..=19u8 {
                let k: Vec<u8> = key.iter().map(|b| b ^ i).collect();
                x = rc4(&k, &x);
            }
            x
        };
        let check = if self.revision == 2 { 32 } else { 16 };
        (self.u.len() >= check && expect.len() >= check && self.u[..check] == expect[..check]).then_some(key)
    }

    /// Algorithm 7's first half: the user password /O holds for an owner password.
    fn user_password_from_owner(&self, owner: &[u8]) -> Vec<u8> {
        let mut key = hash::md5(&padded(owner)).to_vec();
        let n = self.length.min(16);
        if self.revision >= 3 {
            for _ in 0..50 {
                key = hash::md5(&key[..n]).to_vec();
            }
        }
        key.truncate(n);
        let o = self.o.get(..32).unwrap_or(&self.o[..]);
        if self.revision == 2 {
            return rc4(&key, o);
        }
        let mut x = o.to_vec();
        for i in (0..=19u8).rev() {
            let k: Vec<u8> = key.iter().map(|b| b ^ i).collect();
            x = rc4(&k, &x);
        }
        x
    }

    /// Revisions 5 and 6: the file key from /UE (or /OE for the owner).
    fn aes256_key(&self, pw: &[u8], owner: bool) -> Option<Vec<u8>> {
        let (o, u) = (self.o.get(..48)?, self.u.get(..48)?);
        let (hash_part, salts, extra, wrapped) = if owner { (&o[..32], &o[32..48], u, &self.oe) } else { (&u[..32], &u[32..48], &[][..], &self.ue) };
        let (validation, key_salt) = (&salts[..8], &salts[8..16]);
        if self.hash_r56(pw, validation, extra) != hash_part {
            return None;
        }
        let k = self.hash_r56(pw, key_salt, extra);
        let aes = aes::Aes::new(&k)?;
        let key = aes.cbc_decrypt(&[0; 16], wrapped.get(..32)?, false);
        (key.len() == 32).then_some(key)
    }

    fn hash_r56(&self, pw: &[u8], salt: &[u8], extra: &[u8]) -> Vec<u8> {
        let mut input = pw.to_vec();
        input.extend_from_slice(salt);
        input.extend_from_slice(extra);
        let k = hash::sha256(&input).to_vec();
        if self.revision == 5 {
            return k;
        }
        hash_2b(pw, k, extra)
    }

    pub fn revision(&self) -> i64 {
        self.revision
    }
}

/// Algorithm 2.B (revision 6): rounds of AES-128-CBC and SHA-2 chosen by
/// the data, at least 64, until the last byte says stop.
fn hash_2b(pw: &[u8], mut k: Vec<u8>, extra: &[u8]) -> Vec<u8> {
    let mut round = 0u32;
    loop {
        let mut k1 = Vec::with_capacity(64 * (pw.len() + k.len() + extra.len()));
        for _ in 0..64 {
            k1.extend_from_slice(pw);
            k1.extend_from_slice(&k);
            k1.extend_from_slice(extra);
        }
        let aes = aes::Aes::new(&k[..16]).expect("16 bytes");
        let iv: [u8; 16] = k[16..32].try_into().expect("16 bytes");
        let e = aes.cbc_encrypt(&iv, &k1);
        let sum: u32 = e[..16].iter().map(|b| *b as u32).sum();
        k = match sum % 3 {
            0 => hash::sha256(&e).to_vec(),
            1 => hash::sha384(&e).to_vec(),
            _ => hash::sha512(&e).to_vec(),
        };
        round += 1;
        let last = *e.last().expect("not empty") as u32;
        // Stop once at least 64 rounds are done and the last byte is at most round - 32.
        if round >= 64 && last + 32 <= round {
            break;
        }
        // A bound no real file reaches; a hostile one cannot make it spin.
        if round > 4096 {
            break;
        }
    }
    k.truncate(32);
    k
}

fn padded(password: &[u8]) -> [u8; 32] {
    let mut out = PAD;
    let n = password.len().min(32);
    out[..n].copy_from_slice(&password[..n]);
    out[n..].copy_from_slice(&PAD[..32 - n]);
    out
}

pub fn rc4(key: &[u8], data: &[u8]) -> Vec<u8> {
    if key.is_empty() {
        return data.to_vec();
    }
    let mut s: [u8; 256] = std::array::from_fn(|i| i as u8);
    let mut j = 0u8;
    for i in 0..256 {
        j = j.wrapping_add(s[i]).wrapping_add(key[i % key.len()]);
        s.swap(i, j as usize);
    }
    let (mut i, mut j) = (0u8, 0u8);
    data.iter()
        .map(|b| {
            i = i.wrapping_add(1);
            j = j.wrapping_add(s[i as usize]);
            s.swap(i as usize, j as usize);
            b ^ s[s[i as usize].wrapping_add(s[j as usize]) as usize]
        })
        .collect()
}

impl Keys {
    /// Decrypts one string or stream of object `num` `generation`.
    pub fn decrypt(&self, method: Method, num: u32, generation: u16, data: &[u8]) -> Vec<u8> {
        match method {
            Method::None => data.to_vec(),
            Method::Aes256 => aes_cbc(&self.key, data),
            Method::Rc4 | Method::Aes128 => {
                // Algorithm 1: the object's own key.
                let mut input = self.key.clone();
                input.extend_from_slice(&num.to_le_bytes()[..3]);
                input.extend_from_slice(&generation.to_le_bytes());
                if method == Method::Aes128 {
                    input.extend_from_slice(b"sAlT");
                }
                let k = hash::md5(&input);
                let k = &k[..(self.key.len() + 5).min(16)];
                if method == Method::Rc4 { rc4(k, data) } else { aes_cbc(k, data) }
            }
        }
    }
}

/// AES-CBC with the IV in the first 16 bytes and PKCS #7 padding.
fn aes_cbc(key: &[u8], data: &[u8]) -> Vec<u8> {
    let (Some(aes), Some(iv)) = (aes::Aes::new(key), data.get(..16)) else { return Vec::new() };
    let iv: [u8; 16] = iv.try_into().expect("16 bytes");
    aes.cbc_decrypt(&iv, &data[16..], true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rc4_published_vector() {
        assert_eq!(rc4(b"Key", b"Plaintext"), vec![0xbb, 0xf3, 0x16, 0xe8, 0xd9, 0x40, 0xaf, 0x0a, 0xd3]);
    }

    /// An /Encrypt dictionary made the way a writer would (revision 3,
    /// RC4 128-bit, an empty user password and an owner password), then
    /// unlocked both ways.
    #[test]
    fn revision_3_empty_user_password_and_owner_password() {
        let id0 = b"0123456789abcdef".to_vec();
        let (owner_pw, p) = (b"ownerpw".to_vec(), -44i32);
        // Algorithm 3: /O from the owner and user passwords.
        let mut okey = hash::md5(&padded(&owner_pw)).to_vec();
        for _ in 0..50 {
            okey = hash::md5(&okey[..16]).to_vec();
        }
        let mut o = rc4(&okey[..16], &padded(b""));
        for i in 1..=19u8 {
            let k: Vec<u8> = okey[..16].iter().map(|b| b ^ i).collect();
            o = rc4(&k, &o);
        }
        let enc = |u: Vec<u8>| {
            Dict(vec![
                (b"Filter".to_vec(), Object::Name(b"Standard".to_vec())),
                (b"V".to_vec(), Object::Int(2)),
                (b"R".to_vec(), Object::Int(3)),
                (b"Length".to_vec(), Object::Int(128)),
                (b"O".to_vec(), Object::String(o.clone())),
                (b"U".to_vec(), Object::String(u)),
                (b"P".to_vec(), Object::Int(p as i64)),
            ])
        };
        // /U by algorithm 5, using the user key found with a placeholder /U.
        let probe = Security::new(&enc(vec![0; 32]), &id0).unwrap();
        let mut input = padded(b"").to_vec();
        input.extend_from_slice(&o);
        input.extend_from_slice(&p.to_le_bytes());
        input.extend_from_slice(&id0);
        let mut key = hash::md5(&input).to_vec();
        for _ in 0..50 {
            key = hash::md5(&key[..16]).to_vec();
        }
        key.truncate(16);
        let mut m = PAD.to_vec();
        m.extend_from_slice(&id0);
        let mut u = rc4(&key, &hash::md5(&m));
        for i in 1..=19u8 {
            let k: Vec<u8> = key.iter().map(|b| b ^ i).collect();
            u = rc4(&k, &u);
        }
        u.extend_from_slice(&[0; 16]);
        assert!(probe.unlock(b"").is_none(), "the placeholder /U does not match");
        let sec = Security::new(&enc(u), &id0).unwrap();
        let k = sec.unlock(b"").expect("the empty user password");
        assert!(!k.owner);
        let k2 = sec.unlock(b"ownerpw").expect("the owner password");
        assert!(k2.owner);
        assert!(sec.unlock(b"wrong").is_none());
        // Strings round trip through the object key.
        let secret = k.decrypt(Method::Rc4, 7, 0, b"Hello");
        assert_eq!(k.decrypt(Method::Rc4, 7, 0, &secret), b"Hello", "RC4 is its own inverse");
    }

    /// Revision 6: an /Encrypt made with algorithms 8 and 9, unlocked with
    /// the user password and refused with a wrong one.
    #[test]
    fn revision_6_aes_256() {
        let file_key = [0x5au8; 32];
        let user_pw = b"vergence";
        let (uvs, uks) = (*b"uvsalt01", *b"ukeyslt1");
        let sec0 = Security { revision: 6, length: 32, o: vec![], u: vec![], oe: vec![], ue: vec![], p: -4, id0: vec![], encrypt_metadata: true, strings: Method::Aes256, streams: Method::Aes256 };
        let mut u = sec0.hash_r56(user_pw, &uvs, &[]);
        u.extend_from_slice(&uvs);
        u.extend_from_slice(&uks);
        let kek = sec0.hash_r56(user_pw, &uks, &[]);
        let ue = aes::Aes::new(&kek).unwrap().cbc_encrypt(&[0; 16], &file_key);
        let sec = Security { u: u.clone(), ue, o: vec![0; 48], ..sec0 };
        let k = sec.unlock(user_pw).expect("unlocked");
        assert_eq!(k.key, file_key);
        assert!(sec.unlock(b"nope").is_none());
    }
}
