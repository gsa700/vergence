//! vsound: the machine's sound outputs from the terminal (sound milestone 1,
//! step 1), to try vergence-sound before the settings page has it.
//!
//!     vsound                      list the outputs
//!     vsound volume <id> <0-100>  set an output's volume
//!     vsound mute <id> on|off
//!     vsound default <id>         send sound there from now on (root on FreeBSD)
//!     vsound tone <id> [CHANNELS] [ON]
//!                                 a test tone, 440 Hz for 1.5 s, quiet: in a
//!                                 stream of CHANNELS channels (2), heard on those
//!                                 in ON (1,2), e.g. `vsound tone pcm3 4 3,4`

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(mut sound) = vergence_sound::open() else {
        #[cfg(target_os = "linux")]
        if vergence_sound::pipewire_running() {
            eprintln!("vsound: PipeWire (or PulseAudio) manages sound here; vergence-sound does not speak to it yet (a later milestone)");
            std::process::exit(1);
        }
        eprintln!("vsound: no sound system we know here");
        std::process::exit(1);
    };
    let result = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        [] => sound.outputs().map(|outs| {
            println!("{}:", sound.system());
            for o in outs {
                let volume = if o.adjustable { format!("{:>3}%", o.volume) } else { "   -".into() };
                println!("{} {:<6} {:<22} {volume}{} ({})", if o.default { "*" } else { " " }, o.id, o.name, if o.muted { " muted" } else { "" }, o.detail);
            }
        }),
        ["volume", id, v] => match v.parse::<u8>() {
            Ok(v) => sound.set_volume(id, v),
            Err(_) => Err(std::io::Error::other("volume is 0 to 100")),
        },
        ["mute", id, on @ ("on" | "off")] => sound.set_muted(id, *on == "on"),
        ["default", id] => sound.set_default(id),
        ["tone", id, rest @ ..] => {
            let channels = rest.first().and_then(|c| c.parse::<u32>().ok()).unwrap_or(2).clamp(1, 32);
            let on: Vec<u32> = rest.get(1).map(|o| o.split(',').filter_map(|c| c.parse().ok()).filter(|c| (1..=channels).contains(c)).collect()).unwrap_or_else(|| vec![1, 2.min(channels)]);
            sound.tone(id, channels, &on).map(|said| println!("{said}"))
        }
        _ => Err(std::io::Error::other("usage: vsound | volume <id> <0-100> | mute <id> on|off | default <id> | tone <id> [channels] [on]")),
    };
    if let Err(e) = result {
        eprintln!("vsound: {e}");
        std::process::exit(1);
    }
}
