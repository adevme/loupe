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

const LEASH_PX: f32 = 40.0;
const HOME_SLACK_PX: f32 = 1.5;

pub struct EndlessDrag {
    origin: Point,
    last: Point,
    travel_up: f32,
    awaiting_return: bool,
    anchor: Option<Anchor>,
}

impl EndlessDrag {
    pub fn start(at: Point, hold_pointer: bool) -> Self {
        let anchor = if hold_pointer { Anchor::here() } else { None };
        Self { origin: at, last: at, travel_up: 0.0, awaiting_return: false, anchor }
    }

    pub fn moved(&mut self, to: Point) -> Option<f32> {
        if self.awaiting_return && to.distance(self.origin) <= HOME_SLACK_PX {
            self.awaiting_return = false;
            self.last = to;
            return None;
        }
        self.travel_up += self.last.y - to.y;
        self.last = to;
        if to.distance(self.origin) > LEASH_PX && !self.awaiting_return {
            self.awaiting_return = self.anchor.as_ref().is_some_and(Anchor::bring_pointer_back);
        }
        Some(self.travel_up)
    }
}
