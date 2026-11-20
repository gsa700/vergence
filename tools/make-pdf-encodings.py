#!/usr/bin/env python3
"""Writes crates/vergence-pdf/src/font/encoding.rs: the simple fonts'
encodings (StandardEncoding, WinAnsiEncoding, MacRomanEncoding: code to
glyph name) and glyph names to Unicode, from the name table below and
Python's own cp1252 and mac_roman codecs, so 768 entries are not typed.

    python3 tools/make-pdf-encodings.py > crates/vergence-pdf/src/font/encoding.rs
"""
names = {}
ascii_names = "space exclam quotedbl numbersign dollar percent ampersand quotesingle parenleft parenright asterisk plus comma hyphen period slash zero one two three four five six seven eight nine colon semicolon less equal greater question at".split()
for i, n in enumerate(ascii_names):
    names[0x20 + i] = n
for c in range(26):
    names[0x41 + c] = chr(0x41 + c)
    names[0x61 + c] = chr(0x61 + c)
for u, n in [(0x5B, 'bracketleft'), (0x5C, 'backslash'), (0x5D, 'bracketright'), (0x5E, 'asciicircum'), (0x5F, 'underscore'), (0x60, 'grave'), (0x7B, 'braceleft'), (0x7C, 'bar'), (0x7D, 'braceright'), (0x7E, 'asciitilde')]:
    names[u] = n
latin1 = "space exclamdown cent sterling currency yen brokenbar section dieresis copyright ordfeminine guillemotleft logicalnot hyphen registered macron degree plusminus twosuperior threesuperior acute mu paragraph periodcentered cedilla onesuperior ordmasculine guillemotright onequarter onehalf threequarters questiondown Agrave Aacute Acircumflex Atilde Adieresis Aring AE Ccedilla Egrave Eacute Ecircumflex Edieresis Igrave Iacute Icircumflex Idieresis Eth Ntilde Ograve Oacute Ocircumflex Otilde Odieresis multiply Oslash Ugrave Uacute Ucircumflex Udieresis Yacute Thorn germandbls agrave aacute acircumflex atilde adieresis aring ae ccedilla egrave eacute ecircumflex edieresis igrave iacute icircumflex idieresis eth ntilde ograve oacute ocircumflex otilde odieresis divide oslash ugrave uacute ucircumflex udieresis yacute thorn ydieresis".split()
assert len(latin1) == 96, len(latin1)
for i, n in enumerate(latin1):
    names[0xA0 + i] = n
extra = {0x20AC: 'Euro', 0x201A: 'quotesinglbase', 0x0192: 'florin', 0x201E: 'quotedblbase', 0x2026: 'ellipsis', 0x2020: 'dagger', 0x2021: 'daggerdbl', 0x02C6: 'circumflex', 0x2030: 'perthousand', 0x0160: 'Scaron', 0x2039: 'guilsinglleft', 0x0152: 'OE', 0x017D: 'Zcaron', 0x2018: 'quoteleft', 0x2019: 'quoteright', 0x201C: 'quotedblleft', 0x201D: 'quotedblright', 0x2022: 'bullet', 0x2013: 'endash', 0x2014: 'emdash', 0x02DC: 'tilde', 0x2122: 'trademark', 0x0161: 'scaron', 0x203A: 'guilsinglright', 0x0153: 'oe', 0x017E: 'zcaron', 0x0178: 'Ydieresis',
         0x2260: 'notequal', 0x221E: 'infinity', 0x2264: 'lessequal', 0x2265: 'greaterequal', 0x2202: 'partialdiff', 0x2211: 'summation', 0x220F: 'product', 0x03C0: 'pi', 0x222B: 'integral', 0x2126: 'Omega', 0x03A9: 'Omega', 0x221A: 'radical', 0x2248: 'approxequal', 0x2206: 'Delta', 0x25CA: 'lozenge', 0x2044: 'fraction', 0xFB01: 'fi', 0xFB02: 'fl', 0xF8FF: 'apple', 0x0131: 'dotlessi', 0x02DA: 'ring', 0x02D8: 'breve', 0x02D9: 'dotaccent', 0x02DD: 'hungarumlaut', 0x02DB: 'ogonek', 0x02C7: 'caron', 0x00AF: 'macron', 0x0141: 'Lslash', 0x0142: 'lslash', 0x2212: 'minus'}
names.update(extra)


def table(codec):
    out = []
    for b in range(256):
        if b < 0x20:
            out.append('')
            continue
        try:
            u = ord(bytes([b]).decode(codec))
        except Exception:
            out.append('')
            continue
        out.append(names.get(u, ''))
    return out


win = table('cp1252')
# PDF's WinAnsi: undefined codes above 0x7E show bullets; 0xA0 is space, 0xAD hyphen.
for b in [0x7F, 0x81, 0x8D, 0x8F, 0x90, 0x9D]:
    win[b] = 'bullet'
