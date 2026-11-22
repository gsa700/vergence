//! PDF functions (ISO 32000-2 §7.10): sampled (type 0), exponential (2),
//! stitching (3) and PostScript calculator (4). Separation and DeviceN
//! colors run them to get a color, shadings to get one along a gradient.
//!
//! Untrusted: sample tables and nesting are bounded, a calculator program
//! runs a bounded number of steps on a bounded stack.

use crate::Document;
use crate::object::{Dict, Object};

const MAX_INPUTS: usize = 8;
const MAX_OUTPUTS: usize = 32;
const MAX_SAMPLES: usize = 1 << 24;
const MAX_DEPTH: usize = 8;
const MAX_STEPS: usize = 100_000;
const MAX_STACK: usize = 100;

#[derive(Debug, Clone)]
pub enum Function {
    Sampled {
        domain: Vec<f64>,
        range: Vec<f64>,
        size: Vec<usize>,
        encode: Vec<f64>,
        decode: Vec<f64>,
        samples: Vec<f64>,
        outputs: usize,
    },
    Exponential {
        domain: Vec<f64>,
        c0: Vec<f64>,
        c1: Vec<f64>,
        n: f64,
    },
    Stitching {
        domain: Vec<f64>,
        functions: Vec<Function>,
        bounds: Vec<f64>,
        encode: Vec<f64>,
    },
    Calculator {
        domain: Vec<f64>,
        range: Vec<f64>,
        program: Vec<Op>,
    },
    /// Several one-output functions, one per output (an array of them).
    Each(Vec<Function>),
}

fn nums(o: Option<&Object>, doc: &Document) -> Vec<f64> {
    match o.map(|o| doc.resolve(o)) {
        Some(Object::Array(a)) => a.iter().filter_map(|x| doc.resolve(x).as_f64()).collect(),
        Some(o) => o.as_f64().into_iter().collect(),
        None => Vec::new(),
    }
}

impl Function {
    /// From a function object (a dictionary, a stream, or an array of them).
    pub fn parse(o: &Object, doc: &Document) -> Option<Function> {
        Self::parse_at(o, doc, 0)
    }

    fn parse_at(o: &Object, doc: &Document, depth: usize) -> Option<Function> {
        if depth > MAX_DEPTH {
            return None;
        }
        let o = doc.resolve(o);
        if let Object::Array(a) = &o {
            let fs: Vec<Function> =
                a.iter().take(MAX_OUTPUTS).filter_map(|f| Self::parse_at(f, doc, depth + 1)).collect();
            return (!fs.is_empty()).then_some(Function::Each(fs));
        }
        let d: Dict = o.as_dict()?.clone();
        let domain = nums(d.get("Domain"), doc);
        if domain.len() < 2 {
            return None;
        }
        match d.int("FunctionType")? {
            0 => {
                let Object::Stream(s) = &o else { return None };
                let size: Vec<usize> = nums(d.get("Size"), doc).iter().map(|v| v.clamp(1.0, 1e6) as usize).collect();
                let range = nums(d.get("Range"), doc);
                let inputs = (domain.len() / 2).min(MAX_INPUTS);
                let outputs = (range.len() / 2).min(MAX_OUTPUTS);
                if size.len() < inputs || outputs == 0 || inputs == 0 {
                    return None;
                }
                let count = size[..inputs].iter().try_fold(1usize, |a, s| a.checked_mul(*s))?.checked_mul(outputs)?;
                if count > MAX_SAMPLES {
                    return None;
                }
                let bps = d.int("BitsPerSample").unwrap_or(8);
                if ![1, 2, 4, 8, 12, 16, 24, 32].contains(&bps) {
                    return None;
                }
                let data = doc.stream(s).data;
                let max = ((1u64 << bps) - 1) as f64;
                let mut reader = Bits::new(&data);
                let samples: Vec<f64> = (0..count).map(|_| reader.read(bps as u32).unwrap_or(0) as f64 / max).collect();
                let mut encode = nums(d.get("Encode"), doc);
                if encode.len() < inputs * 2 {
                    encode = size[..inputs].iter().flat_map(|s| [0.0, (*s as f64 - 1.0).max(0.0)]).collect();
                }
                let mut decode = nums(d.get("Decode"), doc);
                if decode.len() < outputs * 2 {
                    decode = range.clone();
                }
                Some(Function::Sampled { domain, range, size, encode, decode, samples, outputs })
            }
            2 => {
                let c0 = d.get("C0").map(|o| nums(Some(o), doc)).unwrap_or_else(|| vec![0.0]);
                let c1 = d.get("C1").map(|o| nums(Some(o), doc)).unwrap_or_else(|| vec![1.0]);
                let n = d.get("N").and_then(|o| doc.resolve(o).as_f64()).unwrap_or(1.0);
                (c0.len() == c1.len() && c0.len() <= MAX_OUTPUTS).then_some(Function::Exponential { domain, c0, c1, n })
            }
            3 => {
                let functions: Vec<Function> = match doc.get_in(&d, "Functions") {
                    Object::Array(a) => a.iter().take(256).filter_map(|f| Self::parse_at(f, doc, depth + 1)).collect(),
                    _ => return None,
                };
                let bounds = nums(d.get("Bounds"), doc);
                let encode = nums(d.get("Encode"), doc);
                (!functions.is_empty() && bounds.len() + 1 == functions.len() && encode.len() >= functions.len() * 2)
                    .then_some(Function::Stitching { domain, functions, bounds, encode })
            }
            4 => {
                let Object::Stream(s) = &o else { return None };
                let program = parse_program(&doc.stream(s).data)?;
                Some(Function::Calculator { domain, range: nums(d.get("Range"), doc), program })
            }
            _ => None,
        }
    }

