//! Spelling (editor milestone 1, step 7): our own checker, reading the list
//! Vergence ships (data/words/en_US.txt, made from SCOWL by
//! tools/make-words.py), shipped with Vergence rather than taken from the
//! system (see docs/editor-milestone-1.md).
//!
//! - **Checking** a word: as listed, or capitalized at a sentence's start
//!   ("The" for "the"), or all in capitals ("THE", "NASA"); a name stays a
//!   name ("paris" is not "Paris"). Curly apostrophes count as straight.
//! - **Suggestions**: listed words one or two edits away (a letter left
//!   out, one too many, one wrong, two swapped), the nearest first; among
//!   them, a slip to a neighboring key and the commoner word (the list's
//!   size levels) first; in the case the word was typed in.
//! - **Completions** for the text area's autocomplete: listed words
//!   starting with what is typed, commoner first.
//! - **The user's own words**, shared by every app:
//!   ~/.local/share/vergence/words.txt (owner only), one a line; read again
//!   when it changes.
//!
//! The list is read once per process, the first time it is wanted.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime};

/// One listed word, by its lower-case form: its size level, and its forms
/// when they are not just the lower-case one ("Paris", "NASA", "iPhone").
struct Entry {
    level: u8,
    forms: Option<Box<[Box<str>]>>,
}

pub struct Speller {
    words: HashMap<Box<str>, Entry>,
    /// The lower-case forms, sorted, for completions by prefix.
    sorted: Vec<Box<str>>,
    user: Mutex<User>,
}

struct User {
    path: Option<PathBuf>,
    words: HashSet<String>,
    stamp: Option<SystemTime>,
    looked: Option<Instant>,
}

impl User {
    /// Reads the user's words again if the file changed (looked at most once a second).
    fn fresh(&mut self) {
        if self.looked.is_some_and(|t| t.elapsed().as_secs_f32() < 1.0) {
            return;
        }
        self.looked = Some(Instant::now());
        let Some(p) = &self.path else { return };
        let stamp = std::fs::metadata(p).and_then(|m| m.modified()).ok();
        if stamp != self.stamp {
            self.stamp = stamp;
            self.words = std::fs::read_to_string(p)
                .unwrap_or_default()
                .lines()
                .map(str::trim)
                .filter(|w| !w.is_empty() && w.len() <= 64)
                .map(str::to_string)
                .collect();
        }
    }
}

static SPELLER: OnceLock<Option<Speller>> = OnceLock::new();

/// The speller for this process (None if no list was found).
pub fn speller() -> Option<&'static Speller> {
    SPELLER
        .get_or_init(|| {
            list_path().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| Speller::from_list(&t, user_path()))
        })
        .as_ref()
}

/// Where the list is: $VERGENCE_WORDS; beside an installed program
/// (bin/../share/vergence/words/en_US.txt); the checkout's data/words.
fn list_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("VERGENCE_WORDS").map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(p);
    }
    if let Some(p) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent()?.parent().map(|d| d.join("share/vergence/words/en_US.txt")))
        .filter(|p| p.is_file())
    {
        return Some(p);
    }
    let dev = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/words/en_US.txt");
    dev.is_file().then_some(dev)
}

fn user_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(base.join("vergence/words.txt"))
}

/// A word with curly apostrophes made straight.
fn plain(w: &str) -> String {
    w.replace(['\u{2019}', '\u{2018}'], "'")
}

fn is_lower(w: &str) -> bool {
    !w.chars().any(char::is_uppercase)
}

fn is_title(w: &str) -> bool {
    let mut c = w.chars();
    c.next().is_some_and(char::is_uppercase) && is_lower(c.as_str())
}

fn is_caps(w: &str) -> bool {
    w.chars().filter(|c| c.is_alphabetic()).count() > 1 && !w.chars().any(char::is_lowercase)
}

