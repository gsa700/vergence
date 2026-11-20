//! AES (FIPS 197), our own, with 128- and 256-bit keys, and CBC: PDF
//! decrypts with AES-128 (V4) and AES-256 (V5), and revision 6's password
//! hash runs AES-128-CBC forward. The S-box is computed from its definition
//! (the inverse in GF(2^8), then the affine map), not typed in.
//!
//! Not constant-time: it decrypts documents the user opened, on their own
//! machine; nothing secret to a third party passes through it.

pub struct Aes {
    round_keys: Vec<[u8; 16]>,
}

struct Tables {
    sbox: [u8; 256],
    inv: [u8; 256],
}

fn xtime(x: u8) -> u8 {
    (x << 1) ^ if x & 0x80 != 0 { 0x1b } else { 0 }
}

fn mul(mut a: u8, mut b: u8) -> u8 {
    let mut p = 0;
    while b != 0 {
        if b & 1 != 0 {
            p ^= a;
        }
        a = xtime(a);
        b >>= 1;
    }
    p
}

fn tables() -> &'static Tables {
    static T: std::sync::OnceLock<Tables> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut sbox = [0u8; 256];
        let mut inv = [0u8; 256];
        for x in 0..=255u8 {
            // The multiplicative inverse (0 for 0): x^254.
            let mut r = 1u8;
            for _ in 0..254 {
                r = mul(r, x);
            }
            let b = if x == 0 { 0 } else { r };
            let s = b ^ b.rotate_left(1) ^ b.rotate_left(2) ^ b.rotate_left(3) ^ b.rotate_left(4) ^ 0x63;
            sbox[x as usize] = s;
            inv[s as usize] = x;
        }
        Tables { sbox, inv }
    })
}

impl Aes {
    /// For a 16- or 32-byte key; None for any other length.
    pub fn new(key: &[u8]) -> Option<Aes> {
        let nk = match key.len() {
            16 => 4,
            32 => 8,
            _ => return None,
        };
        let rounds = nk + 6;
        let t = tables();
        let mut w: Vec<[u8; 4]> = key.as_chunks::<4>().0.to_vec();
        let mut rcon = 1u8;
        for i in nk..4 * (rounds + 1) {
            let mut temp = w[i - 1];
            if i % nk == 0 {
                temp = [t.sbox[temp[1] as usize] ^ rcon, t.sbox[temp[2] as usize], t.sbox[temp[3] as usize], t.sbox[temp[0] as usize]];
                rcon = xtime(rcon);
            } else if nk > 6 && i % nk == 4 {
                temp = temp.map(|b| t.sbox[b as usize]);
            }
            let prev = w[i - nk];
            w.push([prev[0] ^ temp[0], prev[1] ^ temp[1], prev[2] ^ temp[2], prev[3] ^ temp[3]]);
        }
        let round_keys = w
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| {
                let mut k = [0u8; 16];
                for (j, word) in c.iter().enumerate() {
                    k[j * 4..j * 4 + 4].copy_from_slice(word);
                }
                k
            })
            .collect();
        Some(Aes { round_keys })
    }

    pub fn encrypt_block(&self, block: &mut [u8; 16]) {
        let t = tables();
        let last = self.round_keys.len() - 1;
        xor(block, &self.round_keys[0]);
        for r in 1..=last {
            for b in block.iter_mut() {
                *b = t.sbox[*b as usize];
            }
            shift_rows(block);
            if r != last {
                mix_columns(block);
            }
            xor(block, &self.round_keys[r]);
        }
    }

    pub fn decrypt_block(&self, block: &mut [u8; 16]) {
        let t = tables();
        let last = self.round_keys.len() - 1;
        xor(block, &self.round_keys[last]);
        for r in (0..last).rev() {
            inv_shift_rows(block);
            for b in block.iter_mut() {
                *b = t.inv[*b as usize];
            }
            xor(block, &self.round_keys[r]);
            if r != 0 {
                inv_mix_columns(block);
            }
        }
    }

    /// CBC decryption of whole blocks after `iv`; a trailing partial block
    /// is dropped. `unpad`: take off PKCS #7 padding when it is well formed.
    pub fn cbc_decrypt(&self, iv: &[u8; 16], data: &[u8], unpad: bool) -> Vec<u8> {
        let mut out = Vec::with_capacity(data.len());
        let mut prev = *iv;
        for chunk in data.as_chunks::<16>().0 {
            let mut block = *chunk;
            let cipher = block;
            self.decrypt_block(&mut block);
            xor(&mut block, &prev);
            out.extend_from_slice(&block);
            prev = cipher;
        }
        if unpad
            && let Some(&n) = out.last()
            && (1..=16).contains(&n)
            && out.len() >= n as usize
            && out[out.len() - n as usize..].iter().all(|b| *b == n)
        {
            out.truncate(out.len() - n as usize);
        }
        out
    }

    /// CBC encryption of whole blocks (no padding: the length must be a
    /// multiple of 16, as revision 6's hash makes it).
    pub fn cbc_encrypt(&self, iv: &[u8; 16], data: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(data.len());
        let mut prev = *iv;
        for chunk in data.as_chunks::<16>().0 {
            let mut block = *chunk;
            xor(&mut block, &prev);
            self.encrypt_block(&mut block);
            out.extend_from_slice(&block);
            prev = block;
        }
        out
    }
}

