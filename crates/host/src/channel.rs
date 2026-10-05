use std::fs::File;
use std::io::Write;

#[cfg(unix)]
mod names {
    extern "C" {
        pub fn dup(old: i32) -> i32;
        pub fn dup2(old: i32, new: i32) -> i32;
        pub fn open(path: *const i8, flags: i32, ...) -> i32;
    }
    pub const NOWHERE: &[u8] = b"/dev/null\0";
    pub const WRITE_ONLY: i32 = 1;
}

#[cfg(windows)]
mod names {
    extern "C" {
        #[link_name = "_dup"]
        pub fn dup(old: i32) -> i32;
        #[link_name = "_dup2"]
        pub fn dup2(old: i32, new: i32) -> i32;
        #[link_name = "_open"]
        pub fn open(path: *const i8, flags: i32, ...) -> i32;
    }
    pub const NOWHERE: &[u8] = b"NUL\0";
    pub const WRITE_ONLY: i32 = 1;
}

pub fn take_stdout() -> (Box<dyn Write + Send>, i32) {
    unsafe {
        let mine = names::dup(1);
        let nowhere = names::open(names::NOWHERE.as_ptr() as *const i8, names::WRITE_ONLY);
        if nowhere >= 0 {
            names::dup2(nowhere, 1);
        }
        if mine < 0 {
            return (Box::new(std::io::stdout()), -1);
        }
        (Box::new(from_fd(mine)), mine)
    }
}

#[cfg(unix)]
unsafe fn from_fd(fd: i32) -> File {
    use std::os::fd::FromRawFd;
    File::from_raw_fd(fd)
}

#[cfg(windows)]
unsafe fn from_fd(fd: i32) -> File {
    use std::os::windows::io::FromRawHandle;
    extern "C" {
        #[link_name = "_get_osfhandle"]
        fn get_handle(fd: i32) -> isize;
    }
    File::from_raw_handle(get_handle(fd) as *mut core::ffi::c_void)
}
