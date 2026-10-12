use iced::Point;

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
    #[derive(Clone, Copy)]
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

    const WHOLE_WINDOW: u32 = 2;
    const DPI_AT_NO_SCALING: f64 = 96.0;

    #[link(name = "user32")]
    extern "system" {
        fn ScreenToClient(window: *mut core::ffi::c_void, point: *mut Point) -> i32;
        fn WindowFromPoint(point: Point) -> *mut core::ffi::c_void;
        fn GetAncestor(window: *mut core::ffi::c_void, what: u32) -> *mut core::ffi::c_void;
        fn GetWindowThreadProcessId(window: *mut core::ffi::c_void, process: *mut u32) -> u32;
        fn GetDpiForWindow(window: *mut core::ffi::c_void) -> u32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcessId() -> u32;
    }

    fn ours_under(point: Point) -> Option<*mut core::ffi::c_void> {
        let found = unsafe { WindowFromPoint(point) };
        if found.is_null() {
            return None;
        }
        let root = unsafe { GetAncestor(found, WHOLE_WINDOW) };
        let window = if root.is_null() { found } else { root };
        let mut process = 0u32;
        unsafe { GetWindowThreadProcessId(window, &mut process) };
        (process != 0 && process == unsafe { GetCurrentProcessId() }).then_some(window)
    }

    pub fn in_window(interface_scale: f64) -> Option<super::Point> {
        let mut point = position()?;
        let window = ours_under(point)?;
        if unsafe { ScreenToClient(window, &mut point) } == 0 {
            return None;
        }
        let dpi = unsafe { GetDpiForWindow(window) };
        let display_scale = match dpi {
            0 => 1.0,
            found => found as f64 / DPI_AT_NO_SCALING,
        };
        super::in_points(point.x as f32, point.y as f32, display_scale, interface_scale)
    }

    pub fn show(visible: bool) {
        unsafe {
            ShowCursor(visible as i32);
        }
    }
}

pub struct EndlessDrag {
    origin: Point,
    last: Point,
    travel_up: f32,
    anchor: Option<Anchor>,
}

impl EndlessDrag {
    pub fn start(at: Point, hold_pointer: bool) -> Self {
        let anchor = if hold_pointer { Anchor::here() } else { None };
        Self { origin: at, last: at, travel_up: 0.0, anchor }
    }

    pub fn moved(&mut self, to: Point) -> Option<f32> {
        let Some(anchor) = &self.anchor else {
            self.travel_up += self.last.y - to.y;
            self.last = to;
            return Some(self.travel_up);
        };
        if to == self.origin {
            return None;
        }
        if anchor.bring_pointer_back() {
            self.travel_up += self.origin.y - to.y;
        } else {
            self.travel_up += self.last.y - to.y;
            self.last = to;
            self.anchor = None;
        }
        Some(self.travel_up)
    }
}

#[cfg(any(windows, test))]
pub fn in_points(x: f32, y: f32, display_scale: f64, interface_scale: f64) -> Option<Point> {
    let pixels_per_point = display_scale * interface_scale;
    if !pixels_per_point.is_finite() || pixels_per_point <= 0.0 {
        return None;
    }
    Some(Point::new((x as f64 / pixels_per_point) as f32, (y as f64 / pixels_per_point) as f32))
}

#[cfg(windows)]
pub fn where_it_is_now(interface_scale: f64) -> Option<Point> {
    windows::in_window(interface_scale)
}

#[cfg(not(windows))]
pub fn where_it_is_now(_interface_scale: f64) -> Option<Point> {
    None
}

#[cfg(test)]
mod points {
    use super::*;

    #[test]
    fn a_point_on_a_plain_display_comes_back_unchanged() {
        assert_eq!(in_points(240.0, 180.0, 1.0, 1.0), Some(Point::new(240.0, 180.0)));
    }

    #[test]
    fn a_point_on_a_doubled_display_halves() {
        assert_eq!(in_points(240.0, 180.0, 2.0, 1.0), Some(Point::new(120.0, 90.0)));
    }

    #[test]
    fn the_interface_scale_counts_on_top_of_the_display() {
        assert_eq!(in_points(500.0, 250.0, 2.5, 2.0), Some(Point::new(100.0, 50.0)));
    }

    #[test]
    fn a_scale_of_nothing_gives_no_point_rather_than_infinity() {
        assert_eq!(in_points(240.0, 180.0, 0.0, 1.0), None);
        assert_eq!(in_points(240.0, 180.0, 2.0, 0.0), None);
        assert_eq!(in_points(240.0, 180.0, f64::NAN, 1.0), None);
    }
}
