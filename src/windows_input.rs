//! Passive sampling exports activity totals only; key identities stay ephemeral.
use std::time::Duration;

fn desktop_name_is_available(name: &str) -> bool {
    !name.is_empty()
        && !name.eq_ignore_ascii_case("Winlogon")
        && !name.eq_ignore_ascii_case("Disconnect")
}

#[cfg(windows)]
struct InputDesktop(windows::Win32::System::StationsAndDesktops::HDESK);

#[cfg(windows)]
impl InputDesktop {
    fn is_available(&self) -> bool {
        use windows::Win32::{
            Foundation::HANDLE,
            System::StationsAndDesktops::{GetUserObjectInformationW, UOI_NAME},
        };
        let mut name = [0u16; 512];
        // The guard owns this valid desktop handle. The UTF-16 buffer is
        // initialized and writable for the stated byte count throughout the
        // synchronous call. A missing/oversized/unreadable name pauses safely.
        let result = unsafe {
            GetUserObjectInformationW(
                HANDLE(self.0.0),
                UOI_NAME,
                Some(name.as_mut_ptr().cast()),
                std::mem::size_of_val(&name) as u32,
                None,
            )
        };
        if result.is_err() {
            return false;
        }
        let Some(end) = name.iter().position(|unit| *unit == 0) else {
            return false;
        };
        String::from_utf16(&name[..end]).is_ok_and(|name| desktop_name_is_available(&name))
    }
}

#[cfg(windows)]
impl Drop for InputDesktop {
    fn drop(&mut self) {
        // This guard is constructed only from OpenInputDesktop success and
        // owns the handle exclusively; every query outcome closes it once.
        let _ = unsafe { windows::Win32::System::StationsAndDesktops::CloseDesktop(self.0) };
    }
}

#[derive(Clone, Copy, Debug)]
pub struct InputSample {
    pub activity_pulses: u64,
    pub mouse: Option<(i32, i32)>,
    pub desktop_available: bool,
}

pub struct Sampler {
    pressed: [bool; 256],
    desktop_available: bool,
    last_desktop_check: Option<Duration>,
}

impl Default for Sampler {
    fn default() -> Self {
        Self {
            pressed: [false; 256],
            desktop_available: false,
            last_desktop_check: None,
        }
    }
}

#[cfg(windows)]
impl Sampler {
    pub fn sample(&mut self, now: Duration) -> InputSample {
        use windows::Win32::{
            Foundation::POINT,
            System::StationsAndDesktops::{
                DESKTOP_CONTROL_FLAGS, DESKTOP_READOBJECTS, OpenInputDesktop,
            },
            UI::{Input::KeyboardAndMouse::GetAsyncKeyState, WindowsAndMessaging::GetCursorPos},
        };
        if self
            .last_desktop_check
            .is_none_or(|last| now.saturating_sub(last) >= Duration::from_secs(1))
        {
            self.last_desktop_check = Some(now);
            // The handle is opened with read-only access, never made active,
            // and closed in this scope. Failure includes locked/secure desktops.
            self.desktop_available = unsafe {
                match OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, DESKTOP_READOBJECTS) {
                    Ok(desktop) => InputDesktop(desktop).is_available(),
                    Err(_) => false,
                }
            };
        }
        if !self.desktop_available {
            self.pressed.fill(false);
            return InputSample {
                activity_pulses: 0,
                mouse: None,
                desktop_available: false,
            };
        }
        let mut pulses = 0u64;
        for (key, previous) in self.pressed.iter_mut().enumerate() {
            // Win32 accepts a virtual key in 0..=255 and returns a value only;
            // no hooks, callbacks or externally owned memory are involved.
            let pressed = unsafe { GetAsyncKeyState(key as i32) } < 0;
            if pressed && !*previous {
                pulses += 1;
            }
            *previous = pressed;
        }
        let mut point = POINT::default();
        // POINT is initialized and its writable pointer remains valid until
        // this synchronous OS call returns.
        let mouse = unsafe { GetCursorPos(&mut point) }
            .ok()
            .map(|()| (point.x, point.y));
        InputSample {
            activity_pulses: pulses,
            mouse,
            desktop_available: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::desktop_name_is_available;

    #[test]
    fn secure_desktops_are_unavailable_even_when_open_succeeds() {
        for name in [
            "Winlogon",
            "WINLOGON",
            "winlogon",
            "Disconnect",
            "DISCONNECT",
            "",
        ] {
            assert!(
                !desktop_name_is_available(name),
                "secure or unknown name must pause"
            );
        }
    }

    #[test]
    fn ordinary_and_custom_desktops_remain_available() {
        for name in ["Default", "default", "MyWorkspace", "专注桌面"] {
            assert!(desktop_name_is_available(name));
        }
    }
}

#[cfg(not(windows))]
impl Sampler {
    pub fn sample(&mut self, _now: Duration) -> InputSample {
        InputSample {
            activity_pulses: 0,
            mouse: None,
            desktop_available: false,
        }
    }
}
