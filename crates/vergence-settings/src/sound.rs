//! The Sound page (Page::Sound): the outputs, their volume and mute, the
//! default, a test tone (vergence-sound).
//! The app's skeleton (the Program impl, the page list and routing, shared
//! state and helpers) is in main.rs.

use super::*;

impl App {
    /// The sound system and its outputs, read again.
    pub(crate) fn sound_read(&mut self) {
        if self.sound.is_none() {
            self.sound = vergence_sound::open();
        }
        self.sound_outputs = self.sound.as_mut().and_then(|s| s.outputs().ok()).unwrap_or_default();
    }

    /// A change to the sound system, then the outputs read again; a refusal
    /// goes to the status line in words.
    fn sound_do(&mut self, change: impl FnOnce(&mut dyn vergence_sound::Sound) -> std::io::Result<()>) {
        let Some(s) = self.sound.as_mut() else { return };
        match change(s.as_mut()) {
            Ok(()) => self.status.clear(),
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                // FreeBSD's default output (hw.snd.default_unit) is root's;
                // the helper says why it could not do it.
                self.status = format!("Choosing the output: {e}");
            }
            Err(e) => self.status = format!("Sound: {e}"),
        }
        self.sound_read();
    }

    pub(crate) fn sound_page(&self) -> Element<Message> {
        let title = "Sound";
        let about = "Where sound goes, and how loud. A program that does not choose plays to the default output.";
        let Some(system) = self.sound.as_ref().map(|s| s.system()) else {
            #[cfg(target_os = "linux")]
            let why = if vergence_sound::pipewire_running() {
                "PipeWire manages sound on this machine. Vergence will speak to it in a later milestone; until then, use its own tools."
            } else {
                "No sound system Vergence knows was found."
            };
            #[cfg(not(target_os = "linux"))]
            let why = "No sound system Vergence knows was found.";
            return self.page_layout(title, about, vec![self.group(None, vec![setting(why, "", space())])]);
        };
        // The output in use, with its controls; the others one line each under
        // "Other outputs", closed (Dave, 2026-11-22: "kind of busy ... just
        // show the device in use").
        let mut groups = Vec::new();
        let label_for = |o: &vergence_sound::Output| {
            // The system's name lives in Advanced, unless two outputs share a
            // name here: then it is how they are told apart.
            if self.sound_outputs.iter().filter(|x| x.name == o.name).count() > 1 {
                o.detail.clone()
            } else {
                String::new()
            }
        };
        if let Some(o) = self.sound_outputs.iter().find(|o| o.default) {
            let (vol_id, mute_id, tone_id) = (o.id.clone(), o.id.clone(), o.id.clone());
            let twin = label_for(o);
            let volume = if o.adjustable {
                setting(
                    "Volume",
                    &format!("{}%", o.volume),
                    slider(0.0, 100.0, o.volume as f32, move |v| Message::SoundVolume(vol_id.clone(), v))
                        .step(1.0)
                        .width(Length::Fixed(220.0))
                        .name(&format!("{} volume", o.name)),
                )
            } else {
                setting("Volume", "Set on the monitor or device itself.", space())
            };
            groups.push(self.group(
                Some(&o.name),
                vec![
                    setting(
                        "Sound goes here",
                        if twin.is_empty() { "Programs that do not choose play here." } else { twin.as_str() },
                        space(),
                    ),
                    volume,
                    setting(
                        "Mute",
                        "",
                        switch(o.muted, move |on| Message::SoundMute(mute_id.clone(), on))
                            .name(&format!("Mute {}", o.name)),
                    ),
                    setting(
                        "Test sound",
                        "A short tone, quiet.",
                        button(label("Play").size(12.0))
                            .padding(Padding::hv(4.0, 12.0))
                            .on_press(Message::SoundTone(tone_id))
                            .name(&format!("Test {}", o.name)),
                    ),
                ],
            ));
        }
        let others: Vec<Element<Message>> = self
            .sound_outputs
            .iter()
            .filter(|o| !o.default)
            .map(|o| {
                let id = o.id.clone();
                setting(
                    &o.name,
                    &label_for(o),
                    button(label("Use this").size(12.0))
                        .padding(Padding::hv(4.0, 12.0))
                        .on_press(Message::SoundDefault(id))
                        .name(&format!("Use {}", o.name)),
                )
            })
            .collect();
        if !others.is_empty() {
            let title = format!("Other outputs ({})", others.len());
            groups.push(self.disclosure("sound-others", &title, others));
        }
        if groups.is_empty() {
            groups.push(self.group(None, vec![setting("No outputs", system, space())]));
        }
        // The devices as the system names them.
        let mut raw = vec![setting("Sound system", "", label(system).dim())];
        for o in &self.sound_outputs {
            raw.push(setting(&o.name, &format!("{} ({})", o.detail, o.id), space()));
        }
        groups.push(self.advanced("sound", raw));
        groups.push(self.status_line());
        self.page_layout(title, about, groups)
    }

    /// This page's messages, as main.rs's handle routes them here.
    pub(crate) fn sound_message(&mut self, m: Message) {
        match m {
            Message::SoundVolume(id, v) => self.sound_do(|s| s.set_volume(&id, v.round() as u8)),
            Message::SoundMute(id, on) => self.sound_do(|s| s.set_muted(&id, on)),
            Message::SoundDefault(id) => self.sound_do(|s| s.set_default(&id)),
            Message::SoundTone(id) => {
                // 1.5 s of tone on its own thread (the window keeps answering),
                // through a sound system of its own.
                std::thread::spawn(move || {
                    if let Some(mut s) = vergence_sound::open() {
                        let _ = s.tone(&id, 2, &[1, 2]);
                    }
                });
            }
            _ => {}
        }
    }
}
