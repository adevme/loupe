use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

use vst3::Steinberg::Vst::{
    IParamValueQueue, IParamValueQueueTrait, IParameterChanges, IParameterChangesTrait, ParamID, ParamValue,
};
use vst3::Steinberg::{int32, kInvalidArgument, kResultOk, tresult};
use vst3::{Class, ComPtr, ComWrapper};

pub struct Turn {
    id: ParamID,
    value: AtomicU64,
}

impl Turn {
    fn holding(id: ParamID, value: ParamValue) -> ComWrapper<Self> {
        ComWrapper::new(Self { id, value: AtomicU64::new(value.to_bits()) })
    }

    fn value(&self) -> ParamValue {
        f64::from_bits(self.value.load(Ordering::Relaxed))
    }

    fn becomes(&self, value: ParamValue) {
        self.value.store(value.to_bits(), Ordering::Relaxed);
    }
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
        *value = self.value();
        kResultOk
    }

    unsafe fn addPoint(&self, _offset: int32, value: ParamValue, index: *mut int32) -> tresult {
        self.becomes(value);
        if !index.is_null() {
            *index = 0;
        }
        kResultOk
    }
}

pub struct Turns {
    asked: Mutex<Vec<(ParamID, ParamValue)>>,
    held: Mutex<Vec<ComWrapper<Turn>>>,
}

impl Class for Turns {
    type Interfaces = (IParameterChanges,);
}

fn open<T>(kept: &Mutex<T>) -> MutexGuard<'_, T> {
    kept.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn hold(held: &mut Vec<ComWrapper<Turn>>, id: ParamID, value: ParamValue) {
    match held.iter().find(|kept| kept.id == id) {
        Some(found) => found.becomes(value),
        None => held.push(Turn::holding(id, value)),
    }
}

impl Turns {
    pub fn empty() -> ComWrapper<Self> {
        ComWrapper::new(Self { asked: Mutex::new(Vec::new()), held: Mutex::new(Vec::new()) })
    }

    pub fn set(&self, id: ParamID, value: ParamValue) {
        let mut asked = open(&self.asked);
        match asked.iter_mut().find(|(kept, _)| *kept == id) {
            Some(found) => found.1 = value,
            None => asked.push((id, value)),
        }
    }

    pub fn get(&self, id: ParamID) -> Option<ParamValue> {
        let asked = open(&self.asked).iter().find(|(kept, _)| *kept == id).map(|(_, value)| *value);
        asked.or_else(|| open(&self.held).iter().find(|kept| kept.id == id).map(|kept| kept.value()))
    }

    pub fn take_in(&self) {
        let asked: Vec<(ParamID, ParamValue)> = std::mem::take(&mut *open(&self.asked));
        let mut held = open(&self.held);
        for (id, value) in asked {
            hold(&mut held, id, value);
        }
    }

    pub fn clear(&self) {
        open(&self.held).clear();
    }

    pub fn waiting(&self) -> bool {
        !open(&self.asked).is_empty() || !open(&self.held).is_empty()
    }
}

impl IParameterChangesTrait for Turns {
    unsafe fn getParameterCount(&self) -> int32 {
        open(&self.held).len() as int32
    }

    unsafe fn getParameterData(&self, index: int32) -> *mut IParamValueQueue {
        let held = open(&self.held);
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
        let mut held = open(&self.held);
        hold(&mut held, *id, 0.0);
        let Some(at) = held.iter().position(|kept| kept.id == *id) else { return std::ptr::null_mut() };
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

#[cfg(test)]
mod tests {
    use super::*;

    fn seen_by_the_plugin(turns: &ComWrapper<Turns>) -> Vec<(ParamID, ParamValue)> {
        let changes = turns.as_com_ref::<IParameterChanges>().expect("the changes are a plugin's to read");
        let mut seen = Vec::new();
        unsafe {
            for index in 0..changes.getParameterCount() {
                let queue = vst3::ComRef::from_raw(changes.getParameterData(index)).expect("a queue for every change");
                let mut offset = 0;
                let mut value = 0.0;
                queue.getPoint(0, &mut offset, &mut value);
                seen.push((queue.getParameterId(), value));
            }
        }
        seen
    }

    #[test]
    fn an_edit_made_while_a_block_is_running_waits_for_the_next_block() {
        let turns = Turns::empty();
        turns.set(7, 0.25);
        turns.take_in();
        turns.set(9, 0.75);
        assert_eq!(seen_by_the_plugin(&turns), vec![(7, 0.25)]);
        turns.clear();
        assert!(turns.waiting(), "the edit made during the block was lost");
        turns.take_in();
        assert_eq!(seen_by_the_plugin(&turns), vec![(9, 0.75)]);
        turns.clear();
        assert!(!turns.waiting());
    }

    #[test]
    fn the_latest_edit_of_a_knob_is_the_one_heard_and_read_back() {
        let turns = Turns::empty();
        turns.set(3, 0.1);
        turns.set(3, 0.9);
        assert_eq!(turns.get(3), Some(0.9));
        turns.take_in();
        assert_eq!(seen_by_the_plugin(&turns), vec![(3, 0.9)]);
        assert_eq!(turns.get(3), Some(0.9));
    }

    #[test]
    fn edits_from_another_thread_all_arrive() {
        let turns = Turns::empty();
        let sharing = turns.clone();
        let editing = std::thread::spawn(move || {
            for id in 0..500u32 {
                sharing.set(id, id as f64 / 500.0);
            }
        });
        let mut heard = std::collections::HashMap::new();
        while !editing.is_finished() || turns.waiting() {
            turns.take_in();
            for (id, value) in seen_by_the_plugin(&turns) {
                heard.insert(id, value);
            }
            turns.clear();
        }
        editing.join().expect("the editor thread finished");
        turns.take_in();
        for (id, value) in seen_by_the_plugin(&turns) {
            heard.insert(id, value);
        }
        assert_eq!(heard.len(), 500, "an edit from the window never reached the plugin");
    }
}
