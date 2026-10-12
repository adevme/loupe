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
    pub fn bring_pointer_back(&self) -> Option<f32> {
        let strayed = windows::position()?;
        let risen = windows::rise(&self.screen, &strayed);
        windows::move_to(&self.screen).then_some(risen)
    }

    #[cfg(not(windows))]
    pub fn bring_pointer_back(&self) -> Option<f32> {
        None
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

    pub fn rise(from: &Point, to: &Point) -> f32 {
        (from.y - to.y) as f32
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

    fn scale_of(window: *mut core::ffi::c_void) -> f64 {
        match unsafe { GetDpiForWindow(window) } {
            0 => 1.0,
            found => found as f64 / DPI_AT_NO_SCALING,
        }
    }

    pub fn display_scale() -> Option<f64> {
        let point = position()?;
        Some(scale_of(ours_under(point)?))
    }

    pub fn in_window(interface_scale: f64) -> Option<super::Point> {
        let mut point = position()?;
        let window = ours_under(point)?;
        if unsafe { ScreenToClient(window, &mut point) } == 0 {
            return None;
        }
        let display_scale = scale_of(window);
        super::in_points(point.x as f32, point.y as f32, display_scale, interface_scale)
    }

    pub fn show(visible: bool) {
        unsafe {
            ShowCursor(visible as i32);
        }
    }
}

pub struct EndlessDrag {
    last: Point,
    travel_up: f32,
    pixels_per_point: f32,
    anchor: Option<Anchor>,
}

impl EndlessDrag {
    pub fn start(at: Point, hold_pointer: bool, pixels_per_point: f32) -> Self {
        let anchor = if hold_pointer { Anchor::here() } else { None };
        let sane = pixels_per_point.is_finite() && pixels_per_point > 0.0;
        Self { last: at, travel_up: 0.0, pixels_per_point: if sane { pixels_per_point } else { 1.0 }, anchor }
    }

    pub fn moved(&mut self, to: Point, pace: f32) -> Option<f32> {
        let risen = match &self.anchor {
            Some(anchor) => match anchor.bring_pointer_back() {
                Some(risen) => risen,
                None => {
                    self.anchor = None;
                    self.last = to;
                    return None;
                }
            },
            None => self.rise_to(to),
        };
        self.add(risen, pace)
    }

    fn rise_to(&mut self, to: Point) -> f32 {
        let risen = (self.last.y - to.y) * self.pixels_per_point;
        self.last = to;
        risen
    }

    fn add(&mut self, risen: f32, pace: f32) -> Option<f32> {
        if risen == 0.0 {
            return None;
        }
        self.travel_up += risen * pace;
        Some(self.travel_up)
    }
}

#[cfg(windows)]
pub fn pixels_per_point(interface_scale: f64) -> f32 {
    (windows::display_scale().unwrap_or(1.0) * interface_scale) as f32
}

#[cfg(not(windows))]
pub fn pixels_per_point(interface_scale: f64) -> f32 {
    interface_scale as f32
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

#[cfg(test)]
mod dragging {
    use super::*;

    #[test]
    fn the_same_hand_movement_counts_the_same_at_every_scale() {
        let mut travelled = vec![];
        for scale in [1.0, 2.0, 2.5] {
            let mut pull = EndlessDrag::start(Point::new(10.0, 100.0 / scale), false, scale);
            travelled.push(pull.moved(Point::new(10.0, 60.0 / scale), 1.0));
        }
        assert_eq!(travelled, vec![Some(40.0), Some(40.0), Some(40.0)]);
    }

    #[test]
    fn a_scale_of_nothing_leaves_the_drag_counting_plain_pixels() {
        let mut pull = EndlessDrag::start(Point::new(0.0, 50.0), false, 0.0);
        assert_eq!(pull.moved(Point::new(0.0, 30.0), 1.0), Some(20.0));
    }

    #[test]
    fn a_still_pointer_moves_nothing() {
        let mut pull = EndlessDrag::start(Point::new(4.0, 9.0), false, 2.0);
        assert_eq!(pull.moved(Point::new(4.0, 9.0), 1.0), None);
        assert_eq!(pull.moved(Point::new(9.0, 9.0), 1.0), None);
    }

    #[test]
    fn a_slow_pace_counts_less_of_each_step() {
        let mut pull = EndlessDrag::start(Point::new(0.0, 200.0), false, 1.0);
        assert_eq!(pull.moved(Point::new(0.0, 150.0), 0.2), Some(10.0));
        assert_eq!(pull.moved(Point::new(0.0, 100.0), 1.0), Some(60.0));
    }

    #[test]
    fn the_drag_adds_up_step_by_step_and_turns_round_at_once() {
        let mut pull = EndlessDrag::start(Point::new(0.0, 100.0), false, 1.0);
        assert_eq!(pull.moved(Point::new(0.0, 90.0), 1.0), Some(10.0));
        assert_eq!(pull.moved(Point::new(0.0, 70.0), 1.0), Some(30.0));
        assert_eq!(pull.moved(Point::new(0.0, 75.0), 1.0), Some(25.0));
        assert_eq!(pull.moved(Point::new(0.0, 100.0), 1.0), Some(0.0));
    }

    #[test]
    fn a_pointer_brought_back_counts_once_and_only_once() {
        let mut pull = EndlessDrag::start(Point::new(0.0, 100.0), false, 1.0);
        assert_eq!(pull.add(20.0, 1.0), Some(20.0));
        assert_eq!(pull.add(0.0, 1.0), None);
        assert_eq!(pull.add(0.0, 1.0), None);
        assert_eq!(pull.add(20.0, 1.0), Some(40.0));
    }
}
