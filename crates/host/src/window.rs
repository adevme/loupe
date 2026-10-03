#[cfg(windows)]
mod real {
    use std::ffi::c_void;

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
        fn LoadCursorW(instance: Handle, name: *const u16) -> Handle;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleW(name: *const u16) -> Handle;
    }

    const OVERLAPPED_WINDOW: u32 = 0x00CF_0000;
    const SHOW: i32 = 5;
    const REMOVE: u32 = 1;
    const CLOSE: u32 = 0x0010;
    const ARROW: usize = 32512;

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    unsafe extern "system" fn handle(window: Handle, what: u32, first: usize, second: isize) -> isize {
        if what == CLOSE {
            ShowWindow(window, 0);
            return 0;
        }
        DefWindowProcW(window, what, first, second)
    }

    pub struct Window {
        handle: Handle,
    }

    impl Window {
        pub fn open(title: &str, width: i32, height: i32) -> Result<Self, String> {
            let name = wide("LoupePluginWindow");
            unsafe {
                let instance = GetModuleHandleW(std::ptr::null());
                let class = WindowClass {
                    size: std::mem::size_of::<WindowClass>() as u32,
                    style: 0,
                    procedure: Some(handle),
                    class_extra: 0,
                    window_extra: 0,
                    instance,
                    icon: std::ptr::null_mut(),
                    cursor: LoadCursorW(std::ptr::null_mut(), ARROW as *const u16),
                    background: std::ptr::null_mut(),
                    menu: std::ptr::null(),
                    name: name.as_ptr(),
                    small_icon: std::ptr::null_mut(),
                };
                RegisterClassExW(&class);
                let mut rect = [0, 0, width, height];
                AdjustWindowRectEx(&mut rect, OVERLAPPED_WINDOW, 0, 0);
                let title = wide(title);
                let handle = CreateWindowExW(
                    0,
                    name.as_ptr(),
                    title.as_ptr(),
                    OVERLAPPED_WINDOW,
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
                Ok(Self { handle })
            }
        }

        pub fn inner(&self) -> *mut c_void {
            self.handle
        }

        pub fn show(&self) {
            unsafe {
                ShowWindow(self.handle, SHOW);
            }
        }

        pub fn hide(&self) {
            unsafe {
                ShowWindow(self.handle, 0);
            }
        }

        pub fn pump(&self) {
            unsafe {
                let mut message: Message = std::mem::zeroed();
                while PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, REMOVE) != 0 {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
        }
    }

    impl Drop for Window {
        fn drop(&mut self) {
            unsafe {
                DestroyWindow(self.handle);
            }
        }
    }
}

#[cfg(not(windows))]
mod real {
    use std::ffi::c_void;

    pub struct Window;

    impl Window {
        pub fn open(_title: &str, _width: i32, _height: i32) -> Result<Self, String> {
            Err("plugin windows only open on Windows so far".into())
        }

        pub fn inner(&self) -> *mut c_void {
            std::ptr::null_mut()
        }

        pub fn show(&self) {}

        pub fn hide(&self) {}

        pub fn pump(&self) {}
    }
}

pub use real::Window;