/// `w` written in the case `like` was: capitalized, all capitals, or as it is.
fn in_case_of(w: &str, like: &str) -> String {
    if is_caps(like) {
        w.to_uppercase()
    } else if is_title(like) && is_lower(w) {
        let mut c = w.chars();
        c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
    } else {
        w.to_string()
    }
}

impl Speller {
    /// A speller from a list's text (make-words.py's form) and where the user's words are.
    pub fn from_list(text: &str, user: Option<PathBuf>) -> Speller {
        let mut words: HashMap<Box<str>, Entry> = HashMap::with_capacity(130_000);
        let mut level = 60u8;
        for line in text.lines() {
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            if let Some(n) = line.strip_prefix("= ") {
                level = n.trim().parse().unwrap_or(60);
                continue;
            }
            let w = line.trim();
            let key: Box<str> = w.to_lowercase().into();
            let own = *key == *w;
            match words.get_mut(&key) {
                Some(e) => {
                    e.level = e.level.min(level);
                    if !own {
                        let mut f: Vec<Box<str>> = e.forms.take().map(Vec::from).unwrap_or_else(|| vec![key.clone()]);
                        f.push(w.into());
                        e.forms = Some(f.into());
                    } else if let Some(f) = &mut e.forms
                        && !f.iter().any(|x| **x == *w)
                    {
                        let mut v = Vec::from(std::mem::take(f));
                        v.push(w.into());
                        *f = v.into();
                    }
                }
                None => {
                    words.insert(key, Entry { level, forms: (!own).then(|| vec![Box::<str>::from(w)].into()) });
                }
            }
        }
        let mut sorted: Vec<Box<str>> = words.keys().cloned().collect();
        sorted.sort();
        Speller {
            words,
            sorted,
            user: Mutex::new(User { path: user, words: HashSet::new(), stamp: None, looked: None }),
        }
    }

    /// Whether a listed form allows `w`: itself, a lower-case word
    /// capitalized or in capitals, anything in capitals.
    fn allows(form: &str, w: &str) -> bool {
        form == w || (is_lower(form) && (is_title(w) || is_caps(w))) || is_caps(w) || (is_title(form) && is_caps(w))
    }

    /// Whether `word` is spelled right.
    pub fn check(&self, word: &str) -> bool {
        let w = plain(word);
        let key = w.to_lowercase();
        if let Some(e) = self.words.get(key.as_str()) {
            let ok = match &e.forms {
                None => Speller::allows(&key, &w),
                Some(f) => f.iter().any(|form| Speller::allows(form, &w)),
            };
            if ok {
                return true;
            }
        }
        // A possessive of a known word ("Vergence's" once Vergence is the user's).
        let mut u = self.user.lock().unwrap_or_else(|e| e.into_inner());
        u.fresh();
        // `allows` judges case only: the word itself must be the same first
        // (otherwise any capitalized word passed once "vergence" was the user's).
        u.words.iter().any(|form| {
            let f = form.to_lowercase();
            (f == key && Speller::allows(form, &w))
                || (format!("{f}'s") == key && Speller::allows(&format!("{form}'s"), &w))
        })
    }

    /// How common a word is: its size level (10 commonest ... 60), if listed.
    pub fn level(&self, word: &str) -> Option<u8> {
        self.words.get(plain(word).to_lowercase().as_str()).map(|e| e.level)
    }

    /// The listed form of a lower-case key that `like` would be written as.
    fn form_for(&self, key: &str, like: &str) -> Option<String> {
        let e = self.words.get(key)?;
        let form = match &e.forms {
            None => key.to_string(),
            // A lower-case form if there is one, else the first (a name).
            Some(f) => f.iter().find(|x| is_lower(x)).or(f.first()).map(|x| x.to_string())?,
        };
        Some(in_case_of(&form, like))
    }

