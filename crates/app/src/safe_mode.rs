use std::sync::atomic::Ordering;

use iced::widget::{button, container, horizontal_space, row, text};
use iced::{Alignment, Element, Length};

use crate::{App, Message};

const BANNER_HEIGHT: f32 = 34.0;
const SAFE_WORDS: &str = "Opened in safe mode, so no plugins were loaded. Their settings are kept and saved with the song.";
pub const PLUGINS_OFF_PROBLEM: &str = "Plugins are off in safe mode. Turn them back on to open this one.";

impl App {
    pub(crate) fn plugins_are_off(&self) -> bool {
        self.plugins_off.load(Ordering::Relaxed)
    }

    pub(crate) fn set_plugins_off(&mut self, off: bool) {
        if self.plugins_off.swap(off, Ordering::Relaxed) != off {
            self.fx_was = 0;
        }
    }

    pub(crate) fn plugins_back_on(&mut self) {
        self.set_plugins_off(false);
        self.problem = None;
        self.changed();
    }

    pub(crate) fn wants_safe_open(&self) -> bool {
        self.modifiers.shift() || shift_held()
    }

    pub(crate) fn safe_banner(&self) -> Option<Element<'_, Message>> {
        if !self.plugins_are_off() {
            return None;
        }
        let palette = self.palette;
        let line = row![
            text(SAFE_WORDS).size(12).color(palette.text),
            horizontal_space(),
            button(text("Turn plugins back on").size(12).font(palette.medium))
                .padding([3, 12])
                .style(move |_, status| palette.solid(status))
                .on_press(Message::PluginsBackOn),
        ]
        .spacing(12)
        .align_y(Alignment::Center);
        Some(
            container(line)
                .padding([0, 16])
                .width(Length::Fill)
                .height(BANNER_HEIGHT)
                .align_y(Alignment::Center)
                .style(move |_| palette.bar())
                .into(),
        )
    }
}

#[cfg(windows)]
pub fn shift_held() -> bool {
    #[link(name = "user32")]
    extern "system" {
        fn GetAsyncKeyState(key: i32) -> i16;
    }
    const VK_SHIFT: i32 = 0x10;
    unsafe { GetAsyncKeyState(VK_SHIFT) < 0 }
}

#[cfg(target_os = "macos")]
pub fn shift_held() -> bool {
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventSourceFlagsState(state: i32) -> u64;
    }
    const COMBINED_SESSION_STATE: i32 = 0;
    const SHIFT_FLAG: u64 = 0x0002_0000;
    unsafe { CGEventSourceFlagsState(COMBINED_SESSION_STATE) & SHIFT_FLAG != 0 }
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn shift_held() -> bool {
    use x11_dl::{keysym, xlib::Xlib};
    let Ok(xlib) = Xlib::open() else { return false };
    unsafe {
        let display = (xlib.XOpenDisplay)(std::ptr::null());
        if display.is_null() {
            return false;
        }
        let mut keys: [std::os::raw::c_char; 32] = [0; 32];
        (xlib.XQueryKeymap)(display, keys.as_mut_ptr());
        let held = [keysym::XK_Shift_L, keysym::XK_Shift_R].into_iter().any(|symbol| {
            let code = (xlib.XKeysymToKeycode)(display, symbol.into()) as usize;
            code != 0 && (keys[code / 8] as u8) & (1 << (code % 8)) != 0
        });
        (xlib.XCloseDisplay)(display);
        held
    }
}
