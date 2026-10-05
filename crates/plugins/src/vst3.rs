use std::ffi::CStr;
use std::path::{Path, PathBuf};

use vst3::{ComPtr, Steinberg::{IPluginFactory, IPluginFactoryTrait}};

pub struct Library {
    factory: ComPtr<IPluginFactory>,
    us: vst3::ComWrapper<crate::context::Us>,
    exit: Option<unsafe extern "C" fn() -> bool>,
    _lib: libloading::Library,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Class {
    pub name: String,
    pub category: String,
    pub id: [u8; 16],
}

pub fn binary_in(bundle: &Path) -> PathBuf {
    if bundle.is_file() {
        return bundle.to_path_buf();
    }
    let machine = if cfg!(windows) {
        "x86_64-win"
    } else if cfg!(target_os = "macos") {
        "MacOS"
    } else {
        "x86_64-linux"
    };
    let inside = bundle.join("Contents").join(machine);
    let name = bundle.file_name().map(|name| name.to_os_string()).unwrap_or_default();
    let direct = inside.join(&name);
    if direct.exists() {
        return direct;
    }
    std::fs::read_dir(&inside)
        .ok()
        .and_then(|entries| entries.flatten().map(|entry| entry.path()).find(|path| path.is_file()))
        .unwrap_or(direct)
}

impl Library {
    pub fn open(bundle: &Path) -> Result<Self, String> {
        let binary = binary_in(bundle);
        let lib = unsafe { libloading::Library::new(&binary) }.map_err(|why| format!("{} could not be opened: {why}", binary.display()))?;
        unsafe {
            if let Ok(enter) = lib.get::<unsafe extern "C" fn() -> bool>(entry_name()) {
                if !enter() {
                    return Err("the plugin refused to start".into());
                }
            }
            let get_factory = lib
                .get::<unsafe extern "C" fn() -> *mut IPluginFactory>(b"GetPluginFactory\0")
                .map_err(|_| "this file is not a VST3 plugin".to_string())?;
            let raw = get_factory();
            let factory = ComPtr::from_raw(raw).ok_or("the plugin gave back no factory")?;
            let exit = lib.get::<unsafe extern "C" fn() -> bool>(exit_name()).ok().map(|symbol| *symbol);
            let us = crate::context::ours();
            if let Some(newer) = factory.cast::<vst3::Steinberg::IPluginFactory3>() {
                let context = us
                    .as_com_ref::<vst3::Steinberg::FUnknown>()
                    .map(|found| found.as_ptr())
                    .unwrap_or(std::ptr::null_mut());
                use vst3::Steinberg::IPluginFactory3Trait;
                newer.setHostContext(context);
            }
            Ok(Self { factory, us, exit, _lib: lib })
        }
    }

    pub unsafe fn make<I: vst3::Interface>(&self, id: &[u8; 16]) -> Result<ComPtr<I>, String> {
        let mut made: *mut std::ffi::c_void = std::ptr::null_mut();
        let cid = id.map(|b| b as i8);
        let iid = I::IID;
        let ok = self.factory.createInstance(cid.as_ptr(), &iid as *const _ as *const i8, &mut made);
        if ok != vst3::Steinberg::kResultOk || made.is_null() {
            return Err("the plugin would not make that part".into());
        }
        ComPtr::from_raw(made as *mut I).ok_or_else(|| "the plugin gave back nothing".to_string())
    }