    /// Up to `n` suggestions for a word spelled wrong, the likeliest first.
    pub fn suggest(&self, word: &str, n: usize) -> Vec<String> {
        let w = plain(word);
        let key = w.to_lowercase();
        let chars: Vec<char> = key.chars().collect();
        // (edits, slip score, level, the candidate)
        let mut found: HashMap<String, (u8, i32, u8)> = HashMap::new();
        let consider = |cand: &str, edits: u8, slip: i32, found: &mut HashMap<String, (u8, i32, u8)>| {
            if let Some(e) = self.words.get(cand) {
                let s = (edits, -slip, e.level);
                let k = found.entry(cand.to_string()).or_insert(s);
                if s < *k {
                    *k = s;
                }
            }
        };
        // Listed, only not in this case ("paris"): its own form first.
        if self.words.contains_key(key.as_str()) {
            found.insert(key.clone(), (0, 0, 0));
        }
        let first = edits(&chars);
        for (c, slip) in &first {
            consider(c, 1, *slip, &mut found);
        }
        if found.len() < n && chars.len() <= 14 {
            for (c, _) in &first {
                let cc: Vec<char> = c.chars().collect();
                for (c2, _) in edits(&cc) {
                    consider(&c2, 2, 0, &mut found);
                }
            }
        }
        let mut list: Vec<(String, (u8, i32, u8))> = found.into_iter().collect();
        list.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        list.into_iter().filter_map(|(k, _)| self.form_for(&k, &w)).take(n).collect()
    }

    /// Up to `n` listed words that start with `prefix` (without case),
    /// commoner first, in the case the prefix was typed in.
    pub fn complete(&self, prefix: &str, n: usize) -> Vec<String> {
        let p = plain(prefix).to_lowercase();
        if p.is_empty() {
            return Vec::new();
        }
        let start = self.sorted.partition_point(|w| **w < *p);
        let mut hits: Vec<(&str, u8)> = self.sorted[start..]
            .iter()
            .take_while(|w| w.starts_with(&p))
            .filter(|w| w.len() > p.len() && !w.ends_with("'s"))
            .map(|w| (&**w, self.words.get(&**w).map_or(60, |e| e.level)))
            .collect();
        hits.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.len().cmp(&b.0.len())).then_with(|| a.0.cmp(b.0)));
        hits.into_iter().filter_map(|(k, _)| self.form_for(k, prefix)).take(n).collect()
    }

    /// Adds `word` to the user's words (and the file, owner only).
    pub fn add(&self, word: &str) -> std::io::Result<()> {
        let w = plain(word).trim().to_string();
        if w.is_empty() || w.len() > 64 || w.contains(char::is_whitespace) {
            return Ok(());
        }
        let mut u = self.user.lock().unwrap_or_else(|e| e.into_inner());
        u.looked = None;
        u.fresh();
        if u.words.contains(&w) {
            return Ok(());
        }
        if let Some(p) = u.path.clone() {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            if let Some(d) = p.parent() {
                std::fs::create_dir_all(d)?;
            }
            let mut f = std::fs::OpenOptions::new().create(true).append(true).mode(0o600).open(&p)?;
            writeln!(f, "{w}")?;
            u.stamp = std::fs::metadata(&p).and_then(|m| m.modified()).ok();
        }
        u.words.insert(w);
        Ok(())
    }
}

/// QWERTY's neighbors of each letter (a slip to one is likelier).
fn neighbors(c: char) -> &'static str {
    match c {
        'q' => "wa",
        'w' => "qeas",
        'e' => "wrsd",
        'r' => "etdf",
        't' => "ryfg",
        'y' => "tugh",
        'u' => "yihj",
        'i' => "uojk",
        'o' => "ipkl",
        'p' => "ol",
        'a' => "qwsz",
        's' => "awedxz",
        'd' => "serfcx",
        'f' => "drtgvc",
        'g' => "ftyhbv",
        'h' => "gyujnb",
        'j' => "huikmn",
        'k' => "jiolm",
        'l' => "kop",
        'z' => "asx",
        'x' => "zsdc",
        'c' => "xdfv",
        'v' => "cfgb",
        'b' => "vghn",
        'n' => "bhjm",
        'm' => "njk",
        _ => "",
    }
}

