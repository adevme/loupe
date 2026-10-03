use std::cell::RefCell;

use vst3::Steinberg::Vst::{
    IParamValueQueue, IParamValueQueueTrait, IParameterChanges, IParameterChangesTrait, ParamID, ParamValue,
};
use vst3::Steinberg::{int32, kInvalidArgument, kResultOk, tresult};
use vst3::{Class, ComPtr, ComWrapper};

pub struct Turn {
    id: ParamID,
    value: RefCell<ParamValue>,
}

impl Class for Turn {
    type Interfaces = (IParamValueQueue,);
}

impl IParamValueQueueTrait for Turn {
    unsafe fn getParameterId(&self) -> ParamID {
        self.id
    }

    unsafe fn getPointCount(&self) -> int32 {
        1
    }

    unsafe fn getPoint(&self, index: int32, offset: *mut int32, value: *mut ParamValue) -> tresult {
        if index != 0 || offset.is_null() || value.is_null() {
            return kInvalidArgument;
        }
        *offset = 0;
        *value = *self.value.borrow();
        kResultOk
    }

    unsafe fn addPoint(&self, _offset: int32, value: ParamValue, index: *mut int32) -> tresult {
        *self.value.borrow_mut() = value;
        if !index.is_null() {
            *index = 0;
        }
        kResultOk
    }
}

pub struct Turns {
    held: RefCell<Vec<ComWrapper<Turn>>>,
}

impl Class for Turns {
    type Interfaces = (IParameterChanges,);
}

impl Turns {
    pub fn empty() -> ComWrapper<Self> {
        ComWrapper::new(Self { held: RefCell::new(Vec::new()) })
    }

    pub fn set(&self, id: ParamID, value: ParamValue) {
        let mut held = self.held.borrow_mut();
        if let Some(found) = held.iter().find(|kept| kept.id == id) {
            *found.value.borrow_mut() = value;
            return;
        }
        held.push(ComWrapper::new(Turn { id, value: RefCell::new(value) }));
    }

    pub fn clear(&self) {
        self.held.borrow_mut().clear();
    }

    pub fn waiting(&self) -> bool {
        !self.held.borrow().is_empty()
    }
}

impl IParameterChangesTrait for Turns {
    unsafe fn getParameterCount(&self) -> int32 {
        self.held.borrow().len() as int32
    }

    unsafe fn getParameterData(&self, index: int32) -> *mut IParamValueQueue {
        let held = self.held.borrow();
        match held.get(index.max(0) as usize) {
            Some(found) => match found.as_com_ref::<IParamValueQueue>() {
                Some(seen) => seen.as_ptr(),
                None => std::ptr::null_mut(),
            },
            None => std::ptr::null_mut(),
        }
    }

    unsafe fn addParameterData(&self, id: *const ParamID, index: *mut int32) -> *mut IParamValueQueue {
        if id.is_null() {
            return std::ptr::null_mut();
        }
        self.set(*id, 0.0);
        let held = self.held.borrow();
        let at = held.len().saturating_sub(1);
        if !index.is_null() {
            *index = at as int32;
        }
        match held.get(at).and_then(|found| found.as_com_ref::<IParamValueQueue>()) {
            Some(seen) => seen.as_ptr(),
            None => std::ptr::null_mut(),
        }
    }
}

pub fn as_pointer(wrapper: &ComWrapper<Turns>) -> *mut IParameterChanges {
    match wrapper.as_com_ref::<IParameterChanges>() {
        Some(found) => found.as_ptr(),
        None => std::ptr::null_mut(),
    }
}

pub type Held = ComPtr<IParameterChanges>;