    pub fn classes(&self) -> Vec<Class> {
        let mut classes = Vec::new();
        unsafe {
            let count = self.factory.countClasses();
            for index in 0..count {
                let mut info = std::mem::zeroed();
                if self.factory.getClassInfo(index, &mut info) != vst3::Steinberg::kResultOk {
                    continue;
                }
                let text = |bytes: &[i8]| {
                    let raw: Vec<u8> = bytes.iter().map(|c| *c as u8).collect();
                    CStr::from_bytes_until_nul(&raw).map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
                };
                classes.push(Class {
                    name: text(&info.name),
                    category: text(&info.category),
                    id: info.cid.map(|c| c as u8),
                });
            }
        }
        classes
    }
}

impl Drop for Library {
    fn drop(&mut self) {
        if let Some(exit) = self.exit {
            unsafe {
                exit();
            }
        }
    }
}

fn entry_name() -> &'static [u8] {
    if cfg!(windows) {
        b"InitDll\0"
    } else if cfg!(target_os = "macos") {
        b"bundleEntry\0"
    } else {
        b"ModuleEntry\0"
    }
}

fn exit_name() -> &'static [u8] {
    if cfg!(windows) {
        b"ExitDll\0"
    } else if cfg!(target_os = "macos") {
        b"bundleExit\0"
    } else {
        b"ModuleExit\0"
    }
}

use vst3::Steinberg::Vst::{
    AudioBusBuffers, BusDirections_, IAudioProcessor, IAudioProcessorTrait, IComponent, IComponentTrait, IoModes_,
    MediaTypes_, ProcessData, ProcessModes_, SymbolicSampleSizes_,
};
use vst3::Steinberg::{kResultOk, IPluginBaseTrait};

use crate::ara_document::{pack, unpack, Document};
use crate::wire::Region;

pub struct Effect {
    processor: ComPtr<IAudioProcessor>,
    component: ComPtr<IComponent>,
    us: vst3::ComWrapper<crate::context::Us>,
    left: Vec<f32>,
    right: Vec<f32>,
    side_left: Vec<f32>,
    side_right: Vec<f32>,
    pub side_bus: bool,
    turns: vst3::ComWrapper<crate::changes::Turns>,
    ids: Vec<u32>,
    steps: Vec<u32>,
    controller: Option<ComPtr<vst3::Steinberg::Vst::IEditController>>,
    ara: Option<Document>,
    context: vst3::Steinberg::Vst::ProcessContext,
    rate: f64,
    _library: Library,
}

impl Effect {
    pub fn start(library: Library, index: usize, rate: f64, block: usize) -> Result<Self, String> {
        Self::start_on(library, index, rate, block, None)
    }

    pub fn start_on(library: Library, index: usize, rate: f64, block: usize, region: Option<&Region>) -> Result<Self, String> {
        let classes = library.classes();
        let class = classes.get(index).ok_or("that plugin has no such part")?;
        let component: ComPtr<IComponent> = unsafe { library.make(&class.id) }?;
        let us = crate::context::ours();
        let context = us
            .as_com_ref::<vst3::Steinberg::FUnknown>()
            .map(|found| found.as_ptr())
            .unwrap_or(std::ptr::null_mut());
        let _ = &library.us;
        unsafe {
            if component.initialize(context) != kResultOk {
                return Err("the plugin would not start up".into());
            }
            component.setIoMode(IoModes_::kAdvanced as i32);
            let ara = match region {
                Some(region) => join_ara(&component, region)?,
                None => None,
            };
            let processor: ComPtr<IAudioProcessor> = component.cast().ok_or("that plugin does not process audio")?;
            let mut setup = vst3::Steinberg::Vst::ProcessSetup {
                processMode: ProcessModes_::kRealtime as i32,
                symbolicSampleSize: SymbolicSampleSizes_::kSample32 as i32,
                maxSamplesPerBlock: block as i32,
                sampleRate: rate,
            };
            if processor.setupProcessing(&mut setup) != kResultOk {
                return Err("the plugin refused this sample rate or block size".into());
            }
            let ins = component.getBusCount(MediaTypes_::kAudio as i32, BusDirections_::kInput as i32);
            let outs = component.getBusCount(MediaTypes_::kAudio as i32, BusDirections_::kOutput as i32);
            for bus in 0..ins {
                component.activateBus(MediaTypes_::kAudio as i32, BusDirections_::kInput as i32, bus, 1);
            }
            for bus in 0..outs {
                component.activateBus(MediaTypes_::kAudio as i32, BusDirections_::kOutput as i32, bus, 1);
            }
            component.setActive(1);
            processor.setProcessing(1);
            Ok(Self {
                processor,
                component,
                us,
                left: vec![0.0; block],
                right: vec![0.0; block],
                side_left: vec![0.0; block],
                side_right: vec![0.0; block],
                side_bus: ins > 1,
                turns: crate::changes::Turns::empty(),
                ids: Vec::new(),
                steps: Vec::new(),
                controller: None,
                ara,
                context: std::mem::zeroed(),
                rate,
                _library: library,
            })
        }
    }