    /// The outputs for `input`.
    pub fn eval(&self, input: &[f64]) -> Vec<f64> {
        match self {
            Function::Each(fs) => fs.iter().map(|f| f.eval(input).first().copied().unwrap_or(0.0)).collect(),
            Function::Exponential { domain, c0, c1, n } => {
                let x = clip(input.first().copied().unwrap_or(0.0), domain[0], domain[1]);
                let xn = if *n == 1.0 { x } else { x.max(0.0).powf(*n) };
                c0.iter().zip(c1).map(|(a, b)| a + xn * (b - a)).collect()
            }
            Function::Stitching { domain, functions, bounds, encode } => {
                let x = clip(input.first().copied().unwrap_or(0.0), domain[0], domain[1]);
                let k = bounds.iter().position(|b| x < *b).unwrap_or(bounds.len());
                let lo = if k == 0 { domain[0] } else { bounds[k - 1] };
                let hi = if k == bounds.len() { domain[1] } else { bounds[k] };
                let e = interpolate(x, lo, hi, encode[2 * k], encode[2 * k + 1]);
                functions[k].eval(&[e])
            }
            Function::Sampled { domain, range, size, encode, decode, samples, outputs } => {
                let inputs = (domain.len() / 2).min(size.len());
                // Each input to a fractional sample index.
                let mut idx = Vec::with_capacity(inputs);
                for i in 0..inputs {
                    let x = clip(input.get(i).copied().unwrap_or(0.0), domain[2 * i], domain[2 * i + 1]);
                    let e = interpolate(x, domain[2 * i], domain[2 * i + 1], encode[2 * i], encode[2 * i + 1]);
                    idx.push(clip(e, 0.0, size[i] as f64 - 1.0));
                }
                // Multilinear interpolation over the 2^inputs corners.
                let mut out = vec![0.0; *outputs];
                for corner in 0..(1usize << inputs) {
                    let mut weight = 1.0;
                    let mut offset = 0usize;
                    let mut stride = 1usize;
                    for i in 0..inputs {
                        let lo = idx[i].floor();
                        let f = idx[i] - lo;
                        let hi_bit = corner >> i & 1 == 1;
                        let pos = if hi_bit { (lo as usize + 1).min(size[i] - 1) } else { lo as usize };
                        weight *= if hi_bit { f } else { 1.0 - f };
                        offset += pos * stride;
                        stride *= size[i];
                    }
                    if weight == 0.0 {
                        continue;
                    }
                    for (j, o) in out.iter_mut().enumerate() {
                        *o += weight * samples.get(offset * outputs + j).copied().unwrap_or(0.0);
                    }
                }
                out.iter()
                    .enumerate()
                    .map(|(j, v)| {
                        let d = interpolate(*v, 0.0, 1.0, decode[2 * j], decode[2 * j + 1]);
                        clip(d, range[2 * j], range[2 * j + 1])
                    })
                    .collect()
            }
            Function::Calculator { domain, range, program } => {
                let mut stack: Vec<Value> = input
                    .iter()
                    .enumerate()
                    .map(|(i, x)| {
                        Value::Num(clip(
                            *x,
                            domain.get(2 * i).copied().unwrap_or(f64::MIN),
                            domain.get(2 * i + 1).copied().unwrap_or(f64::MAX),
                        ))
                    })
                    .collect();
                let mut steps = 0;
                run(program, &mut stack, &mut steps);
                let outs = range.len() / 2;
                let vals: Vec<f64> = stack.iter().map(Value::num).collect();
                let start = vals.len().saturating_sub(outs);
                (0..outs)
                    .map(|j| clip(vals.get(start + j).copied().unwrap_or(0.0), range[2 * j], range[2 * j + 1]))
                    .collect()
            }
        }
    }
}