/// Every string one edit from `w`, with how likely the slip is (3: two
/// letters swapped; 2: a neighboring key; 1: a doubled letter; 0: else).
fn edits(w: &[char]) -> Vec<(String, i32)> {
    const ALPHABET: &str = "abcdefghijklmnopqrstuvwxyz'";
    let s = |v: &[char]| v.iter().collect::<String>();
    let mut out = Vec::with_capacity(w.len() * 56 + 26);
    for i in 0..w.len() {
        // One too many (a doubled letter is a likely slip).
        let mut v = w.to_vec();
        let gone = v.remove(i);
        out.push((s(&v), if i > 0 && w[i - 1] == gone { 1 } else { 0 }));
        // Two swapped: the commonest slip of all.
        if i + 1 < w.len() {
            let mut v = w.to_vec();
            v.swap(i, i + 1);
            out.push((s(&v), 3));
        }
        // One wrong.
        for c in ALPHABET.chars() {
            if c != w[i] {
                let mut v = w.to_vec();
                v[i] = c;
                out.push((s(&v), if neighbors(w[i]).contains(c) { 2 } else { 0 }));
            }
        }
    }
    // One left out.
    for i in 0..=w.len() {
        for c in ALPHABET.chars() {
            let mut v = w.to_vec();
            v.insert(i, c);
            out.push((s(&v), if (i > 0 && w[i - 1] == c) || w.get(i) == Some(&c) { 1 } else { 0 }));
        }
    }
    out
}

