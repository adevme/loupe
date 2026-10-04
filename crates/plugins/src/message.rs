use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{CStr, CString};

use vst3::Steinberg::Vst::{IAttributeList, IAttributeListTrait, IMessage, IMessageTrait};
use vst3::Steinberg::{int64, kResultFalse, kResultOk, tchar, tresult, uint32, FIDString};
use vst3::{Class, ComWrapper};

enum Held {
    Whole(int64),
    Fractional(f64),
    Text(Vec<u16>),
    Bytes(Vec<u8>),
}

pub struct Attributes {
    held: RefCell<HashMap<Vec<u8>, Held>>,
}

impl Class for Attributes {
    type Interfaces = (IAttributeList,);
}

fn named(id: vst3::Steinberg::FIDString) -> Option<Vec<u8>> {
    (!id.is_null()).then(|| unsafe { CStr::from_ptr(id) }.to_bytes().to_vec())
}

impl IAttributeListTrait for Attributes {
    unsafe fn setInt(&self, id: vst3::Steinberg::FIDString, value: int64) -> tresult {
        match named(id) {
            Some(key) => {
                self.held.borrow_mut().insert(key, Held::Whole(value));
                kResultOk
            }
            None => kResultFalse,
        }
    }

    unsafe fn getInt(&self, id: vst3::Steinberg::FIDString, value: *mut int64) -> tresult {
        let (Some(key), false) = (named(id), value.is_null()) else { return kResultFalse };
        match self.held.borrow().get(&key) {
            Some(Held::Whole(found)) => {
                unsafe { *value = *found };
                kResultOk
            }
            _ => kResultFalse,
        }
    }

    unsafe fn setFloat(&self, id: vst3::Steinberg::FIDString, value: f64) -> tresult {
        match named(id) {
            Some(key) => {
                self.held.borrow_mut().insert(key, Held::Fractional(value));
                kResultOk
            }
            None => kResultFalse,
        }
    }

    unsafe fn getFloat(&self, id: vst3::Steinberg::FIDString, value: *mut f64) -> tresult {
        let (Some(key), false) = (named(id), value.is_null()) else { return kResultFalse };
        match self.held.borrow().get(&key) {
            Some(Held::Fractional(found)) => {
                unsafe { *value = *found };
                kResultOk
            }
            _ => kResultFalse,
        }
    }

    unsafe fn setString(&self, id: vst3::Steinberg::FIDString, string: *const tchar) -> tresult {
        let (Some(key), false) = (named(id), string.is_null()) else { return kResultFalse };
        let mut units = Vec::new();
        let mut at = 0;
        loop {
            let unit = unsafe { *string.add(at) } as u16;
            if unit == 0 {
                break;
            }
            units.push(unit);
            at += 1;
        }
        self.held.borrow_mut().insert(key, Held::Text(units));
        kResultOk
    }

    unsafe fn getString(&self, id: vst3::Steinberg::FIDString, string: *mut tchar, size_in_bytes: uint32) -> tresult {
        let (Some(key), false) = (named(id), string.is_null()) else { return kResultFalse };
        let room = size_in_bytes as usize / std::mem::size_of::<tchar>();
        let held = self.held.borrow();
        let Some(Held::Text(units)) = held.get(&key) else { return kResultFalse };
        if room == 0 || units.len() + 1 > room {
            return kResultFalse;
        }
        for (at, unit) in units.iter().chain(std::iter::once(&0)).enumerate() {
            unsafe { *string.add(at) = *unit as tchar };
        }
        kResultOk
    }

    unsafe fn setBinary(&self, id: vst3::Steinberg::FIDString, data: *const std::ffi::c_void, size_in_bytes: uint32) -> tresult {
        let (Some(key), false) = (named(id), data.is_null()) else { return kResultFalse };
        let bytes = unsafe { std::slice::from_raw_parts(data as *const u8, size_in_bytes as usize) }.to_vec();
        self.held.borrow_mut().insert(key, Held::Bytes(bytes));
        kResultOk
    }

    unsafe fn getBinary(&self, id: vst3::Steinberg::FIDString, data: *mut *const std::ffi::c_void, size_in_bytes: *mut uint32) -> tresult {
        let (Some(key), false, false) = (named(id), data.is_null(), size_in_bytes.is_null()) else { return kResultFalse };
        let held = self.held.borrow();
        let Some(Held::Bytes(bytes)) = held.get(&key) else { return kResultFalse };
        unsafe {
            *data = bytes.as_ptr() as *const std::ffi::c_void;
            *size_in_bytes = bytes.len() as uint32;
        }
        kResultOk
    }
}