    pub fn editor(&mut self) -> Result<crate::editor::Editor, String> {
        use vst3::Steinberg::Vst::IEditController;
        if let Some(document) = self.ara.as_mut() {
            document.start_reading();
        }
        let made = unsafe {
            let context = self
                .us
                .as_com_ref::<vst3::Steinberg::FUnknown>()
                .map(|found| found.as_ptr())
                .unwrap_or(std::ptr::null_mut());
            let mut cid = [0i8; 16];
            let named = (self.component.getControllerClassId(&mut cid) == kResultOk).then(|| cid.map(|c| c as u8));
            let listed = self
                ._library
                .classes()
                .into_iter()
                .find(|class| class.category == "Component Controller Class")
                .map(|class| class.id);
            let mut refused = String::new();
            let mut separate = None;
            for id in [named, listed].into_iter().flatten() {
                match self._library.make::<IEditController>(&id) {
                    Ok(controller) => {
                        separate = Some(controller);
                        break;
                    }
                    Err(why) => refused = why,
                }
            }
            match separate {
                Some(controller) => {
                    let settings = self.settings().unwrap_or_default();
                    crate::editor::Editor::joined(controller, &self.component, &settings, context)
                }
                None if !refused.is_empty() => Err(format!("the window lives in another part of the plugin, and {refused}")),
                None => {
                    let controller: ComPtr<IEditController> = self.component.cast().ok_or("this plugin has no window")?;
                    crate::editor::Editor::already_started(controller, context)
                }
            }
        }?;
        if let Some(document) = self.ara.as_ref() {
            document.select();
        }
        Ok(made)
    }

    pub fn is_ara(&self) -> bool {
        self.ara.is_some()
    }

    pub fn idle(&self) {
        if let Some(document) = self.ara.as_ref() {
            document.idle();
        }
    }

    pub fn place(&mut self, region: &Region) -> Result<(), String> {
        let Some(mut document) = self.ara.take() else {
            return Err("this plugin is not following a clip".into());
        };
        let placed = if document.same_audio(region) {
            document.place(region);
            Ok(())
        } else {
            match crate::ara_audio::Samples::read(Path::new(&region.file)) {
                Ok(samples) => {
                    self.pause();
                    let swapped = unsafe { document.swap_audio(region, samples) };
                    self.resume();
                    swapped
                }
                Err(why) => Err(format!("ARA needs the clip's audio file: {why}")),
            }
        };
        self.ara = Some(document);
        placed
    }

    fn pause(&self) {
        unsafe {
            self.processor.setProcessing(0);
            self.component.setActive(0);
        }
    }

    fn resume(&self) {
        unsafe {
            self.component.setActive(1);
            self.processor.setProcessing(1);
        }
    }

    pub fn process_at(&mut self, audio: &mut [[f32; 2]], side: &[[f32; 2]], at: i64) {
        if let Some(document) = self.ara.as_mut() {
            document.start_reading();
            self.context = playing_at(at, self.rate, document.placed().tempo);
        }
        self.process_with(audio, side);
    }

    pub fn latency(&self) -> usize {
        unsafe { self.processor.getLatencySamples() as usize }
    }

