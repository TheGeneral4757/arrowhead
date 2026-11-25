//! Windows focus probe: the foreground window's process image against `game_window_classes`
//! (spec 05), plus a "is the game running" check for Setup.

use arrowhead_core::input::{FocusProbe, InputError};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
use windows::core::PWSTR;

use crate::winutil::exe_matches;

pub struct WindowsProbe {
    classes: Vec<String>,
}

impl WindowsProbe {
    pub fn new(classes: Vec<String>) -> Self {
        Self { classes }
    }

    /// Image path of the foreground window's process, `None` if there is none or it cannot
    /// be opened (an elevated game is not, so the guard then reads "not focused": it fails safe).
    pub fn foreground_exe(&self) -> Option<String> {
        // SAFETY: plain Win32 queries; the process handle is closed before returning.
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.0.is_null() {
                return None;
            }
            let mut pid = 0;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            let r = QueryFullProcessImageNameW(
                h,
                PROCESS_NAME_WIN32,
                PWSTR(buf.as_mut_ptr()),
                &mut len,
            );
            let _ = CloseHandle(h);
            r.ok()?;
            Some(String::from_utf16_lossy(&buf[..len as usize]))
        }
    }
}

impl FocusProbe for WindowsProbe {
    fn game_focused(&mut self) -> Result<bool, InputError> {
        Ok(self
            .foreground_exe()
            .is_some_and(|p| exe_matches(&p, &self.classes)))
    }

    fn name(&self) -> &'static str {
        "windows foreground window"
    }
}

/// First running process whose exe name matches `classes`.
pub fn running_game(classes: &[String]) -> Option<String> {
    // SAFETY: the snapshot handle is closed; the entry struct is sized as the API requires.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;
        let mut e = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut found = None;
        let mut ok = Process32FirstW(snap, &mut e).is_ok();
        while ok {
            let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(260);
            let name = String::from_utf16_lossy(&e.szExeFile[..len]);
            if exe_matches(&name, classes) {
                found = Some(name);
                break;
            }
            ok = Process32NextW(snap, &mut e).is_ok();
        }
        let _ = CloseHandle(snap);
        found
    }
}