pub struct Note {
    what: RefCell<Option<CString>>,
    attributes: ComWrapper<Attributes>,
}

impl Class for Note {
    type Interfaces = (IMessage,);
}

impl IMessageTrait for Note {
    unsafe fn getMessageID(&self) -> FIDString {
        match self.what.borrow().as_ref() {
            Some(name) => name.as_ptr(),
            None => std::ptr::null(),
        }
    }

    unsafe fn setMessageID(&self, id: FIDString) {
        let name = (!id.is_null()).then(|| unsafe { CStr::from_ptr(id) }.to_owned());
        *self.what.borrow_mut() = name;
    }

    unsafe fn getAttributes(&self) -> *mut IAttributeList {
        self.attributes
            .as_com_ref::<IAttributeList>()
            .map(|found| found.as_ptr())
            .unwrap_or(std::ptr::null_mut())
    }
}

pub fn note() -> ComWrapper<Note> {
    ComWrapper::new(Note { what: RefCell::new(None), attributes: ComWrapper::new(Attributes { held: RefCell::new(HashMap::new()) }) })
}

pub fn attributes() -> ComWrapper<Attributes> {
    ComWrapper::new(Attributes { held: RefCell::new(HashMap::new()) })
}

pub fn same_id(one: *const i8, other: &[u8; 16]) -> bool {
    !one.is_null() && unsafe { std::slice::from_raw_parts(one as *const u8, 16) } == other
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(name: &str) -> CString {
        CString::new(name).unwrap()
    }

    #[test]
    fn a_number_comes_back_as_it_went_in() {
        let list = attributes();
        let held = list.as_com_ref::<IAttributeList>().unwrap();
        let name = key("count");
        unsafe {
            assert_eq!(held.setInt(name.as_ptr(), 42), kResultOk);
            let mut back = 0;
            assert_eq!(held.getInt(name.as_ptr(), &mut back), kResultOk);
            assert_eq!(back, 42);
            let mut nowhere = 0.0;
            assert_eq!(held.getFloat(name.as_ptr(), &mut nowhere), kResultFalse);
        }
    }

    #[test]
    fn what_a_plugin_passes_between_its_halves_survives() {
        let list = attributes();
        let held = list.as_com_ref::<IAttributeList>().unwrap();
        let name = key("JUCE Private Data");
        let given = [1u8, 2, 3, 250, 0, 7];
        unsafe {
            assert_eq!(held.setBinary(name.as_ptr(), given.as_ptr() as *const std::ffi::c_void, given.len() as u32), kResultOk);
            let mut back = std::ptr::null();
            let mut size = 0;
            assert_eq!(held.getBinary(name.as_ptr(), &mut back, &mut size), kResultOk);
            assert_eq!(std::slice::from_raw_parts(back as *const u8, size as usize), &given[..]);
        }
    }

    #[test]
    fn a_message_keeps_its_name_and_hands_out_the_same_attributes() {
        let made = note();
        let message = made.as_com_ref::<IMessage>().unwrap();
        let name = key("hello");
        unsafe {
            assert!(message.getMessageID().is_null());
            message.setMessageID(name.as_ptr());
            assert_eq!(CStr::from_ptr(message.getMessageID()), name.as_c_str());
            let first = message.getAttributes();
            assert!(!first.is_null());
            assert_eq!(first, message.getAttributes());
        }
    }

    #[test]
    fn text_longer_than_the_room_given_is_refused_rather_than_cut() {
        let list = attributes();
        let held = list.as_com_ref::<IAttributeList>().unwrap();
        let name = key("title");
        let said: Vec<tchar> = "Reverb".encode_utf16().map(|unit| unit as tchar).chain(std::iter::once(0)).collect();
        unsafe {
            assert_eq!(held.setString(name.as_ptr(), said.as_ptr()), kResultOk);
            let mut small = [0 as tchar; 3];
            assert_eq!(held.getString(name.as_ptr(), small.as_mut_ptr(), std::mem::size_of_val(&small) as u32), kResultFalse);
            let mut room = [0 as tchar; 16];
            assert_eq!(held.getString(name.as_ptr(), room.as_mut_ptr(), std::mem::size_of_val(&room) as u32), kResultOk);
            let back: Vec<u16> = room.iter().take_while(|unit| **unit != 0).map(|unit| *unit as u16).collect();
            assert_eq!(String::from_utf16_lossy(&back), "Reverb");
        }
    }
}