/// The words of a line to check, as byte ranges: letters, with an
/// apostrophe between letters kept ("don't"). Passed over: words with
/// digits or underscores, parts of addresses and paths (next to `/`, `@`,
/// `.` with a letter after, `:` before `//`), CamelCase and mixed words
/// (code), and single letters.
pub fn words_to_check(line: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < line.len() {
        let Some(c) = line[i..].chars().next() else { break };
        if !c.is_alphanumeric() && c != '_' {
            i += c.len_utf8();
            continue;
        }
        // A run of letters, digits, _ and inner apostrophes.
        let start = i;
        let mut end = i;
        let mut j = i;
        while let Some(c) = line[j..].chars().next() {
            if c.is_alphanumeric() || c == '_' {
                j += c.len_utf8();
                end = j;
            } else if (c == '\'' || c == '\u{2019}')
                && line[j + c.len_utf8()..].chars().next().is_some_and(char::is_alphabetic)
            {
                j += c.len_utf8();
            } else {
                break;
            }
        }
        i = end.max(start + 1);
        let w = &line[start..end];
        let before = line[..start].chars().next_back();
        let after = line[end..].chars().next();
        let after2 = line[end..].chars().nth(1);
        let code = w.chars().any(|c| c.is_ascii_digit() || c == '_')
            || w.chars().count() < 2
            || matches!(before, Some('/' | '@' | '\\' | '.' | '#' | '$' | '%' | '&' | '~' | '<'))
            || matches!(after, Some('/' | '@' | '\\' | '(' | '>'))
            || (after == Some('.') && after2.is_some_and(char::is_alphanumeric))
            || (after == Some(':') && line[end..].starts_with("://"))
            || {
                // CamelCase or mixedCase: a capital after a small letter.
                let cs: Vec<char> = w.chars().collect();
                cs.windows(2).any(|p| p[0].is_lowercase() && p[1].is_uppercase())
            };
        if !code {
            out.push((start, end));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = "# test\n= 10\nthe\ncolor\nParis\nNASA\niPhone\ndon't\nhouse\nhorse\n= 35\nreceive\nseparate\nhouses\n= 50\nhose\n";

    fn sp(user: Option<PathBuf>) -> Speller {
        Speller::from_list(LIST, user)
    }

    #[test]
    fn words_are_checked_in_their_cases() {
        let s = sp(None);
        for ok in [
            "the",
            "The",
            "THE",
            "color",
            "Paris",
            "PARIS",
            "NASA",
            "iPhone",
            "IPHONE",
            "don't",
            "don\u{2019}t",
            "Don't",
        ] {
            assert!(s.check(ok), "{ok}");
        }
        for bad in ["teh", "colour", "paris", "Nasa", "iphone", "Iphone", "dont", "tHe"] {
            assert!(!s.check(bad), "{bad}");
        }
        assert_eq!(s.level("Receive"), Some(35));
    }

    #[test]
    fn suggestions_come_nearest_and_likeliest_first() {
        let s = sp(None);
        assert_eq!(s.suggest("teh", 3), vec!["the"], "two letters swapped");
        assert_eq!(s.suggest("recieve", 3), vec!["receive"]);
        assert_eq!(s.suggest("Seperate", 3), vec!["Separate"], "in the case it was typed");
        assert_eq!(s.suggest("dont", 3), vec!["don't"]);
        assert_eq!(s.suggest("paris", 3), vec!["Paris"], "a name keeps its capital");
        // "hoise": horse and house are one edit, hose two; o->u is a neighbor slip? i->r and i->u: u is i's neighbor.
        assert_eq!(s.suggest("hoise", 3)[0], "house", "i for u is a neighboring key");
        assert!(s.suggest("zzzzzzzz", 3).is_empty());
    }

    #[test]
    fn completions_are_commoner_first() {
        let s = sp(None);
        assert_eq!(s.complete("ho", 5), vec!["horse", "house", "houses", "hose"]);
        assert_eq!(s.complete("Ho", 1), vec!["Horse"]);
        assert_eq!(s.complete("Par", 2), vec!["Paris"]);
    }

    /// A user's lower-case word does not let other words through (it did: "Teh" passed).
    #[test]
    fn the_users_words_let_only_themselves_through() {
        let dir = std::env::temp_dir().join(format!("vergence-spell-own-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("words.txt");
        std::fs::write(&path, "vergence\nVergence\n").unwrap();
        let s = sp(Some(path));
        assert!(s.check("vergence") && s.check("Vergence") && s.check("VERGENCE") && s.check("Vergence's"));
        assert!(!s.check("Teh"), "a capitalized word not listed");
        assert!(!s.check("Xyzzy"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_users_words_are_kept_and_shared() {
        let dir = std::env::temp_dir().join(format!("vergence-spell-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("words.txt");
        let s = sp(Some(path.clone()));
        assert!(!s.check("Vergence"));
        s.add("Vergence").unwrap();
        assert!(s.check("Vergence") && s.check("VERGENCE") && s.check("Vergence's"));
        assert!(!s.check("vergence"), "added with its capital");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        // Another process's speller reads the file.
        let other = sp(Some(path));
        assert!(other.check("Vergence"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_words_of_a_line_leave_code_and_addresses_alone() {
        let line = "Teh quick fox's den, see https://example.com/path or me@mail.org; fooBar x2 snake_case I a don't.";
        let words: Vec<&str> = words_to_check(line).into_iter().map(|(a, b)| &line[a..b]).collect();
        assert_eq!(words, vec!["Teh", "quick", "fox's", "den", "see", "or", "don't"]);
    }

    #[test]
    fn the_shipped_list_reads() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/words/en_US.txt");
        let text = std::fs::read_to_string(path).expect("data/words/en_US.txt");
        let t = Instant::now();
        let s = Speller::from_list(&text, None);
        let read = t.elapsed();
        assert!(s.words.len() > 100_000);
        for ok in ["color", "organize", "receive", "The", "don't", "Chicago", "dog's"] {
            assert!(s.check(ok), "{ok}");
        }
        for bad in ["colour", "recieve", "teh"] {
            assert!(!s.check(bad), "{bad}");
        }
        assert_eq!(s.suggest("recieve", 1), vec!["receive"]);
        assert!(read.as_secs_f32() < 3.0, "read in {read:?}");
        let t = Instant::now();
        let _ = s.suggest("acommodation", 5);
        assert!(t.elapsed().as_secs_f32() < 3.0, "suggested in {:?}", t.elapsed());
    }
}
