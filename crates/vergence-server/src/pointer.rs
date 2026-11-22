// Mice and touchpads: the settings app's Mouse and touchpad page, applied to
// every pointing device libinput has, when it appears and again whenever the
// settings change (within a second, as key repeat is).
//
// ~/.config/vergence/desktop.toml:
//
//     [mouse]
//     speed = 0.0            # -1 (slowest) to 1 (fastest)
//     acceleration = true    # false: flat, the pointer goes as far as the hand
//     natural-scroll = false # true: the content follows the wheel, as on a Mac
//     left-handed = false    # true: the right button is the primary one
//     middle-click = false   # true: both buttons together are the middle one
//     scroll-lines = 3       # lines a notch of the wheel moves, 1 to 10
//     double-click = 400     # ms between two clicks that make a double-click
//
//     [touchpad]
//     speed = 0.0
//     tap-to-click = true
//     tap-and-drag = true
//     natural-scroll = true
//     scroll = "two-finger"  # or "edge"
//     disable-while-typing = true
//     click = "areas"        # the pad's corners; or "fingers": two fingers right-click
//     scroll-lines = 3
//
// Read by settings.rs into the session bus under GNOME's own names
// (org.gnome.desktop.peripherals.mouse and .touchpad), so toolkits that ask
// the settings portal follow too (GTK's double-click time). A device that
// cannot do something (a mouse has no tapping) is left as it is.
//
// The wheel's lines (scroll-lines) are the server's: it scales the amounts
// it hands on, so every application scrolls the same distance a notch.

use smithay::backend::input::AxisSource;
use smithay::reexports::input::{AccelProfile, ClickMethod, Device, DeviceCapability, ScrollMethod};
use vergence_protocol::bus::SettingValue;

use crate::settings::{MOUSE, TOUCHPAD};
use crate::state::Server;

/// Lines a notch moves when nothing says otherwise: what applications assume.
pub const SCROLL_LINES: u32 = 3;

/// A touchpad taps; a mouse does not. A Logitech multi-device receiver says
/// it has a touchpad whether or not anything paired with it has one:
/// settings given to it then reach nothing. So the
/// settings app's Touchpad page waits until one has scrolled with fingers
/// (inputs.rs).
pub fn is_touchpad(d: &Device) -> bool {
    d.config_tap_finger_count() > 0
}

impl Server {
    fn pointer_setting(&self, ns: &str, key: &str) -> Option<&SettingValue> {
        self.settings.current.get(&(ns.to_string(), key.to_string()))
    }

    fn pointer_flag(&self, ns: &str, key: &str, default: bool) -> bool {
        match self.pointer_setting(ns, key) {
            Some(SettingValue::Bool(b)) => *b,
            _ => default,
        }
    }

    /// A device appeared: a keyboard is kept and its lights set from our
    /// state (nothing else sets them: with libinput they are the
    /// compositor's to drive); a pointing one gets the settings and is kept.
    pub fn pointer_device_added(&mut self, mut d: Device) {
        tracing::info!(
            "input: device {:?} (touch {}, pointer {}, keyboard {})",
            d.name(),
            d.has_capability(DeviceCapability::Touch),
            d.has_capability(DeviceCapability::Pointer),
            d.has_capability(DeviceCapability::Keyboard)
        );
        self.inputs_device_added(&d);
        if d.has_capability(DeviceCapability::Keyboard) {
            self.keys_ready();
            d.led_update(self.lock_lights());
            self.keyboard_devices.push(d.clone());
        }
        if d.has_capability(DeviceCapability::Touch) && !self.touch_panels.contains(&d.name().to_string()) {
            tracing::info!("touch: panel {:?} present", d.name());
            self.touch_panels.push(d.name().to_string());
            self.redraw_all();
        }
        if !d.has_capability(DeviceCapability::Pointer) {
            return;
        }
        self.configure_pointer(&mut d);
        tracing::info!("pointer: {} ({})", d.name(), if is_touchpad(&d) { "touchpad" } else { "mouse" });
        self.pointer_devices.push(d);
    }

    pub fn pointer_device_removed(&mut self, d: &Device) {
        self.inputs_device_removed(d);
        self.keyboard_devices.retain(|k| k != d);
        if d.has_capability(DeviceCapability::Touch) {
            let name = d.name().to_string();
            self.touch_panels.retain(|n| *n != name);
            // The controls' size on its screen goes back from Large: drawn again.
            self.redraw_all();
        }
        self.pointer_devices.retain(|k| k != d);
    }