    fn controller(&mut self) -> Option<ComPtr<vst3::Steinberg::Vst::IEditController>> {
        use vst3::Steinberg::Vst::IEditController;
        if let Some(found) = self.controller.as_ref() {
            return Some(found.clone());
        }
        unsafe {
            let mut cid = [0i8; 16];
            let separate: Option<ComPtr<IEditController>> = if self.component.getControllerClassId(&mut cid) == kResultOk {
                let id = cid.map(|c| c as u8);
                self._library.make::<IEditController>(&id).ok()
            } else {
                None
            };
            let controller = separate.or_else(|| self.component.cast())?;
            let context = self
                .us
                .as_com_ref::<vst3::Steinberg::FUnknown>()
                .map(|found| found.as_ptr())
                .unwrap_or(std::ptr::null_mut());
            if controller.initialize(context) != kResultOk {
                return None;
            }
            self.controller = Some(controller.clone());
            Some(controller)
        }
    }

    pub fn knobs(&mut self) -> Vec<String> {
        use vst3::Steinberg::Vst::IEditControllerTrait;
        let mut out = Vec::new();
        let mut steps = Vec::new();
        let mut seen = Vec::new();
        let Some(controller) = self.controller() else { return out };
        unsafe {
            let count = controller.getParameterCount();
            for index in 0..count.min(crate::wording::MOST_KNOBS as _) {
                let mut about: vst3::Steinberg::Vst::ParameterInfo = std::mem::zeroed();
                if controller.getParameterInfo(index, &mut about) != kResultOk {
                    continue;
                }
                let raw: Vec<u16> = about.title.iter().take_while(|unit| **unit != 0).copied().collect();
                out.push(String::from_utf16_lossy(&raw));
                seen.push(about.id);
                steps.push(about.stepCount.max(0) as u32);
            }
        }
        self.ids = seen;
        self.steps = steps;
        out
    }

    pub fn turn(&mut self, knob: usize, value: f32) {
        if self.ids.is_empty() {
            let _ = self.knobs();
        }
        let Some(id) = self.ids.get(knob).copied() else { return };
        self.turns.set(id, value.clamp(0.0, 1.0) as f64);
    }

    pub fn readings(&mut self) -> Vec<crate::wire::Reading> {
        use vst3::Steinberg::Vst::IEditControllerTrait;
        let names = self.knobs();
        let Some(controller) = self.controller() else { return Vec::new() };
        if let Ok(state) = self.settings() {
            let wrapper = crate::stream::Bytes::holding(state);
            if let Some(stream) = wrapper.as_com_ref::<vst3::Steinberg::IBStream>() {
                unsafe {
                    controller.setComponentState(stream.as_ptr());
                }
            }
        }
        let mut out = Vec::new();
        for (name, id) in names.into_iter().zip(self.ids.clone()) {
            let value = self.turns.get(id).unwrap_or_else(|| unsafe { controller.getParamNormalized(id) });
            let mut words = [0u16; 128];
            let text = unsafe {
                if controller.getParamStringByValue(id, value, &mut words) == kResultOk {
                    let raw: Vec<u16> = words.iter().take_while(|unit| **unit != 0).copied().collect();
                    String::from_utf16_lossy(&raw).trim().to_string()
                } else {
                    String::new()
                }
            };
            out.push(crate::wire::Reading { name, value: value as f32, text });
        }
        out
    }

    pub fn from_text(&mut self, knob: usize, text: &str) -> Option<f32> {
        use vst3::Steinberg::Vst::IEditControllerTrait;
        if self.ids.is_empty() {
            let _ = self.knobs();
        }
        let id = self.ids.get(knob).copied()?;
        let controller = self.controller()?;
        let mut words: Vec<u16> = text.encode_utf16().collect();
        words.push(0);
        let mut value = 0.0f64;
        let found = unsafe { controller.getParamValueByString(id, words.as_mut_ptr(), &mut value) };
        if found == kResultOk && value.is_finite() {
            return Some(value.clamp(0.0, 1.0) as f32);
        }
        let steps = self.steps.get(knob).copied().unwrap_or(0);
        let say = |value: f64| {
            let mut words = [0u16; 128];
            (unsafe { controller.getParamStringByValue(id, value, &mut words) } == kResultOk).then(|| {
                let raw: Vec<u16> = words.iter().take_while(|unit| **unit != 0).copied().collect();
                String::from_utf16_lossy(&raw)
            })
        };
        crate::wording::find_value(steps, text, say).map(|value| value as f32)
    }