win[0xA0] = 'space'
win[0xAD] = 'hyphen'
mac = table('mac_roman')
mac[0xDB] = 'currency'  # PDF's MacRoman predates the euro there
std = [''] * 256
for b in range(0x20, 0x7F):
    std[b] = names.get(b, '')
std[0x27] = 'quoteright'
std[0x60] = 'quoteleft'
for b, n in [(0xA1, 'exclamdown'), (0xA2, 'cent'), (0xA3, 'sterling'), (0xA4, 'fraction'), (0xA5, 'yen'), (0xA6, 'florin'), (0xA7, 'section'), (0xA8, 'currency'), (0xA9, 'quotesingle'), (0xAA, 'quotedblleft'), (0xAB, 'guillemotleft'), (0xAC, 'guilsinglleft'), (0xAD, 'guilsinglright'), (0xAE, 'fi'), (0xAF, 'fl'), (0xB1, 'endash'), (0xB2, 'dagger'), (0xB3, 'daggerdbl'), (0xB4, 'periodcentered'), (0xB6, 'paragraph'), (0xB7, 'bullet'), (0xB8, 'quotesinglbase'), (0xB9, 'quotedblbase'), (0xBA, 'quotedblright'), (0xBB, 'guillemotright'), (0xBC, 'ellipsis'), (0xBD, 'perthousand'), (0xBF, 'questiondown'), (0xC1, 'grave'), (0xC2, 'acute'), (0xC3, 'circumflex'), (0xC4, 'tilde'), (0xC5, 'macron'), (0xC6, 'breve'), (0xC7, 'dotaccent'), (0xC8, 'dieresis'), (0xCA, 'ring'), (0xCB, 'cedilla'), (0xCD, 'hungarumlaut'), (0xCE, 'ogonek'), (0xCF, 'caron'), (0xD0, 'emdash'), (0xE1, 'AE'), (0xE3, 'ordfeminine'), (0xE8, 'Lslash'), (0xE9, 'Oslash'), (0xEA, 'OE'), (0xEB, 'ordmasculine'), (0xF1, 'ae'), (0xF5, 'dotlessi'), (0xF8, 'lslash'), (0xF9, 'oslash'), (0xFA, 'oe'), (0xFB, 'germandbls')]:
    std[b] = n


def rs(name, t):
    body = ", ".join('"%s"' % x for x in t)
    return f"pub const {name}: [&str; 256] = [{body}];"


seen = {}
for u in sorted(names):
    n = names[u]
    if n and n not in seen:
        seen[n] = u
pairs = sorted(seen.items())
print("//! The simple fonts' encodings (ISO 32000-2 Annex D): code to glyph")
print("//! name for StandardEncoding, WinAnsiEncoding and MacRomanEncoding, and")
print("//! glyph names to Unicode. Generated by tools/make-pdf-encodings.py from")
print("//! a name table and Python's cp1252 and mac_roman codecs, not typed in.")
print("//! Names not here are read as uniXXXX, uXXXX[XX] or not at all.")
print()
print(rs("STANDARD", std))
print(rs("WIN_ANSI", win))
print(rs("MAC_ROMAN", mac))
print("/// Glyph names to Unicode, sorted by name.")
print("pub const NAMES: [(&str, u32); %d] = [%s];" % (len(pairs), ", ".join('("%s", 0x%04X)' % (n, u) for n, u in pairs)))
print('''
/// A glyph name's character: from the table, or uniXXXX, uXXXX, uXXXXX;
/// a suffix after a period is ignored ("a.sc").
pub fn unicode_of(name: &str) -> Option<char> {
    let base = name.split('.').next().unwrap_or(name);
    if let Ok(i) = NAMES.binary_search_by(|(n, _)| n.cmp(&base)) {
        return char::from_u32(NAMES[i].1);
    }
    let hex = base.strip_prefix("uni").filter(|h| h.len() >= 4).map(|h| &h[..4]).or_else(|| base.strip_prefix('u').filter(|h| (4..=6).contains(&h.len())));
    hex.and_then(|h| u32::from_str_radix(h, 16).ok()).and_then(char::from_u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tables_agree_where_they_should() {
        assert_eq!(STANDARD[0x41], "A");
        assert_eq!(STANDARD[0x27], "quoteright");
        assert_eq!(WIN_ANSI[0x92], "quoteright");
        assert_eq!(WIN_ANSI[0x80], "Euro");
        assert_eq!(WIN_ANSI[0xE9], "eacute");
        assert_eq!(MAC_ROMAN[0x8E], "eacute");
        assert_eq!(MAC_ROMAN[0xA5], "bullet");
        assert_eq!(unicode_of("quoteright"), Some('\\u{2019}'));
        assert_eq!(unicode_of("uni00E9"), Some('é'));
        assert_eq!(unicode_of("a.sc"), Some('a'));
        assert_eq!(unicode_of("nonsense"), None);
    }
}''')
