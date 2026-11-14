//! Random and mutated input into the wire format and the bus messages: nothing
//! may panic, and what encodes must decode to the same thing (security pass
//! 2026-11-08). Deterministic (a fixed seed); FUZZ_ITERS raises the count for a
//! long run: `FUZZ_ITERS=2000000 cargo test --release -p vergence-protocol`.

use vergence_protocol::bus::{ClientMsg, ErrorCode, InputInfo, InputKind, OutputInfo, ServerMsg, Setting, SettingValue, Topic};
use vergence_protocol::wire::{FrameReader, MAX_FRAME};

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
                1 => ['é', '漢', '🙂', '\u{301}', '\u{200b}', '\0'][self.below(6)],
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

fn topic(rng: &mut Rng) -> Topic {
    [Topic::Outputs, Topic::Settings][rng.below(2)]
}

fn client(rng: &mut Rng) -> ClientMsg {
    let serial = rng.next() as u32;
    match rng.below(10) {
        8 => ClientMsg::ListPlaces { serial },
        9 => ClientMsg::ForgetPlaces { serial, app: rng.string(40) },
        0 => ClientMsg::Hello { version: rng.next() as u32, app_id: rng.string(60) },
        1 => ClientMsg::Ping { serial },
        2 => ClientMsg::Subscribe { serial, topic: topic(rng) },
        3 => ClientMsg::Unsubscribe { serial, topic: topic(rng) },
        4 => ClientMsg::GetOutputs { serial },
        5 => ClientMsg::GetSettings { serial },
        6 => ClientMsg::Notify {
            serial,
            replaces: rng.next() as u32,
            summary: rng.string(200),
            body: rng.string(2000),
            urgency: rng.next() as u8,
            timeout_ms: rng.next() as i32,
            actions: (0..rng.below(6)).map(|_| (rng.string(30), rng.string(30))).collect(),
        },
        _ => ClientMsg::CloseNotification { serial, id: rng.next() as u32 },
    }
}

fn output(rng: &mut Rng) -> OutputInfo {
    OutputInfo {
        id: rng.string(16),
        name: rng.string(12),
        description: rng.string(40),
        x: rng.next() as i32,
        y: rng.next() as i32,
        width: rng.next() as i32,
        height: rng.next() as i32,
        scale: rng.f64(),
        note: rng.string(80),
        default_scale: rng.f64(),
        base_scale: rng.f64(),
        shown_by: rng.string(40),
        drawn_by: rng.string(40),
        modes: (0..rng.below(6)).map(|_| vergence_protocol::bus::ModeInfo { width: rng.next() as u32, height: rng.next() as u32, refresh_mhz: rng.next() as u32, preferred: rng.below(2) == 1 }).collect(),
        mode: rng.next() as u32,
    }
}

fn setting(rng: &mut Rng) -> Setting {
    let value = match rng.below(6) {
        0 => SettingValue::Bool(rng.below(2) == 1),
        1 => SettingValue::U32(rng.next() as u32),
        2 => SettingValue::I32(rng.next() as i32),
        3 => SettingValue::F64(rng.f64()),
        4 => SettingValue::Str(rng.string(100)),
        _ => SettingValue::Rgb(rng.f64(), rng.f64(), rng.f64()),
    };
    Setting { namespace: rng.string(40), key: rng.string(40), value }
}

fn server(rng: &mut Rng) -> ServerMsg {
    let serial = rng.next() as u32;
    match rng.below(11) {
        9 => ServerMsg::Places { serial, apps: (0..rng.below(6)).map(|_| (rng.string(30), rng.next() as u32)).collect() },
        0 => ServerMsg::Welcome { version: rng.next() as u32, client_id: rng.next() as u32 },
        1 => ServerMsg::Pong { serial },
        2 => ServerMsg::Done { serial },
        3 => ServerMsg::Error { serial, code: ErrorCode::from_wire(rng.next() as u16), message: rng.string(100) },
        4 => ServerMsg::Outputs { serial, outputs: (0..rng.below(5)).map(|_| output(rng)).collect(), setup: rng.string(20) },
        5 => ServerMsg::Settings { serial, complete: rng.below(2) == 1, settings: (0..rng.below(12)).map(|_| setting(rng)).collect() },
        6 => ServerMsg::Notified { serial, id: rng.next() as u32 },
        7 => ServerMsg::NotificationClosed { id: rng.next() as u32, reason: rng.next() as u32 },
        8 => ServerMsg::Inputs {
            serial,
            devices: (0..rng.below(6)).map(|_| InputInfo { name: rng.string(60), kind: InputKind::from_wire(rng.next() as u8) }).collect(),
            seen: (0..rng.below(5)).map(|_| InputKind::from_wire(rng.next() as u8)).collect(),
        },
        _ => ServerMsg::ActionInvoked { id: rng.next() as u32, key: rng.string(40) },
    }
}

/// Feed `bytes` in random chunks; decode every frame both ways.
fn feed(rng: &mut Rng, reader: &mut FrameReader, bytes: &[u8]) -> Vec<Result<ClientMsg, ()>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let n = (1 + rng.below(64)).min(bytes.len() - i);
        reader.push(&bytes[i..i + n]);
        i += n;
        loop {
            match reader.next_frame() {
                Ok(Some(frame)) => {
                    let _ = ServerMsg::decode(&frame);
                    out.push(ClientMsg::decode(&frame).map_err(|_| ()));
                }
                Ok(None) => break,
                Err(_) => {
                    *reader = FrameReader::new();
                    break;
                }
            }
        }
    }
    out
}