fn xor(a: &mut [u8; 16], b: &[u8; 16]) {
    for (x, y) in a.iter_mut().zip(b) {
        *x ^= y;
    }
}

// The state is column-major: byte r + 4c is row r, column c.
fn shift_rows(s: &mut [u8; 16]) {
    let o = *s;
    for r in 1..4 {
        for c in 0..4 {
            s[r + 4 * c] = o[r + 4 * ((c + r) % 4)];
        }
    }
}

fn inv_shift_rows(s: &mut [u8; 16]) {
    let o = *s;
    for r in 1..4 {
        for c in 0..4 {
            s[r + 4 * ((c + r) % 4)] = o[r + 4 * c];
        }
    }
}

fn mix_columns(s: &mut [u8; 16]) {
    for c in 0..4 {
        let a = [s[4 * c], s[4 * c + 1], s[4 * c + 2], s[4 * c + 3]];
        s[4 * c] = mul(a[0], 2) ^ mul(a[1], 3) ^ a[2] ^ a[3];
        s[4 * c + 1] = a[0] ^ mul(a[1], 2) ^ mul(a[2], 3) ^ a[3];
        s[4 * c + 2] = a[0] ^ a[1] ^ mul(a[2], 2) ^ mul(a[3], 3);
        s[4 * c + 3] = mul(a[0], 3) ^ a[1] ^ a[2] ^ mul(a[3], 2);
    }
}

fn inv_mix_columns(s: &mut [u8; 16]) {
    for c in 0..4 {
        let a = [s[4 * c], s[4 * c + 1], s[4 * c + 2], s[4 * c + 3]];
        s[4 * c] = mul(a[0], 14) ^ mul(a[1], 11) ^ mul(a[2], 13) ^ mul(a[3], 9);
        s[4 * c + 1] = mul(a[0], 9) ^ mul(a[1], 14) ^ mul(a[2], 11) ^ mul(a[3], 13);
        s[4 * c + 2] = mul(a[0], 13) ^ mul(a[1], 9) ^ mul(a[2], 14) ^ mul(a[3], 11);
        s[4 * c + 3] = mul(a[0], 11) ^ mul(a[1], 13) ^ mul(a[2], 9) ^ mul(a[3], 14);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn fips_197_vectors_both_ways() {
        let pt: [u8; 16] = h("00112233445566778899aabbccddeeff").try_into().unwrap();
        for (key, ct) in [("000102030405060708090a0b0c0d0e0f", "69c4e0d86a7b0430d8cdb78070b4c55a"), ("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f", "8ea2b7ca516745bfeafc49904b496089")] {
            let aes = Aes::new(&h(key)).unwrap();
            let mut b = pt;
            aes.encrypt_block(&mut b);
            assert_eq!(b.to_vec(), h(ct), "encrypt with {key}");
            aes.decrypt_block(&mut b);
            assert_eq!(b, pt, "decrypt with {key}");
        }
        assert!(Aes::new(&[0; 24]).is_none());
    }

    #[test]
    fn cbc_round_trip_and_padding() {
        let aes = Aes::new(&[7u8; 16]).unwrap();
        let iv = [3u8; 16];
        let mut msg = b"Vergence reads PDFs".to_vec();
        let pad = 16 - msg.len() % 16;
        msg.extend(std::iter::repeat_n(pad as u8, pad));
        let ct = aes.cbc_encrypt(&iv, &msg);
        assert_eq!(aes.cbc_decrypt(&iv, &ct, true), b"Vergence reads PDFs");
        assert_eq!(aes.cbc_decrypt(&iv, &ct, false), msg);
    }
}