    /// The lock lights our keyboard state calls for (all off before the first key).
    pub fn lock_lights(&self) -> smithay::reexports::input::Led {
        use smithay::reexports::input::Led;
        let (num, caps, scroll) = self.input.keys.as_ref().map_or((false, false, false), |k| k.leds());
        let mut l = Led::empty();
        l.set(Led::NUMLOCK, num);
        l.set(Led::CAPSLOCK, caps);
        l.set(Led::SCROLLLOCK, scroll);
        l
    }

    /// Every keyboard's lights set again (a lock key changed them).
    pub fn update_lock_lights(&mut self) {
        let l = self.lock_lights();
        for d in &mut self.keyboard_devices {
            d.led_update(l);
        }
    }

    /// The settings changed: every pointing device gets them again.
    pub fn apply_pointer_settings(&mut self) {
        let mut devices = std::mem::take(&mut self.pointer_devices);
        for d in &mut devices {
            self.configure_pointer(d);
        }
        self.pointer_devices = devices;
    }

    fn configure_pointer(&self, d: &mut Device) {
        let pad = is_touchpad(d);
        let ns = if pad { TOUCHPAD } else { MOUSE };
        let speed = match self.pointer_setting(ns, "speed") {
            Some(SettingValue::F64(v)) => v.clamp(-1.0, 1.0),
            _ => 0.0,
        };
        // Each is tried; a device without the feature says so, and that is all.
        let _ = d.config_accel_set_speed(speed);
        if !pad {
            let profile = match self.pointer_setting(MOUSE, "accel-profile") {
                Some(SettingValue::Str(p)) if p == "flat" => AccelProfile::Flat,
                _ => AccelProfile::Adaptive,
            };
            let _ = d.config_accel_set_profile(profile);
            let _ = d.config_middle_emulation_set_enabled(self.pointer_flag(MOUSE, "middle-click-emulation", false));
        }
        let _ = d.config_scroll_set_natural_scroll_enabled(self.pointer_flag(ns, "natural-scroll", pad));
        tracing::info!(
            "pointer: {} gets speed {speed}{}, natural scrolling {}, left-handed {}",
            d.name(),
            if !pad && matches!(self.pointer_setting(MOUSE, "accel-profile"), Some(SettingValue::Str(p)) if p == "flat")
            {
                " flat"
            } else {
                ""
            },
            self.pointer_flag(ns, "natural-scroll", pad),
            self.pointer_flag(MOUSE, "left-handed", false)
        );
        let _ = d.config_left_handed_set(self.pointer_flag(MOUSE, "left-handed", false));
        if pad {
            let _ = d.config_tap_set_enabled(self.pointer_flag(TOUCHPAD, "tap-to-click", true));
            let _ = d.config_tap_set_drag_enabled(self.pointer_flag(TOUCHPAD, "tap-and-drag", true));
            let _ = d.config_dwt_set_enabled(self.pointer_flag(TOUCHPAD, "disable-while-typing", true));
            let method = if self.pointer_flag(TOUCHPAD, "edge-scrolling-enabled", false) {
                ScrollMethod::Edge
            } else {
                ScrollMethod::TwoFinger
            };
            let _ = d.config_scroll_set_method(method);
            let click = match self.pointer_setting(TOUCHPAD, "click-method") {
                Some(SettingValue::Str(m)) if m == "fingers" => ClickMethod::Clickfinger,
                _ => ClickMethod::ButtonAreas,
            };
            let _ = d.config_click_set_method(click);
        }
    }

    /// How much the scroll amounts from `source` are multiplied by: the
    /// lines a notch is to move, over the lines applications assume.
    pub fn scroll_factor(&self, source: AxisSource) -> f64 {
        let ns = match source {
            AxisSource::Wheel | AxisSource::WheelTilt => MOUSE,
            AxisSource::Finger => TOUCHPAD,
            AxisSource::Continuous => return 1.0,
        };
        match self.pointer_setting(ns, "scroll-lines") {
            Some(SettingValue::U32(n)) => (*n).clamp(1, 10) as f64 / SCROLL_LINES as f64,
            _ => 1.0,
        }
    }
}