    pub fn save(&self) -> Result<Vec<u8>, String> {
        match self.ara.as_ref() {
            Some(document) => Ok(pack(&self.settings().unwrap_or_default(), &document.archive_id(), &document.store()?)),
            None => self.settings(),
        }
    }

    pub fn restore(&mut self, state: &[u8]) -> Result<(), String> {
        let Some(unpacked) = unpack(state) else {
            let settled = self.take_settings(state);
            self.begin_reading();
            return settled;
        };
        let restored = match self.ara.as_mut() {
            Some(document) => document.restore(unpacked.archive_id, unpacked.archive),
            None => Ok(()),
        };
        let settled = self.take_settings(unpacked.settings);
        self.begin_reading();
        restored.and(settled)
    }

    fn begin_reading(&mut self) {
        if let Some(document) = self.ara.as_mut() {
            document.start_reading();
        }
    }

    pub fn settings(&self) -> Result<Vec<u8>, String> {
        let wrapper = crate::stream::Bytes::empty();
        let stream = wrapper.as_com_ref::<vst3::Steinberg::IBStream>().ok_or("no stream")?;
        unsafe {
            if self.component.getState(stream.as_ptr()) != kResultOk {
                return Err("the plugin would not hand over its settings".into());
            }
        }
        Ok(wrapper.taken())
    }

    pub fn take_settings(&mut self, state: &[u8]) -> Result<(), String> {
        if state.is_empty() {
            return Ok(());
        }
        let wrapper = crate::stream::Bytes::holding(state.to_vec());
        let stream = wrapper.as_com_ref::<vst3::Steinberg::IBStream>().ok_or("no stream")?;
        unsafe {
            if self.component.setState(stream.as_ptr()) != kResultOk {
                return Err("the plugin would not take those settings".into());
            }
        }
        Ok(())
    }

    pub fn process(&mut self, audio: &mut [[f32; 2]]) {
        self.process_with(audio, &[]);
    }

    pub fn process_with(&mut self, audio: &mut [[f32; 2]], side: &[[f32; 2]]) {
        let frames = audio.len().min(self.left.len());
        for (i, frame) in audio.iter().take(frames).enumerate() {
            self.left[i] = frame[0];
            self.right[i] = frame[1];
        }
        for i in 0..frames {
            let frame = side.get(i).copied().unwrap_or([0.0; 2]);
            self.side_left[i] = frame[0];
            self.side_right[i] = frame[1];
        }
        unsafe {
            let mut channels = [self.left.as_mut_ptr(), self.right.as_mut_ptr()];
            let mut side_channels = [self.side_left.as_mut_ptr(), self.side_right.as_mut_ptr()];
            let mut bus = AudioBusBuffers {
                numChannels: 2,
                silenceFlags: 0,
                __field0: vst3::Steinberg::Vst::AudioBusBuffers__type0 { channelBuffers32: channels.as_mut_ptr() },
            };
            let mut both = [
                bus,
                AudioBusBuffers {
                    numChannels: 2,
                    silenceFlags: 0,
                    __field0: vst3::Steinberg::Vst::AudioBusBuffers__type0 { channelBuffers32: side_channels.as_mut_ptr() },
                },
            ];
            let ins = if self.side_bus { 2 } else { 1 };
            let mut data = ProcessData {
                processMode: ProcessModes_::kRealtime as i32,
                symbolicSampleSize: SymbolicSampleSizes_::kSample32 as i32,
                numSamples: frames as i32,
                numInputs: ins,
                numOutputs: 1,
                inputs: both.as_mut_ptr(),
                outputs: &mut bus,
                inputParameterChanges: crate::changes::as_pointer(&self.turns),
                outputParameterChanges: std::ptr::null_mut(),
                inputEvents: std::ptr::null_mut(),
                outputEvents: std::ptr::null_mut(),
                processContext: if self.ara.is_some() { &mut self.context } else { std::ptr::null_mut() },
            };
            self.processor.process(&mut data);
        }
        for (i, frame) in audio.iter_mut().take(frames).enumerate() {
            frame[0] = self.left[i];
            frame[1] = self.right[i];
        }
        self.turns.clear();
    }
}