fn clip(x: f64, lo: f64, hi: f64) -> f64 {
    if !x.is_finite() {
        return lo;
    }
    if lo <= hi { x.clamp(lo, hi) } else { x }
}

fn interpolate(x: f64, x0: f64, x1: f64, y0: f64, y1: f64) -> f64 {
    if x1 == x0 { y0 } else { y0 + (x - x0) * (y1 - y0) / (x1 - x0) }
}

/// Bits read most significant first.
pub struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    bit: u32,
}

impl<'a> Bits<'a> {
    pub fn new(data: &'a [u8]) -> Bits<'a> {
        Bits { data, pos: 0, bit: 0 }
    }

    pub fn read(&mut self, n: u32) -> Option<u64> {
        let mut v = 0u64;
        for _ in 0..n {
            let byte = *self.data.get(self.pos)?;
            v = v << 1 | ((byte >> (7 - self.bit)) & 1) as u64;
            self.bit += 1;
            if self.bit == 8 {
                self.bit = 0;
                self.pos += 1;
            }
        }
        Some(v)
    }

    /// To the next whole byte (image rows start on one).
    pub fn align(&mut self) {
        if self.bit != 0 {
            self.bit = 0;
            self.pos += 1;
        }
    }
}

/// A calculator program, parsed.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    Num(f64),
    Bool(bool),
    Op(String),
    /// `{ ... } if` or `{ ... } { ... } ifelse`.
    If(Vec<Op>, Option<Vec<Op>>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Value {
    Num(f64),
    Bool(bool),
}

impl Value {
    fn num(&self) -> f64 {
        match self {
            Value::Num(n) => *n,
            Value::Bool(b) => *b as i32 as f64,
        }
    }
}

fn parse_program(src: &[u8]) -> Option<Vec<Op>> {
    let text = String::from_utf8_lossy(src);
    let spaced = text.replace('{', " { ").replace('}', " } ");
    let mut tokens = spaced.split_whitespace().peekable();
    if tokens.next()? != "{" {
        return None;
    }
    parse_block(&mut tokens, 0)
}

fn parse_block<'a>(tokens: &mut std::iter::Peekable<impl Iterator<Item = &'a str>>, depth: usize) -> Option<Vec<Op>> {
    if depth > 16 {
        return None;
    }
    let mut out = Vec::new();
    while let Some(t) = tokens.next() {
        match t {
            "}" => return Some(out),
            "{" => {
                let first = parse_block(tokens, depth + 1)?;
                if tokens.peek() == Some(&"{") {
                    tokens.next();
                    let second = parse_block(tokens, depth + 1)?;
                    if tokens.next()? != "ifelse" {
                        return None;
                    }
                    out.push(Op::If(first, Some(second)));
                } else {
                    if tokens.next()? != "if" {
                        return None;
                    }
                    out.push(Op::If(first, None));
                }
            }
            "true" => out.push(Op::Bool(true)),
            "false" => out.push(Op::Bool(false)),
            _ => match t.parse::<f64>() {
                Ok(n) => out.push(Op::Num(n)),
                Err(_) => out.push(Op::Op(t.to_string())),
            },
        }
        if out.len() > 10_000 {
            return None;
        }
    }
    None
}

fn run(program: &[Op], st: &mut Vec<Value>, steps: &mut usize) {
    for op in program {
        *steps += 1;
        if *steps > MAX_STEPS || st.len() > MAX_STACK {
            return;
        }
        match op {
            Op::Num(n) => st.push(Value::Num(*n)),
            Op::Bool(b) => st.push(Value::Bool(*b)),
            Op::If(a, b) => {
                let cond = match st.pop() {
                    Some(Value::Bool(c)) => c,
                    Some(Value::Num(n)) => n != 0.0,
                    None => return,
                };
                if cond {
                    run(a, st, steps);
                } else if let Some(b) = b {
                    run(b, st, steps);
                }
            }
            Op::Op(name) => {
                if !operator(name, st) {
                    return;
                }
            }
        }
    }
}

/// One operator; false on stack underflow or an unknown name (the
/// program stops).
fn operator(name: &str, st: &mut Vec<Value>) -> bool {
    let pop = |st: &mut Vec<Value>| st.pop();
    macro_rules! num1 {
        ($f:expr) => {{
            let Some(a) = pop(st) else { return false };
            st.push(Value::Num($f(a.num())));
        }};
    }
    macro_rules! num2 {
        ($f:expr) => {{
            let (Some(b), Some(a)) = (pop(st), pop(st)) else { return false };
            st.push(Value::Num($f(a.num(), b.num())));
        }};
    }
    macro_rules! cmp {
        ($f:expr) => {{
            let (Some(b), Some(a)) = (pop(st), pop(st)) else { return false };
            st.push(Value::Bool($f(a.num(), b.num())));
        }};
    }
    match name {
        "abs" => num1!(f64::abs),
        "neg" => num1!(|a: f64| -a),
        "ceiling" => num1!(f64::ceil),
        "floor" => num1!(f64::floor),
        "round" => num1!(|a: f64| (a + 0.5).floor()),
        "truncate" | "cvi" => num1!(f64::trunc),
        "cvr" => num1!(|a| a),
        "sqrt" => num1!(|a: f64| a.max(0.0).sqrt()),
        "sin" => num1!(|a: f64| a.to_radians().sin()),
        "cos" => num1!(|a: f64| a.to_radians().cos()),
        "ln" => num1!(|a: f64| a.ln()),
        "log" => num1!(|a: f64| a.log10()),
        "add" => num2!(|a, b| a + b),
        "sub" => num2!(|a, b| a - b),
        "mul" => num2!(|a, b| a * b),
        "div" => num2!(|a, b| if b == 0.0 { 0.0 } else { a / b }),
        "idiv" => num2!(|a: f64, b: f64| if b.trunc() == 0.0 { 0.0 } else { (a.trunc() / b.trunc()).trunc() }),
        "mod" => num2!(|a: f64, b: f64| if b.trunc() == 0.0 { 0.0 } else { a.trunc() % b.trunc() }),
        "exp" => num2!(|a: f64, b| a.powf(b)),
        "atan" => num2!(|a: f64, b: f64| {
            let d = a.atan2(b).to_degrees();
            if d < 0.0 { d + 360.0 } else { d }
        }),
        "bitshift" => num2!(|a: f64, b: f64| {
            let (a, b) = (a as i64, b as i64);
            (if b >= 0 { a.checked_shl(b.min(63) as u32).unwrap_or(0) } else { a >> (-b).min(63) }) as f64
        }),
        "eq" => cmp!(|a, b| a == b),
        "ne" => cmp!(|a, b| a != b),
        "gt" => cmp!(|a, b| a > b),
        "ge" => cmp!(|a, b| a >= b),
        "lt" => cmp!(|a, b| a < b),
        "le" => cmp!(|a, b| a <= b),
        "and" | "or" | "xor" => {
            let (Some(b), Some(a)) = (pop(st), pop(st)) else { return false };
            st.push(match (a, b) {
                (Value::Bool(x), Value::Bool(y)) => Value::Bool(match name {
                    "and" => x && y,
                    "or" => x || y,
                    _ => x ^ y,
                }),
                (x, y) => {
                    let (x, y) = (x.num() as i64, y.num() as i64);
                    Value::Num(match name {
                        "and" => x & y,
                        "or" => x | y,
                        _ => x ^ y,
                    } as f64)
                }
            });
        }
        "not" => {
            let Some(a) = pop(st) else { return false };
            st.push(match a {
                Value::Bool(b) => Value::Bool(!b),
                Value::Num(n) => Value::Num(!(n as i64) as f64),
            });
        }
        "dup" => {
            let Some(a) = st.last().copied() else { return false };
            st.push(a);
        }
        "pop" => {
            if pop(st).is_none() {
                return false;
            }
        }
        "exch" => {
            let n = st.len();
            if n < 2 {
                return false;
            }
            st.swap(n - 1, n - 2);
        }
        "copy" => {
            let Some(k) = pop(st) else { return false };
            let k = k.num().max(0.0) as usize;
            if k > st.len() || st.len() + k > MAX_STACK {
                return false;
            }
            let from = st.len() - k;
            let copied: Vec<Value> = st[from..].to_vec();
            st.extend(copied);
        }
        "index" => {
            let Some(k) = pop(st) else { return false };
            let k = k.num().max(0.0) as usize;
            let Some(v) = st.len().checked_sub(k + 1).and_then(|i| st.get(i)).copied() else { return false };
            st.push(v);
        }
        "roll" => {
            let (Some(j), Some(n)) = (pop(st), pop(st)) else { return false };
            let n = n.num().max(0.0) as usize;
            if n > st.len() || n == 0 {
                return n == 0;
            }
            let j = (j.num() as i64).rem_euclid(n as i64) as usize;
            let from = st.len() - n;
            st[from..].rotate_right(j);
        }
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exponential_and_stitching() {
        let f = Function::Exponential { domain: vec![0.0, 1.0], c0: vec![0.0, 0.0], c1: vec![1.0, 0.5], n: 1.0 };
        assert_eq!(f.eval(&[0.5]), vec![0.5, 0.25]);
        let g = Function::Stitching {
            domain: vec![0.0, 1.0],
            functions: vec![
                Function::Exponential { domain: vec![0.0, 1.0], c0: vec![0.0], c1: vec![1.0], n: 1.0 },
                Function::Exponential { domain: vec![0.0, 1.0], c0: vec![1.0], c1: vec![0.0], n: 1.0 },
            ],
            bounds: vec![0.5],
            encode: vec![0.0, 1.0, 0.0, 1.0],
        };
        assert_eq!(g.eval(&[0.25]), vec![0.5]);
        assert_eq!(g.eval(&[0.75]), vec![0.5]);
    }

    #[test]
    fn calculator_programs() {
        let prog = parse_program(b"{ dup 0.5 gt { 1 exch sub } if 2 mul }").unwrap();
        let f = Function::Calculator { domain: vec![0.0, 1.0], range: vec![0.0, 2.0], program: prog };
        assert_eq!(f.eval(&[0.25]), vec![0.5]);
        assert_eq!(f.eval(&[0.75]), vec![0.5]);
        // A CMYK-from-tint transform of the usual kind: tint -> 4 outputs.
        let prog = parse_program(b"{ dup 0 mul exch dup 1 mul exch dup 0.5 mul exch 0 mul }").unwrap();
        let f = Function::Calculator {
            domain: vec![0.0, 1.0],
            range: vec![0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0],
            program: prog,
        };
        assert_eq!(f.eval(&[1.0]), vec![0.0, 1.0, 0.5, 0.0]);
        // Runaway or broken programs stop.
        assert!(parse_program(b"{ 1 2 ").is_none());
        let bomb = parse_program(b"{ 1 1 1 1 1 1 1 1 1 1 10 copy 10 copy 10 copy 10 copy 10 copy 10 copy 10 copy 10 copy 10 copy 10 copy 10 copy }").unwrap();
        let f = Function::Calculator { domain: vec![0.0, 1.0], range: vec![0.0, 1.0], program: bomb };
        let _ = f.eval(&[0.0]);
    }

    #[test]
    fn sampled_interpolates() {
        // One input, two samples 0 and 255: halfway is half.
        let f = Function::Sampled {
            domain: vec![0.0, 1.0],
            range: vec![0.0, 1.0],
            size: vec![2],
            encode: vec![0.0, 1.0],
            decode: vec![0.0, 1.0],
            samples: vec![0.0, 1.0],
            outputs: 1,
        };
        assert_eq!(f.eval(&[0.5]), vec![0.5]);
        assert_eq!(f.eval(&[2.0]), vec![1.0], "clipped to the domain");
    }
}
