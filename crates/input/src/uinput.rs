//! Linux `KeySink`: a uinput virtual keyboard (spec 05). Ported from v2 `input/linux.rs`.

use std::time::{Duration, Instant};

use arrowhead_core::input::{InputError, KeyAction, KeySink};
use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, InputEvent, KeyCode, KeyEvent};

pub fn key_code(name: &str) -> Option<KeyCode> {
    Some(match name.to_lowercase().as_str() {
        "a" => KeyCode::KEY_A,
        "s" => KeyCode::KEY_S,
        "w" => KeyCode::KEY_W,
        "d" => KeyCode::KEY_D,
        "left" => KeyCode::KEY_LEFT,
        "down" => KeyCode::KEY_DOWN,
        "up" => KeyCode::KEY_UP,
        "right" => KeyCode::KEY_RIGHT,
        "f" => KeyCode::KEY_F,
        "j" => KeyCode::KEY_J,
        "k" => KeyCode::KEY_K,
        "l" => KeyCode::KEY_L,
        "z" => KeyCode::KEY_Z,
        "x" => KeyCode::KEY_X,
        "space" => KeyCode::KEY_SPACE,
        _ => return None,
    })
}

pub struct UinputSink {
    dev: VirtualDevice,
    keys: [KeyCode; 4],
    events: Vec<InputEvent>,
}

impl UinputSink {
    pub const NAME: &'static str = "arrowhead virtual keyboard";

    /// Creates the device with only the 4 lane keys, then waits (max 1 s) until it shows up
    /// in `/dev/input` so the first press is not lost.
    pub fn new(names: [&str; 4]) -> Result<Self, InputError> {
        let mut keys = [KeyCode::KEY_A; 4];
        let mut set = AttributeSet::<KeyCode>::new();
        for (i, n) in names.iter().enumerate() {
            keys[i] = key_code(n).ok_or_else(|| InputError(format!("unknown key {n:?}")))?;
            set.insert(keys[i]);
        }
        let err = |e: std::io::Error| {
            InputError(format!(
                "uinput: {e} (is /dev/uinput writable for this user?)"
            ))
        };
        let mut dev = VirtualDevice::builder()
            .map_err(err)?
            .name(Self::NAME)
            .with_keys(&set)
            .map_err(err)?
            .build()
            .map_err(err)?;
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_secs(1) {
            if dev
                .enumerate_dev_nodes_blocking()
                .map_err(err)?
                .next()
                .is_some()
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        // Let the compositor pick the new device up before the first key.
        std::thread::sleep(Duration::from_millis(300));
        Ok(Self {
            dev,
            keys,
            events: Vec::with_capacity(8),
        })
    }
}

impl KeySink for UinputSink {
    fn send_batch(&mut self, actions: &[KeyAction]) -> Result<(), InputError> {
        self.events.clear();
        for a in actions {
            let (lane, v) = match *a {
                KeyAction::Press(l) => (l, 1),
                KeyAction::Release(l) => (l, 0),
            };
            self.events
                .push(*KeyEvent::new(self.keys[lane as usize], v));
        }
        // One emit = one SYN_REPORT: the batch lands in a single input frame.
        self.dev
            .emit(&self.events)
            .map_err(|e| InputError(e.to_string()))
    }

    fn release_all(&mut self) -> Result<(), InputError> {
        let up: Vec<InputEvent> = self.keys.iter().map(|&k| *KeyEvent::new(k, 0)).collect();
        self.dev.emit(&up).map_err(|e| InputError(e.to_string()))
    }

    fn name(&self) -> &'static str {
        "uinput"
    }
}

impl Drop for UinputSink {
    fn drop(&mut self) {
        // S2. Destroying the device also makes the kernel release its keys.
        let _ = self.release_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_sets_map() {
        use arrowhead_core::config::KeySet;
        let none: [String; 4] = Default::default();
        for set in [KeySet::Wasd, KeySet::Arrows] {
            let names = arrowhead_core::input::key_names(set, &none);
            assert!(names.iter().all(|n| key_code(n).is_some()), "{set:?}");
        }
        assert!(key_code("nope").is_none());
    }
}
