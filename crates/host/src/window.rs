#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum Asked {
    Load(String),
    Save(String),
}

#[cfg(windows)]
mod real {
    use std::cell::{Cell, RefCell};
    use std::ffi::c_void;

    use super::Asked;

    type Handle = *mut c_void;

    #[repr(C)]
    struct Message {
        window: Handle,
        what: u32,
        first: usize,
        second: isize,
        time: u32,
        x: i32,
        y: i32,
        private: u32,
    }

    #[repr(C)]
    struct WindowClass {
        size: u32,
        style: u32,
        procedure: Option<unsafe extern "system" fn(Handle, u32, usize, isize) -> isize>,
        class_extra: i32,
        window_extra: i32,
        instance: Handle,
        icon: Handle,
        cursor: Handle,
        background: Handle,
        menu: *const u16,
        name: *const u16,
        small_icon: Handle,
    }

    #[link(name = "user32")]
    extern "system" {
        fn RegisterClassExW(class: *const WindowClass) -> u16;
        fn CreateWindowExW(
            extra: u32,
            class: *const u16,
            title: *const u16,
            style: u32,
            x: i32,
            y: i32,
            width: i32,
            height: i32,
            parent: Handle,
            menu: Handle,
            instance: Handle,
            param: *mut c_void,
        ) -> Handle;
        fn DestroyWindow(window: Handle) -> i32;
        fn ShowWindow(window: Handle, how: i32) -> i32;
        fn DefWindowProcW(window: Handle, what: u32, first: usize, second: isize) -> isize;
        fn PeekMessageW(message: *mut Message, window: Handle, low: u32, high: u32, remove: u32) -> i32;
        fn TranslateMessage(message: *const Message) -> i32;
        fn DispatchMessageW(message: *const Message) -> isize;
        fn AdjustWindowRectEx(rect: *mut [i32; 4], style: u32, menu: i32, extra: u32) -> i32;
        fn GetClientRect(window: *mut c_void, rect: *mut [i32; 4]) -> i32;
        fn LoadCursorW(instance: Handle, name: *const u16) -> Handle;
        fn MoveWindow(window: Handle, x: i32, y: i32, width: i32, height: i32, repaint: i32) -> i32;
        fn SetWindowPos(window: Handle, after: Handle, x: i32, y: i32, width: i32, height: i32, how: u32) -> i32;
        fn SetForegroundWindow(window: Handle) -> i32;
        fn SendMessageW(window: Handle, what: u32, first: usize, second: isize) -> isize;
        fn GetWindowTextW(window: Handle, text: *mut u16, most: i32) -> i32;
        fn GetWindowTextLengthW(window: Handle) -> i32;
        fn SetWindowTextW(window: Handle, text: *const u16) -> i32;
        fn GetSysColorBrush(index: i32) -> Handle;
    }

    #[link(name = "gdi32")]
    extern "system" {
        fn GetStockObject(which: i32) -> Handle;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleW(name: *const u16) -> Handle;
    }

    const OVERLAPPED_WINDOW: u32 = 0x00CF_0000;
    /// The same window without the drag edges or the maximise button, for a plugin
    /// that only draws at one size.
    const FIXED_WINDOW: u32 = 0x00CA_0000;
    const CLIP_CHILDREN: u32 = 0x0200_0000;
    const CHILD: u32 = 0x4000_0000;
    const VISIBLE: u32 = 0x1000_0000;
    const TAB_STOP: u32 = 0x0001_0000;
    const VERTICAL_SCROLL: u32 = 0x0020_0000;
    const BORDERED: u32 = 0x0080_0000;
    const DROP_DOWN_LIST: u32 = 0x0003;
    const SCROLLS_SIDEWAYS: u32 = 0x0080;
    const SHOW: i32 = 5;
    const TOP: Handle = std::ptr::null_mut();
    const KEEP_SIZE: u32 = 0x0001;
    const KEEP_PLACE: u32 = 0x0002;
    const AND_SHOW: u32 = 0x0040;
    const REMOVE: u32 = 1;
    const CLOSE: u32 = 0x0010;
    const SIZE: u32 = 0x0005;
    const COMMAND: u32 = 0x0111;
    const SET_FONT: u32 = 0x0030;
    const ADD_STRING: u32 = 0x0143;
    const RESET_CONTENT: u32 = 0x014B;
    const CURRENT: u32 = 0x0147;
    const ITEM_TEXT: u32 = 0x0148;
    const ITEM_LENGTH: u32 = 0x0149;
    const SET_CURRENT: u32 = 0x014E;
    const SELECTION_CHANGED: usize = 1;
    const CLICKED: usize = 0;
    const ARROW: usize = 32512;
    const GUI_FONT: i32 = 17;
    const FACE_COLOUR: i32 = 15;
    const BAR: i32 = 34;
    const GAP: i32 = 6;
    const LIST_WIDTH: i32 = 220;
    const NAME_WIDTH: i32 = 160;
    const SAVE_WIDTH: i32 = 90;
    const ROW: i32 = 22;
    const LIST_DROP: i32 = 300;
    const LIST_ID: usize = 101;
    const NAME_ID: usize = 102;
    const SAVE_ID: usize = 103;
    const PICK: &str = "Presets";

