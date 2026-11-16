//! A password as it is typed and sent: kept in one page of memory of its
//! own (never swapped, never in a core file: [`crate::sys::LockedPage`]),
//! wiped when dropped or cleared, with no Debug and no Display so it cannot
//! be formatted into a log by mistake (threat model T16, review I1).

use crate::sys::LockedPage;

/// The longest secret kept, in bytes; more is refused.
pub const MAX: usize = 1024;

pub struct Secret {
    page: LockedPage,
    len: usize,
}

impl Secret {
    /// An empty secret; None only if the system will not map a page.
    pub fn new() -> Option<Secret> {
        Some(Secret { page: LockedPage::new()?, len: 0 })
    }

    /// Add text typed; false if it would not fit (nothing is added then).
    pub fn push_str(&mut self, s: &str) -> bool {
        self.push(s.as_bytes())
    }

    pub fn push(&mut self, bytes: &[u8]) -> bool {
        if self.len + bytes.len() > MAX {
            return false;
        }
        self.page.bytes_mut()[self.len..self.len + bytes.len()].copy_from_slice(bytes);
        self.len += bytes.len();
        true
    }

    /// Remove the last character typed (Backspace), whole if it is more than one byte.
    pub fn pop_char(&mut self) {
        let bytes = &self.page.bytes()[..self.len];
        let mut cut = self.len.saturating_sub(1);
        while cut > 0 && (bytes[cut] & 0xC0) == 0x80 {
            cut -= 1;
        }
        for b in &mut self.page.bytes_mut()[cut..self.len] {
            *b = 0;
        }
        self.len = cut;
    }

    /// How many characters, to draw as many dots.
    pub fn chars(&self) -> usize {
        self.as_bytes().iter().filter(|b| (**b & 0xC0) != 0x80).count()
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.page.bytes()[..self.len]
    }

    /// Room to read into directly (the protocol's Answer), then [`Secret::set_len`].
    pub fn room(&mut self) -> &mut [u8] {
        &mut self.page.bytes_mut()[..MAX]
    }

    pub fn set_len(&mut self, len: usize) {
        self.len = len.min(MAX);
    }

    pub fn clear(&mut self) {
        self.page.wipe();
        self.len = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_and_backspace() {
        let mut s = Secret::new().unwrap();
        assert!(s.push_str("pä"));
        assert_eq!((s.len(), s.chars()), (3, 2));
        s.pop_char();
        assert_eq!(s.as_bytes(), b"p", "a character of two bytes goes whole");
        s.pop_char();
        s.pop_char();
        assert!(s.is_empty());
        assert!(!s.push(&[b'x'; MAX + 1]), "too long is refused whole");
        assert!(s.is_empty());
        assert!(s.push(&[b'x'; MAX]));
        s.clear();
        assert!(s.is_empty());
    }
}
