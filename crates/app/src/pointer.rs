pub struct Anchor {
    #[cfg(windows)]
    screen: windows::Point,
}

impl Anchor {
    #[cfg(windows)]
    pub fn here() -> Option<Self> {
        let screen = windows::position()?;
        windows::show(false);
        Some(Self { screen })
    }

    #[cfg(not(windows))]
    pub fn here() -> Option<Self> {
        None
    }

    #[cfg(windows)]
    pub fn bring_pointer_back(&self) -> bool {
        windows::move_to(&self.screen)
    }

    #[cfg(not(windows))]
    pub fn bring_pointer_back(&self) -> bool {
        false
    }
}

#[cfg(windows)]
impl Drop for Anchor {
    fn drop(&mut self) {
        windows::move_to(&self.screen);
        windows::show(true);
    }
}

#[cfg(windows)]
mod windows {
    #[repr(C)]
    pub struct Point {
        x: i32,
        y: i32,
    }

    #[link(name = "user32")]
    extern "system" {
        fn GetCursorPos(point: *mut Point) -> i32;
        fn SetCursorPos(x: i32, y: i32) -> i32;
        fn ShowCursor(show: i32) -> i32;
    }

    pub fn position() -> Option<Point> {
        let mut point = Point { x: 0, y: 0 };
        let found = unsafe { GetCursorPos(&mut point) } != 0;
        found.then_some(point)
    }

    pub fn move_to(point: &Point) -> bool {
        unsafe { SetCursorPos(point.x, point.y) != 0 }
    }

    pub fn show(visible: bool) {
        unsafe {
            ShowCursor(visible as i32);
        }
    }
}