#[test]
fn client_messages_round_trip() {
    let mut rng = Rng::new(1);
    for _ in 0..iters() {
        let m = client(&mut rng);
        let bytes = m.encode().expect("encodes");
        let mut reader = FrameReader::new();
        let got = feed(&mut rng, &mut reader, &bytes);
        assert_eq!(got, vec![Ok(m.clone())], "{m:?}");
        assert_eq!(reader.pending(), 0);
    }
}

#[test]
fn server_messages_round_trip() {
    let mut rng = Rng::new(2);
    for _ in 0..iters() {
        let m = server(&mut rng);
        let bytes = m.encode().expect("encodes");
        let mut reader = FrameReader::new();
        reader.push(&bytes);
        let frame = reader.next_frame().unwrap().expect("one frame");
        assert_eq!(ServerMsg::decode(&frame).unwrap(), m);
        assert_eq!(reader.pending(), 0);
    }
}

#[test]
fn mutated_frames_never_panic() {
    let mut rng = Rng::new(3);
    let mut reader = FrameReader::new();
    for _ in 0..iters() {
        let mut bytes = if rng.below(2) == 0 { client(&mut rng).encode().unwrap() } else { server(&mut rng).encode().unwrap() };
        let rounds = 1 + rng.below(4);
        for _ in 0..rounds {
            mutate(&mut rng, &mut bytes);
        }
        let _ = feed(&mut rng, &mut reader, &bytes);
        if reader.pending() > MAX_FRAME + 6 {
            reader = FrameReader::new();
        }
    }
}

#[test]
fn random_frames_never_panic() {
    let mut rng = Rng::new(4);
    let mut reader = FrameReader::new();
    for _ in 0..iters() {
        // A plausible header on random bytes, so the payload decoders run.
        let big = rng.below(10) == 0;
        let payload_len = rng.below(if big { 4000 } else { 40 });
        let len = (payload_len + 2) as u32;
        let mut bytes = len.to_le_bytes().to_vec();
        bytes.extend((rng.next() as u16 % 24).to_le_bytes());
        bytes.extend(rng.bytes(payload_len));
        if rng.below(5) == 0 {
            // Or no header at all.
            let n = rng.below(80);
            bytes = rng.bytes(n);
        }
        let _ = feed(&mut rng, &mut reader, &bytes);
    }
}

/// Window messages (native milestone 1): random and mutated frames, with a
/// random number of real descriptors waiting beside them. Nothing may panic;
/// a message never takes more descriptors than were there; what is refused
/// takes none it does not own (they close when dropped).
#[test]
fn window_messages_with_descriptors() {
    use std::collections::VecDeque;
    use std::os::fd::OwnedFd;
    use vergence_protocol::window::{Bar, Buffer, Damage, Fd, MenuEntry, Node, Place, Tab, WinEvent, WinRequest};
    let mut rng = Rng::new(0x5eed_0f_f1e5);
    let pipe = || Fd::new(vergence_protocol::sock::pipe().unwrap().0);
    // Good messages to mutate from.
    let seeds: Vec<WinRequest> = vec![
        WinRequest::Create { serial: 1, window: 1, role: "main".into(), token: String::new(), title: "t".into(), width: 640.0, height: 480.0, parent: 0, place: Place::Anywhere },
        WinRequest::Attach { window: 1, buffer: Buffer::Shm { fd: pipe(), width: 64, height: 32, stride: 256, format: 0 }, damage: vec![Damage { x: 0, y: 0, w: 8, h: 8 }] },
        WinRequest::Meaning { window: 1, focus: 2, caret: None, nodes: vec![Node { id: 2, parent: 0, role: "button".into(), name: "OK".into(), value: None, states: vec![], x: 0.0, y: 0.0, w: 10.0, h: 10.0 }] },
        WinRequest::Copy { input_serial: 3, kinds: vec!["text/plain".into()] },
        WinRequest::Style { window: 1, kind: vergence_protocol::window::WinKind::Dialog, modal: true, min: (320.0, 200.0), max: (1100.0, 0.0) },
        WinRequest::Bar {
            window: 1,
            bar: Bar::Tabs { tabs: vec![Tab { id: 1, title: "a".into(), closable: true, alert: false }, Tab { id: 2, title: "b".into(), closable: false, alert: true }], front: 2, plus: true, accent: Some(0xff0000), menu: vec![MenuEntry { id: 7, label: "m".into() }] },
        },
    ];
    let encoded: Vec<Vec<u8>> = seeds.iter().map(|m| m.encode().unwrap().0).collect();
    let events = [
        WinEvent::Keymap { fd: pipe(), size: 10 }.encode().unwrap().0,
        WinEvent::ClipboardData { serial: 1, fd: pipe() }.encode().unwrap().0,
        WinEvent::TabOut { window: 1, tab: 2, place: Place::Monitor { id: "m".into(), x: 1.0, y: 2.0 } }.encode().unwrap().0,
        WinEvent::TabJoin { window: 1, tab: 2, into: 3, to: 0 }.encode().unwrap().0,
    ];
    for round in 0..iters() {
        let mut data = if rng.below(3) == 0 { events[rng.below(events.len())].clone() } else { encoded[rng.below(encoded.len())].clone() };
        for _ in 0..rng.below(4) {
            mutate(&mut rng, &mut data);
        }
        let opcode = 100 + rng.below(30) as u16;
        let payload = if data.len() > 6 { &data[6..] } else { &[][..] };
        let waiting = rng.below(4);
        let mut fds: VecDeque<OwnedFd> = (0..waiting).map(|_| vergence_protocol::sock::pipe().unwrap().0).collect();
        let before = fds.len();
        if round % 2 == 0 {
            let _ = WinRequest::decode(opcode, payload, &mut fds);
        } else {
            let _ = WinEvent::decode(opcode, payload, &mut fds);
        }
        assert!(fds.len() <= before);
    }
}
