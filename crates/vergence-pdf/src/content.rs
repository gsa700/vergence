//! A content stream into operations (ISO 32000-2 §7.8.2): operands, then
//! an operator. Inline images (`BI` ... `ID` data `EI`) come out as one
//! operation, "BI", with their dictionary and raw data.
//!
//! Untrusted: the number of operations and of operands per operation is
//! bounded; stray bytes are skipped, as readers do.

use crate::lexer::{Lexer, is_white};
use crate::object::{Dict, Object};

/// The most operations taken from one stream, and operands kept per one.
pub const MAX_OPS: usize = 4_000_000;
const MAX_ARGS: usize = 64;

#[derive(Debug, Clone, PartialEq)]
pub struct Op {
    pub name: Vec<u8>,
    pub args: Vec<Object>,
}

pub fn parse(data: &[u8]) -> Vec<Op> {
    let mut out = Vec::new();
    let mut lx = Lexer::new(data, 0);
    let mut args: Vec<Object> = Vec::new();
    while out.len() < MAX_OPS {
        lx.skip_white();
        let Some(&b) = data.get(lx.pos) else { break };
        let starts_object = matches!(b, b'/' | b'(' | b'<' | b'[' | b'+' | b'-' | b'.' | b'0'..=b'9');
        let word = lx.peek_word();
        if starts_object || matches!(word, b"true" | b"false" | b"null") {
            let at = lx.pos;
            match lx.object(0, &|_| None) {
                Some(o) => {
                    if args.len() < MAX_ARGS {
                        args.push(o);
                    }
                }
                None => {
                    if lx.pos == at {
                        lx.pos += 1;
                    }
                }
            }
            continue;
        }
        if word.is_empty() {
            // A delimiter out of place (`]`, `>`, `}`...): skip it.
            lx.pos += 1;
            continue;
        }
        lx.pos += word.len();
        if word == b"BI" {
            if let Some((dict, image)) = inline_image(&mut lx) {
                out.push(Op { name: b"BI".to_vec(), args: vec![Object::Dict(dict), Object::String(image)] });
            }
            args.clear();
            continue;
        }
        out.push(Op { name: word.to_vec(), args: std::mem::take(&mut args) });
    }
    out
}

/// After `BI`: the dictionary to `ID`, then the data to `EI`.
fn inline_image(lx: &mut Lexer) -> Option<(Dict, Vec<u8>)> {
    let mut entries = Vec::new();
    loop {
        lx.skip_white();
        if lx.keyword(b"ID") {
            break;
        }
        let key = match lx.object(0, &|_| None)? {
            Object::Name(n) => n,
            _ => return None,
        };
        let value = lx.object(0, &|_| None).unwrap_or(Object::Null);
        entries.push((key, value));
        if entries.len() > 64 {
            return None;
        }
    }
    let dict = Dict(entries);
    // One white-space byte after ID, then the data.
    if lx.data.get(lx.pos).is_some_and(|b| is_white(*b)) {
        lx.pos += 1;
    }
    let start = lx.pos;
    let data = lx.data;
    let told = dict.int("L").or_else(|| dict.int("Length")).filter(|n| *n >= 0).map(|n| n as usize);
    let end = match told.filter(|n| start + n <= data.len()) {
        Some(n) => start + n,
        None => {
            // "EI" with white space (or the end) on both sides.
            let mut i = start;
            loop {
                if i + 2 > data.len() {
                    break data.len();
                }
                if &data[i..i + 2] == b"EI" && (i == start || is_white(data[i - 1])) && data.get(i + 2).is_none_or(|b| is_white(*b)) {
                    break if i > start { i - 1 } else { i };
                }
                i += 1;
            }
        }
    };
    let image = data[start..end].to_vec();
    lx.pos = end;
    lx.keyword(b"EI");
    Some((dict, image))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operations_with_operands() {
        let ops = parse(b"q 1 0 0 1 72 720 cm 0.5 g\n10 10 m 20 20 l S Q /F1 12 Tf (Hi) Tj [(A) -120 (B)] TJ");
        let names: Vec<&[u8]> = ops.iter().map(|o| o.name.as_slice()).collect();
        assert_eq!(names, [&b"q"[..], b"cm", b"g", b"m", b"l", b"S", b"Q", b"Tf", b"Tj", b"TJ"]);
        assert_eq!(ops[1].args.len(), 6);
        assert_eq!(ops[7].args, vec![Object::Name(b"F1".to_vec()), Object::Int(12)]);
        assert!(matches!(ops[9].args[0], Object::Array(_)));
    }

    #[test]
    fn an_inline_image() {
        let ops = parse(b"q BI /W 2 /H 1 /BPC 8 /CS /G ID \x00\xff EI Q");
        let bi = ops.iter().find(|o| o.name == b"BI").expect("an inline image");
        assert_eq!(bi.args[0].as_dict().unwrap().int("W"), Some(2));
        assert_eq!(bi.args[1], Object::String(vec![0x00, 0xff]));
        assert_eq!(ops.last().unwrap().name, b"Q");
    }

    #[test]
    fn junk_is_skipped() {
        let ops = parse(b"] } 1 2 re ) f");
        assert_eq!(ops.iter().map(|o| o.name.as_slice()).collect::<Vec<_>>(), [&b"re"[..], b"f"]);
    }
}
