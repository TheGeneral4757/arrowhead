//! Windows `KeySink`: `SendInput` with scan codes (spec 05). Ported from v2 `input/windows.rs`.

use arrowhead_core::input::{InputError, KeyAction, KeySink};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, SendInput, VIRTUAL_KEY,
};

use crate::winutil::scan_code;

pub struct SendInputSink {
    keys: [(u16, bool); 4],
    inputs: Vec<INPUT>,
}

fn key_input((scan, extended): (u16, bool), up: bool) -> INPUT {
    let mut flags = KEYEVENTF_SCANCODE;
    if extended {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    if up {
        flags |= KEYEVENTF_KEYUP;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(0),
                wScan: scan,
                dwFlags: KEYBD_EVENT_FLAGS(flags.0),
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

impl SendInputSink {
    pub fn new(names: [&str; 4]) -> Result<Self, InputError> {
        let mut keys = [(0, false); 4];
        for (i, n) in names.iter().enumerate() {
            keys[i] = scan_code(n).ok_or_else(|| InputError(format!("unknown key {n:?}")))?;
        }
        Ok(Self {
            keys,
            inputs: Vec::with_capacity(8),
        })
    }

    fn send(&self) -> Result<(), InputError> {
        if self.inputs.is_empty() {
            return Ok(());
        }
        // SAFETY: a valid slice of INPUT structs and their size.
        let n = unsafe { SendInput(&self.inputs, std::mem::size_of::<INPUT>() as i32) };
        if n as usize == self.inputs.len() {
            Ok(())
        } else {
            // 0 usually means UIPI blocked it (the game runs elevated and we do not).
            Err(InputError(format!(
                "SendInput sent {n} of {} events (is the game running as administrator?)",
                self.inputs.len()
            )))
        }
    }
}

impl KeySink for SendInputSink {
    fn send_batch(&mut self, actions: &[KeyAction]) -> Result<(), InputError> {
        self.inputs.clear();
        for a in actions {
            let (lane, up) = match *a {
                KeyAction::Press(l) => (l, false),
                KeyAction::Release(l) => (l, true),
            };
            self.inputs.push(key_input(self.keys[lane as usize], up));
        }
        self.send()
    }

    fn release_all(&mut self) -> Result<(), InputError> {
        self.inputs.clear();
        self.inputs
            .extend(self.keys.iter().map(|&k| key_input(k, true)));
        self.send()
    }

    fn name(&self) -> &'static str {
        "sendinput"
    }
}

impl Drop for SendInputSink {
    fn drop(&mut self) {
        // S2.
        let _ = self.release_all();
    }
}
