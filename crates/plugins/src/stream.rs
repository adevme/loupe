use std::cell::RefCell;

use vst3::Steinberg::{int32, int64, kInvalidArgument, kResultOk, tresult, IBStream, IBStreamTrait, IBStream_::IStreamSeekMode_};
use vst3::{Class, ComWrapper};

pub struct Bytes {
    inner: RefCell<Inner>,
}

struct Inner {
    held: Vec<u8>,
    at: usize,
}

impl Class for Bytes {
    type Interfaces = (IBStream,);
}

impl Bytes {
    pub fn holding(held: Vec<u8>) -> ComWrapper<Self> {
        ComWrapper::new(Self { inner: RefCell::new(Inner { held, at: 0 }) })
    }

    pub fn empty() -> ComWrapper<Self> {
        Self::holding(Vec::new())
    }

    pub fn taken(&self) -> Vec<u8> {
        self.inner.borrow().held.clone()
    }
}

impl IBStreamTrait for Bytes {
    unsafe fn read(&self, buffer: *mut std::ffi::c_void, bytes: int32, read: *mut int32) -> tresult {
        if buffer.is_null() || bytes < 0 {
            return kInvalidArgument;
        }
        let mut inner = self.inner.borrow_mut();
        let left = inner.held.len().saturating_sub(inner.at);
        let take = left.min(bytes as usize);
        if take > 0 {
            std::ptr::copy_nonoverlapping(inner.held[inner.at..].as_ptr(), buffer as *mut u8, take);
            inner.at += take;
        }
        if !read.is_null() {
            *read = take as int32;
        }
        kResultOk
    }

    unsafe fn write(&self, buffer: *mut std::ffi::c_void, bytes: int32, written: *mut int32) -> tresult {
        if buffer.is_null() || bytes < 0 {
            return kInvalidArgument;
        }
        let mut inner = self.inner.borrow_mut();
        let count = bytes as usize;
        let end = inner.at + count;
        if inner.held.len() < end {
            inner.held.resize(end, 0);
        }
        let at = inner.at;
        std::ptr::copy_nonoverlapping(buffer as *const u8, inner.held[at..].as_mut_ptr(), count);
        inner.at = end;
        if !written.is_null() {
            *written = count as int32;
        }
        kResultOk
    }

    unsafe fn seek(&self, pos: int64, mode: int32, result: *mut int64) -> tresult {
        let mut inner = self.inner.borrow_mut();
        let from = if mode == IStreamSeekMode_::kIBSeekSet as int32 {
            0i64
        } else if mode == IStreamSeekMode_::kIBSeekCur as int32 {
            inner.at as i64
        } else if mode == IStreamSeekMode_::kIBSeekEnd as int32 {
            inner.held.len() as i64
        } else {
            return kInvalidArgument;
        };
        let want = from + pos;
        if want < 0 {
            return kInvalidArgument;
        }
        inner.at = want as usize;
        if !result.is_null() {
            *result = inner.at as int64;
        }
        kResultOk
    }

    unsafe fn tell(&self, pos: *mut int64) -> tresult {
        if pos.is_null() {
            return kInvalidArgument;
        }
        *pos = self.inner.borrow().at as int64;
        kResultOk
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vst3::Steinberg::IBStream_::IStreamSeekMode_;

    #[test]
    fn what_goes_in_comes_back_out() {
        let wrapper = Bytes::holding(Vec::new());
        let stream = wrapper.as_com_ref::<IBStream>().unwrap();
        let mut out = *b"hello plugin";
        let mut written = 0;
        unsafe {
            assert_eq!(stream.write(out.as_mut_ptr() as *mut _, out.len() as i32, &mut written), kResultOk);
        }
        assert_eq!(written, out.len() as i32);
        let mut back = 0i64;
        unsafe {
            stream.seek(0, IStreamSeekMode_::kIBSeekSet as i32, &mut back);
        }
        let mut buffer = [0u8; 32];
        let mut read = 0;
        unsafe {
            stream.read(buffer.as_mut_ptr() as *mut _, buffer.len() as i32, &mut read);
        }
        assert_eq!(&buffer[..read as usize], b"hello plugin");
    }

    #[test]
    fn seeking_from_the_end_lands_in_the_right_place() {
        let wrapper = Bytes::holding(b"0123456789".to_vec());
        let stream = wrapper.as_com_ref::<IBStream>().unwrap();
        let mut landed = 0i64;
        unsafe {
            stream.seek(-3, IStreamSeekMode_::kIBSeekEnd as i32, &mut landed);
        }
        assert_eq!(landed, 7);
        let mut buffer = [0u8; 8];
        let mut read = 0;
        unsafe {
            stream.read(buffer.as_mut_ptr() as *mut _, buffer.len() as i32, &mut read);
        }
        assert_eq!(&buffer[..read as usize], b"789");
    }
}