impl Drop for Effect {
    fn drop(&mut self) {
        unsafe {
            if let Some(controller) = self.controller.take() {
                if controller.as_ptr() as *mut () != self.component.as_ptr() as *mut () {
                    controller.terminate();
                }
            }
            self.processor.setProcessing(0);
            self.component.setActive(0);
            self.component.terminate();
        }
    }
}

unsafe fn join_ara(component: &ComPtr<IComponent>, region: &Region) -> Result<Option<Document>, String> {
    use crate::ara::{ask_for, let_go, EntryPoint2Vtbl, EntryPointVtbl, EVERY_ROLE};
    let unknown = component.as_ptr() as *mut vst3::Steinberg::FUnknown;
    let Some(entry) = ask_for::<EntryPointVtbl>(unknown, &crate::ara::IPlugInEntryPoint) else {
        return Ok(None);
    };
    let factory = ((*(*entry).vtbl).get_factory)(entry as *mut std::ffi::c_void);
    let mut document = match Document::open(factory, region) {
        Ok(document) => document,
        Err(why) => {
            let_go(entry);
            return Err(why);
        }
    };
    let extension = match ask_for::<EntryPoint2Vtbl>(unknown, &crate::ara::IPlugInEntryPoint2) {
        Some(second) => {
            let bound = ((*(*second).vtbl).bind_to_document_controller_with_roles)(
                second as *mut std::ffi::c_void,
                document.controller(),
                EVERY_ROLE,
                EVERY_ROLE,
            );
            let_go(second);
            bound
        }
        None => ((*(*entry).vtbl).bind_to_document_controller)(entry as *mut std::ffi::c_void, document.controller()),
    };
    let_go(entry);
    document.attach(extension)?;
    Ok(Some(document))
}

fn playing_at(at: i64, rate: f64, tempo: f64) -> vst3::Steinberg::Vst::ProcessContext {
    use vst3::Steinberg::Vst::ProcessContext_::StatesAndFlags_::{kPlaying, kProjectTimeMusicValid, kTempoValid, kTimeSigValid};
    let mut context: vst3::Steinberg::Vst::ProcessContext = unsafe { std::mem::zeroed() };
    context.state = (kPlaying | kTempoValid | kTimeSigValid | kProjectTimeMusicValid) as u32;
    context.sampleRate = rate;
    context.projectTimeSamples = at;
    context.continousTimeSamples = at;
    context.tempo = tempo;
    context.projectTimeMusic = if rate > 0.0 { at as f64 / rate * tempo / 60.0 } else { 0.0 };
    context.timeSigNumerator = 4;
    context.timeSigDenominator = 4;
    context
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ara_plugin_is_told_where_the_song_is() {
        let context = playing_at(96_000, 48_000.0, 120.0);
        assert_eq!(context.projectTimeSamples, 96_000);
        assert_eq!(context.projectTimeMusic, 4.0);
        assert_eq!(context.tempo, 120.0);
        assert_eq!(context.state & 2, 2);
        assert_eq!((context.timeSigNumerator, context.timeSigDenominator), (4, 4));
    }
}
