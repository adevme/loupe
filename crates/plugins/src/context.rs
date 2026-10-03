use vst3::Steinberg::Vst::{IHostApplication, IHostApplicationTrait, String128};
use vst3::Steinberg::{kNotImplemented, kResultOk, tresult, TUID};
use vst3::{Class, ComWrapper};

pub struct Us;

impl Class for Us {
    type Interfaces = (IHostApplication,);
}

impl IHostApplicationTrait for Us {
    unsafe fn getName(&self, name: *mut String128) -> tresult {
        if name.is_null() {
            return kNotImplemented;
        }
        let said: Vec<u16> = "Loupe".encode_utf16().collect();
        let out = &mut *name;
        for (slot, unit) in out.iter_mut().zip(said.iter().chain(std::iter::once(&0))) {
            *slot = *unit;
        }
        kResultOk
    }

    unsafe fn createInstance(&self, _cid: *mut TUID, _iid: *mut TUID, obj: *mut *mut std::ffi::c_void) -> tresult {
        if !obj.is_null() {
            *obj = std::ptr::null_mut();
        }
        kNotImplemented
    }
}

pub fn ours() -> ComWrapper<Us> {
    ComWrapper::new(Us)
}