    #[derive(Clone, Copy)]
    struct Parts {
        area: Handle,
        list: Handle,
        name: Handle,
    }

    thread_local! {
        static PARTS: Cell<Option<Parts>> = const { Cell::new(None) };
        static ASKED: RefCell<Vec<Asked>> = const { RefCell::new(Vec::new()) };
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    unsafe fn text_of(window: Handle) -> String {
        let len = GetWindowTextLengthW(window).max(0) as usize;
        let mut text = vec![0u16; len + 1];
        let got = GetWindowTextW(window, text.as_mut_ptr(), text.len() as i32).max(0) as usize;
        String::from_utf16_lossy(&text[..got])
    }

    unsafe fn chosen_in(list: Handle) -> Option<String> {
        // The list answers -1 for nothing chosen. Nought is the first preset, not nothing.
        let at = SendMessageW(list, CURRENT, 0, 0);
        if at < 0 {
            return None;
        }
        let len = SendMessageW(list, ITEM_LENGTH, at as usize, 0).max(0) as usize;
        let mut text = vec![0u16; len + 1];
        SendMessageW(list, ITEM_TEXT, at as usize, text.as_mut_ptr() as isize);
        Some(String::from_utf16_lossy(&text[..len]))
    }

    unsafe extern "system" fn handle(window: Handle, what: u32, first: usize, second: isize) -> isize {
        match what {
            CLOSE => {
                ShowWindow(window, 0);
                return 0;
            }
            SIZE => {
                if let Some(parts) = PARTS.with(Cell::get) {
                    let width = (second & 0xFFFF) as i32;
                    let height = ((second >> 16) & 0xFFFF) as i32;
                    MoveWindow(parts.area, 0, BAR, width, (height - BAR).max(0), 1);
                }
            }
            COMMAND => {
                if let Some(parts) = PARTS.with(Cell::get) {
                    let (id, why) = (first & 0xFFFF, (first >> 16) & 0xFFFF);
                    let asked = match (id, why) {
                        (LIST_ID, SELECTION_CHANGED) => chosen_in(parts.list).map(Asked::Load),
                        (SAVE_ID, CLICKED) => Some(Asked::Save(text_of(parts.name))),
                        _ => None,
                    };
                    if let Some(asked) = asked {
                        ASKED.with(|held| held.borrow_mut().push(asked));
                    }
                }
            }
            _ => {}
        }
        DefWindowProcW(window, what, first, second)
    }

    unsafe extern "system" fn plain(window: Handle, what: u32, first: usize, second: isize) -> isize {
        DefWindowProcW(window, what, first, second)
    }

    unsafe fn register(name: &[u16], procedure: unsafe extern "system" fn(Handle, u32, usize, isize) -> isize, instance: Handle) {
        let class = WindowClass {
            size: std::mem::size_of::<WindowClass>() as u32,
            style: 0,
            procedure: Some(procedure),
            class_extra: 0,
            window_extra: 0,
            instance,
            icon: std::ptr::null_mut(),
            cursor: LoadCursorW(std::ptr::null_mut(), ARROW as *const u16),
            background: GetSysColorBrush(FACE_COLOUR),
            menu: std::ptr::null(),
            name: name.as_ptr(),
            small_icon: std::ptr::null_mut(),
        };
        RegisterClassExW(&class);
    }

    unsafe fn control(class: &str, text: &str, style: u32, at: [i32; 4], parent: Handle, id: usize, instance: Handle) -> Handle {
        let (class, text) = (wide(class), wide(text));
        let made = CreateWindowExW(0, class.as_ptr(), text.as_ptr(), CHILD | VISIBLE | style, at[0], at[1], at[2], at[3], parent, id as Handle, instance, std::ptr::null_mut());
        SendMessageW(made, SET_FONT, GetStockObject(GUI_FONT) as usize, 1);
        made
    }

    pub struct Window {
        handle: Handle,
        parts: Parts,
    }

    impl Window {
        pub fn open(title: &str, width: i32, height: i32, resizable: bool) -> Result<Self, String> {
            let frame = if resizable { OVERLAPPED_WINDOW } else { FIXED_WINDOW } | CLIP_CHILDREN;
            let name = wide("LoupePluginWindow");
            let area_name = wide("LoupePluginArea");
            unsafe {
                let instance = GetModuleHandleW(std::ptr::null());
                register(&name, handle, instance);
                register(&area_name, plain, instance);
                let mut rect = [0, 0, width, height + BAR];
                AdjustWindowRectEx(&mut rect, frame, 0, 0);
                let title = wide(title);
                let handle = CreateWindowExW(
                    0,
                    name.as_ptr(),
                    title.as_ptr(),
                    frame,
                    i32::MIN,
                    i32::MIN,
                    rect[2] - rect[0],
                    rect[3] - rect[1],
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    instance,
                    std::ptr::null_mut(),
                );
                if handle.is_null() {
                    return Err("Loupe could not open a window for the plugin".into());
                }
                let top = (BAR - ROW) / 2;
                let list = control("COMBOBOX", "", DROP_DOWN_LIST | VERTICAL_SCROLL | TAB_STOP, [GAP, top, LIST_WIDTH, LIST_DROP], handle, LIST_ID, instance);
                let name_at = GAP * 3 + LIST_WIDTH;
                let name_box = control("EDIT", "", BORDERED | SCROLLS_SIDEWAYS | TAB_STOP, [name_at, top, NAME_WIDTH, ROW], handle, NAME_ID, instance);
                control("BUTTON", "Save preset", TAB_STOP, [name_at + NAME_WIDTH + GAP, top, SAVE_WIDTH, ROW], handle, SAVE_ID, instance);
                let area = CreateWindowExW(0, area_name.as_ptr(), std::ptr::null(), CHILD | VISIBLE | CLIP_CHILDREN, 0, BAR, width, height, handle, std::ptr::null_mut(), instance, std::ptr::null_mut());
                if area.is_null() {
                    DestroyWindow(handle);
                    return Err("Loupe could not open a window for the plugin".into());
                }
                let parts = Parts { area, list, name: name_box };
                PARTS.with(|held| held.set(Some(parts)));
                Ok(Self { handle, parts })
            }
        }

        pub fn inner(&self) -> *mut c_void {
            self.parts.area
        }

        pub fn show(&self) {
            unsafe {
                ShowWindow(self.handle, SHOW);
                // The plugin window belongs to the host, which is not the program the
                // user is clicking in. Windows leaves another program's window where it
                // was in the stack, which with Loupe filling the screen means behind it.
                SetWindowPos(self.handle, TOP, 0, 0, 0, 0, KEEP_PLACE | KEEP_SIZE | AND_SHOW);
                SetForegroundWindow(self.handle);
            }
        }

        pub fn hide(&self) {
            unsafe {
                ShowWindow(self.handle, 0);
            }
        }

        /// The size of the area the plugin draws in, which changes as the user drags
        /// the frame.
        pub fn inside(&self) -> (i32, i32) {
            unsafe {
                let mut rect = [0i32; 4];
                if GetClientRect(self.parts.area, &mut rect) == 0 {
                    return (0, 0);
                }
                (rect[2] - rect[0], rect[3] - rect[1])
            }
        }

        pub fn presets(&self, names: &[String], chosen: Option<&str>) {
            unsafe {
                SendMessageW(self.parts.list, RESET_CONTENT, 0, 0);
                for name in std::iter::once(PICK).chain(names.iter().map(String::as_str)) {
                    let text = wide(name);
                    SendMessageW(self.parts.list, ADD_STRING, 0, text.as_ptr() as isize);
                }
                let at = chosen.and_then(|wanted| names.iter().position(|name| name == wanted)).map_or(0, |at| at + 1);
                SendMessageW(self.parts.list, SET_CURRENT, at, 0);
            }
        }

        pub fn clear_name(&self) {
            unsafe {
                let empty = wide("");
                SetWindowTextW(self.parts.name, empty.as_ptr());
            }
        }

        pub fn pump(&self) -> Vec<Asked> {
            unsafe {
                let mut message: Message = std::mem::zeroed();
                while PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, REMOVE) != 0 {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            ASKED.with(|held| std::mem::take(&mut *held.borrow_mut()))
        }
    }

    impl Drop for Window {
        fn drop(&mut self) {
            PARTS.with(|held| held.set(None));
            unsafe {
                DestroyWindow(self.handle);
            }
        }
    }
}

#[cfg(not(windows))]
mod real {
    use std::ffi::c_void;

    use super::Asked;

    pub struct Window;

    impl Window {
        pub fn open(_title: &str, _width: i32, _height: i32, _resizable: bool) -> Result<Self, String> {
            Err("plugin windows only open on Windows so far".into())
        }

        pub fn inner(&self) -> *mut c_void {
            std::ptr::null_mut()
        }

        pub fn show(&self) {}

        pub fn hide(&self) {}

        pub fn inside(&self) -> (i32, i32) {
            (0, 0)
        }

        pub fn presets(&self, _names: &[String], _chosen: Option<&str>) {}

        pub fn clear_name(&self) {}

        pub fn pump(&self) -> Vec<Asked> {
            Vec::new()
        }
    }
}

pub use real::Window;
