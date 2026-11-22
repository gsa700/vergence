//! Random and mutated input into the D-Bus wire protocol: nothing may panic,
//! and what serializes must parse back to the same thing (security pass
//! 2026-11-08). Deterministic (a fixed seed); FUZZ_ITERS raises the count for a
//! long run: `FUZZ_ITERS=2000000 cargo test --release -p vergence-dbus`.

use vergence_dbus::auth::{Auth, Step};
use vergence_dbus::matchrule::MatchRule;
use vergence_dbus::message::{self, frame_len};
use vergence_dbus::{Message, Value};

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next() as u8).collect()
    }
    fn string(&mut self, max: usize) -> String {
        let n = self.below(max + 1);
        (0..n)
            .map(|_| match self.below(6) {
                0 => char::from_u32(0x80 + self.below(0x7ff) as u32).unwrap_or('?'),
                1 => ['é', '漢', '🙂', '\u{301}', '\u{200b}'][self.below(5)],
                _ => (b' ' + self.below(95) as u8) as char,
            })
            .collect()
    }
    fn f64(&mut self) -> f64 {
        (self.next() as u32 as f64) / 7.0 - 1000.0
    }
}

fn iters() -> usize {
    std::env::var("FUZZ_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(20_000)
}

fn mutate(rng: &mut Rng, data: &mut Vec<u8>) {
    match rng.below(6) {
        0 if !data.is_empty() => {
            let i = rng.below(data.len());
            data[i] = rng.next() as u8;
        }
        1 if !data.is_empty() => {
            let i = rng.below(data.len());
            data[i] ^= 1 << rng.below(8);
        }
        2 if !data.is_empty() => {
            let n = rng.below(data.len());
            data.truncate(n);
        }
        3 => {
            let i = rng.below(data.len() + 1);
            data.insert(i, rng.next() as u8);
        }
        4 if !data.is_empty() => {
            let i = rng.below(data.len());
            data.remove(i);
        }
        _ if data.len() >= 4 => {
            let i = rng.below(data.len() - 3);
            let v = [0u8, 0xff, 0x7f, 0x80][rng.below(4)];
            data[i..i + 4].fill(v);
        }
        _ => {}
    }
}

const PATHS: [&str; 4] = ["/", "/org/freedesktop/Notifications", "/a/b0/_c", "/x"];
const SIGS: [&str; 5] = ["s", "i", "a{sv}", "(ii)", "aay"];

/// A well-formed value: valid paths and signatures, dictionary entries only in
/// arrays, every array's items of one type.
fn value(rng: &mut Rng, depth: usize) -> Value {
    let kinds = if depth > 3 { 13 } else { 17 };
    match rng.below(kinds) {
        0 => Value::Byte(rng.next() as u8),
        1 => Value::Bool(rng.below(2) == 1),
        2 => Value::I16(rng.next() as i16),
        3 => Value::U16(rng.next() as u16),
        4 => Value::I32(rng.next() as i32),
        5 => Value::U32(rng.next() as u32),
        6 => Value::I64(rng.next() as i64),
        7 => Value::U64(rng.next()),
        8 => Value::Double(rng.f64()),
        9 => Value::Str(rng.string(200)),
        10 => Value::Path(PATHS[rng.below(4)].into()),
        11 => Value::Sig(SIGS[rng.below(5)].into()),
        12 => Value::Fd(rng.next() as u32),
        13 => Value::Variant(Box::new(value(rng, depth + 1))),
        14 => Value::Struct((0..1 + rng.below(4)).map(|_| value(rng, depth + 1)).collect()),
        15 => {
            // A dictionary: basic keys, any values.
            let k = value(rng, 9); // depth 9: basic kinds only
            let v = value(rng, depth + 1);
            let n = rng.below(5);
            let items =
                (0..n).map(|_| Value::Entry(Box::new(same_shape(rng, &k)), Box::new(same_shape(rng, &v)))).collect();
            Value::Array(Value::Entry(Box::new(k), Box::new(v)).signature(), items)
        }
        _ => {
            let first = value(rng, depth + 1);
            let n = rng.below(6);
            let items = (0..n).map(|_| same_shape(rng, &first)).collect();
            Value::Array(first.signature(), items)
        }
    }
}

/// Another value of the same type.
fn same_shape(rng: &mut Rng, v: &Value) -> Value {
    match v {
        Value::Byte(_) => Value::Byte(rng.next() as u8),
        Value::Bool(_) => Value::Bool(rng.below(2) == 1),
        Value::I16(_) => Value::I16(rng.next() as i16),
        Value::U16(_) => Value::U16(rng.next() as u16),
        Value::I32(_) => Value::I32(rng.next() as i32),
        Value::U32(_) => Value::U32(rng.next() as u32),
        Value::I64(_) => Value::I64(rng.next() as i64),
        Value::U64(_) => Value::U64(rng.next()),
        Value::Double(_) => Value::Double(rng.f64()),
        Value::Str(_) => Value::Str(rng.string(100)),
        Value::Path(_) => Value::Path(PATHS[rng.below(4)].into()),
        Value::Sig(_) => Value::Sig(SIGS[rng.below(5)].into()),
        Value::Fd(_) => Value::Fd(rng.next() as u32),
        Value::Variant(_) => Value::Variant(Box::new(value(rng, 2))),
        Value::Struct(vs) => Value::Struct(vs.iter().map(|x| same_shape(rng, x)).collect()),
        Value::Entry(k, v) => Value::Entry(Box::new(same_shape(rng, k)), Box::new(same_shape(rng, v))),
        Value::Array(sig, items) => {
            let n = if items.is_empty() { 0 } else { rng.below(4) };
            Value::Array(sig.clone(), (0..n).map(|_| same_shape(rng, &items[0])).collect())
        }
    }
}

fn args(rng: &mut Rng) -> Vec<Value> {
    (0..rng.below(5)).map(|_| value(rng, 0)).collect()
}

fn message(rng: &mut Rng) -> (Message, Vec<Value>) {
    let a = args(rng);
    let dest = format!(":1.{}", rng.below(100));
    let mut m = match rng.below(3) {
        0 => Message::method_call(&dest, PATHS[rng.below(4)], "org.example.Iface", "Method", &a),
        1 => Message::signal(&dest, PATHS[rng.below(4)], "org.example.Iface", "Changed", &a),
        _ => {
            let call = Message::method_call(&dest, "/", "org.example.Iface", "M", &[]);
            if rng.below(2) == 0 {
                Message::method_return(&call, ":1.1", &a)
            } else {
                Message::error(&call, ":1.1", "org.example.Error", "text")
            }
        }
    };
    // (`big` is left alone: the constructors marshal the body at once, in
    // little-endian; flipping the flag afterwards would mislabel that body.)
    // The bus gives a message its serial before sending it; 0 is not one.
    m.serial = 1 + rng.next() as u32 % 1000;
    let a = if m.kind == message::ERROR { vec![Value::Str("text".into())] } else { a };
    (m, a)
}

#[test]
fn messages_round_trip() {
    let mut rng = Rng::new(1);
    for _ in 0..iters() {
        let (m, a) = message(&mut rng);
        let bytes = m.serialize();
        assert_eq!(frame_len(&bytes).unwrap(), Some(bytes.len()), "{m:?}");
        let p = Message::parse(&bytes).unwrap_or_else(|e| panic!("{e:?}: {m:?}"));
        assert_eq!(p.args().unwrap_or_else(|e| panic!("{e:?}: {m:?}")), a);
        assert_eq!(
            (p.kind, p.big, p.serial, &p.path, &p.member, &p.destination),
            (m.kind, m.big, m.serial, &m.path, &m.member, &m.destination)
        );
        assert_eq!(p.serialize(), bytes);
    }
}

#[test]
fn mutated_messages_never_panic() {
    let mut rng = Rng::new(2);
    for _ in 0..iters() {
        let (m, _) = message(&mut rng);
        let mut bytes = m.serialize();
        let rounds = 1 + rng.below(4);
        for _ in 0..rounds {
            mutate(&mut rng, &mut bytes);
        }
        let _ = frame_len(&bytes);
        if let Ok(p) = Message::parse(&bytes) {
            let _ = p.args();
            let _ = p.method();
            let _ = p.serialize();
        }
    }
}

#[test]
fn random_messages_never_panic() {
    let mut rng = Rng::new(3);
    for _ in 0..iters() {
        let bytes = if rng.below(3) == 0 {
            let n = rng.below(64);
            rng.bytes(n)
        } else {
            // A plausible fixed header on random fields and body.
            let big_fields = rng.below(10) == 0;
            let fields = rng.below(if big_fields { 2000 } else { 64 });
            let big_body = rng.below(10) == 0;
            let body = rng.below(if big_body { 2000 } else { 64 });
            let mut b = vec![if rng.below(4) == 0 { b'B' } else { b'l' }, 1 + rng.below(5) as u8, rng.next() as u8, 1];
            let (body_w, fields_w) = if b[0] == b'B' {
                ((body as u32).to_be_bytes(), (fields as u32).to_be_bytes())
            } else {
                ((body as u32).to_le_bytes(), (fields as u32).to_le_bytes())
            };
            b.extend(body_w);
            b.extend((rng.next() as u32).to_le_bytes());
            b.extend(fields_w);
            b.extend(rng.bytes((16 + fields).div_ceil(8) * 8 - 16 + body));
            b
        };
        let _ = frame_len(&bytes);
        if let Ok(p) = Message::parse(&bytes) {
            let _ = p.args();
            let _ = p.serialize();
        }
    }
}

#[test]
fn auth_never_panics() {
    let mut rng = Rng::new(4);
    const TOKENS: [&str; 12] = [
        "AUTH",
        " ",
        "EXTERNAL",
        "31303030",
        "DATA",
        "BEGIN",
        "NEGOTIATE_UNIX_FD",
        "CANCEL",
        "ERROR",
        "\r\n",
        "\0",
        "zz",
    ];
    for _ in 0..iters() {
        let mut a = Auth::new(1000, "0123456789abcdef0123456789abcdef");
        if rng.below(3) == 0 {
            a.feed(&[0]);
        }
        let chunks = 1 + rng.below(8);
        for _ in 0..chunks {
            let mut chunk: Vec<u8> = if rng.below(4) == 0 {
                let n = rng.below(40);
                rng.bytes(n)
            } else {
                let n = 1 + rng.below(6);
                (0..n).flat_map(|_| TOKENS[rng.below(12)].bytes()).collect::<Vec<u8>>()
            };
            if rng.below(8) == 0 {
                chunk = rng.bytes(20_000);
            }
            match a.feed(&chunk) {
                Step::Continue(_) => {}
                Step::Done(..) | Step::Fail => break,
            }
        }
    }
}

#[test]
fn match_rules_never_panic() {
    let mut rng = Rng::new(5);
    const TOKENS: [&str; 16] = [
        "type",
        "=",
        "'",
        "signal",
        "'",
        ",",
        "sender",
        "member",
        "path",
        "path_namespace",
        "arg",
        "0",
        "99999999999999999999",
        "path",
        "\\'",
        "eavesdrop='true'",
    ];
    for _ in 0..iters() {
        let text: String = if rng.below(4) == 0 {
            rng.string(60)
        } else {
            let n = rng.below(12);
            (0..n).map(|_| if rng.below(6) == 0 { rng.string(4) } else { TOKENS[rng.below(16)].to_string() }).collect()
        };
        if let Some(rule) = MatchRule::parse(&text) {
            let (m, _) = message(&mut rng);
            let _ = rule.matches(&m, &[":1.5".into(), "org.example".into()]);
        }
    }
}
