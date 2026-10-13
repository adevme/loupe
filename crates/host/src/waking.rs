use std::time::Duration;

#[cfg(windows)]
mod native {
    use std::ffi::c_void;
    use std::time::Duration;

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateEventW(attributes: *mut c_void, manual: i32, signalled: i32, name: *const u16) -> *mut c_void;
        fn SetEvent(event: *mut c_void) -> i32;
        fn CloseHandle(handle: *mut c_void) -> i32;
        fn GetCurrentThread() -> *mut c_void;
        fn SetThreadPriority(thread: *mut c_void, priority: i32) -> i32;
    }

    #[link(name = "user32")]
    extern "system" {
        fn MsgWaitForMultipleObjectsEx(count: u32, handles: *const *mut c_void, millis: u32, wake_on: u32, flags: u32) -> u32;
    }

    const ANY_MESSAGE: u32 = 0x04FF;
    const EVEN_ONES_ALREADY_SEEN: u32 = 0x0004;
    const TIME_CRITICAL: i32 = 15;

    pub struct Bell(usize);

    impl Bell {
        pub fn new() -> Self {
            Self(unsafe { CreateEventW(std::ptr::null_mut(), 0, 0, std::ptr::null()) } as usize)
        }

        pub fn ring(&self) {
            if self.0 != 0 {
                unsafe {
                    SetEvent(self.0 as *mut c_void);
                }
            }
        }

        pub fn wait(&self, longest: Duration) {
            let millis = longest.as_millis().min(u32::MAX as u128) as u32;
            let handle = self.0 as *mut c_void;
            let (count, handles) = if self.0 == 0 { (0, std::ptr::null()) } else { (1, &handle as *const *mut c_void) };
            unsafe {
                MsgWaitForMultipleObjectsEx(count, handles, millis, ANY_MESSAGE, EVEN_ONES_ALREADY_SEEN);
            }
        }
    }

    impl Drop for Bell {
        fn drop(&mut self) {
            if self.0 != 0 {
                unsafe {
                    CloseHandle(self.0 as *mut c_void);
                }
            }
        }
    }

    pub fn hurry_this_thread() {
        unsafe {
            SetThreadPriority(GetCurrentThread(), TIME_CRITICAL);
        }
    }
}

#[cfg(not(windows))]
mod native {
    use std::sync::{Condvar, Mutex};
    use std::time::Duration;

    pub struct Bell {
        rung: Mutex<bool>,
        heard: Condvar,
    }

    impl Bell {
        pub fn new() -> Self {
            Self { rung: Mutex::new(false), heard: Condvar::new() }
        }

        pub fn ring(&self) {
            let mut rung = self.rung.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            *rung = true;
            self.heard.notify_one();
        }

        pub fn wait(&self, longest: Duration) {
            let rung = self.rung.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let (mut rung, _) = self.heard.wait_timeout_while(rung, longest, |rung| !*rung).unwrap_or_else(|poisoned| poisoned.into_inner());
            *rung = false;
        }
    }

    pub fn hurry_this_thread() {}
}

pub use native::hurry_this_thread;

pub struct Bell(native::Bell);

impl Bell {
    pub fn new() -> Self {
        Self(native::Bell::new())
    }

    pub fn ring(&self) {
        self.0.ring();
    }

    pub fn wait(&self, longest: Duration) {
        self.0.wait(longest);
    }
}
