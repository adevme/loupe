use vst3::Steinberg::Vst::{IAttributeList, IAttributeList_iid, IHostApplication, IHostApplicationTrait, IMessage, IMessage_iid, String128};
use vst3::Steinberg::{kNotImplemented, kResultOk, tresult, TUID};
use vst3::{Class, ComWrapper};

use crate::message;

pub struct Us;

impl Class for Us {
    type Interfaces = (IHostApplication,);
}

fn asked_for(iid: *mut TUID, wanted: &TUID) -> bool {
    !iid.is_null() && unsafe { *iid } == *wanted
}

impl IHostApplicationTrait for Us {
    unsafe fn getName(&self, name: *mut String128) -> tresult {
        if name.is_null() {
            return kNotImplemented;
        }
        let said: Vec<u16> = "Loupe".encode_utf16().collect();
        let out = unsafe { &mut *name };
        for (slot, unit) in out.iter_mut().zip(said.iter().chain(std::iter::once(&0))) {
            *slot = *unit;
        }
        kResultOk
    }

    unsafe fn createInstance(&self, _cid: *mut TUID, iid: *mut TUID, obj: *mut *mut std::ffi::c_void) -> tresult {
        { use std::io::Write; let _ = std::fs::OpenOptions::new().create(true).append(true).open("C:\\Users\\ash\\probe\\step.txt").map(|mut f| writeln!(f, "STEP createInstance")); }
        if obj.is_null() {
            return kNotImplemented;
        }
        unsafe { *obj = std::ptr::null_mut() };
        if asked_for(iid, &IMessage_iid) {
            let made = message::note();
            if let Some(found) = made.to_com_ptr::<IMessage>() {
                unsafe { *obj = found.into_raw() as *mut std::ffi::c_void };
                return kResultOk;
            }
        }
        if asked_for(iid, &IAttributeList_iid) {
            let made = message::attributes();
            if let Some(found) = made.to_com_ptr::<IAttributeList>() {
                unsafe { *obj = found.into_raw() as *mut std::ffi::c_void };
                return kResultOk;
            }
        }
        kNotImplemented
    }
}

pub fn ours() -> ComWrapper<Us> {
    ComWrapper::new(Us)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vst3::Steinberg::Vst::IMessageTrait;
    use vst3::ComPtr;

    fn made(iid: TUID) -> *mut std::ffi::c_void {
        let us = ours();
        let host = us.as_com_ref::<IHostApplication>().unwrap();
        let mut out = std::ptr::null_mut();
        let mut wanted = iid;
        let mut cid = iid;
        unsafe { host.createInstance(&mut cid, &mut wanted, &mut out) };
        out
    }

    #[test]
    fn a_plugin_asking_for_a_message_is_given_one() {
        let out = made(IMessage_iid);
        assert!(!out.is_null());
        let message: ComPtr<IMessage> = unsafe { ComPtr::from_raw(out as *mut IMessage) }.unwrap();
        unsafe { assert!(!message.getAttributes().is_null()) };
    }

    #[test]
    fn a_plugin_asking_for_an_attribute_list_is_given_one() {
        let out = made(IAttributeList_iid);
        assert!(!out.is_null());
        let _: ComPtr<IAttributeList> = unsafe { ComPtr::from_raw(out as *mut IAttributeList) }.unwrap();
    }

    #[test]
    fn anything_else_is_turned_away_with_nothing_left_behind() {
        let out = made(vst3::Steinberg::FUnknown_iid);
        assert!(out.is_null());
    }
}
