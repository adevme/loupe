use std::ffi::CStr;
use std::path::{Path, PathBuf};

use vst3::{ComPtr, Steinberg::{IPluginFactory, IPluginFactoryTrait}};

pub struct Library {
    factory: ComPtr<IPluginFactory>,
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
            Ok(Self { factory, exit, _lib: lib })
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

pub struct Effect {
    processor: ComPtr<IAudioProcessor>,
    component: ComPtr<IComponent>,
    left: Vec<f32>,
    right: Vec<f32>,
    side_left: Vec<f32>,
    side_right: Vec<f32>,
    pub side_bus: bool,
    turns: vst3::ComWrapper<crate::changes::Turns>,
    ids: Vec<u32>,
    _library: Library,
}

impl Effect {
    pub fn start(library: Library, index: usize, rate: f64, block: usize) -> Result<Self, String> {
        let classes = library.classes();
        let class = classes.get(index).ok_or("that plugin has no such part")?;
        let component: ComPtr<IComponent> = unsafe { library.make(&class.id) }?;
        unsafe {
            if component.initialize(std::ptr::null_mut()) != kResultOk {
                return Err("the plugin would not start up".into());
            }
            component.setIoMode(IoModes_::kAdvanced as i32);
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
                left: vec![0.0; block],
                right: vec![0.0; block],
                side_left: vec![0.0; block],
                side_right: vec![0.0; block],
                side_bus: ins > 1,
                turns: crate::changes::Turns::empty(),
                ids: Vec::new(),
                _library: library,
            })
        }
    }

    pub fn editor(&self) -> Result<crate::editor::Editor, String> {
        use vst3::Steinberg::Vst::IEditController;
        unsafe {
            let mut cid = [0i8; 16];
            if self.component.getControllerClassId(&mut cid) == kResultOk {
                let id = cid.map(|c| c as u8);
                if let Ok(controller) = self._library.make::<IEditController>(&id) {
                    return crate::editor::Editor::from(controller);
                }
            }
            let controller: ComPtr<IEditController> = self.component.cast().ok_or("this plugin has no window")?;
            crate::editor::Editor::from(controller)
        }
    }

    pub fn latency(&self) -> usize {
        unsafe { self.processor.getLatencySamples() as usize }
    }

    pub fn knobs(&mut self) -> Vec<String> {
        use vst3::Steinberg::Vst::{IEditController, IEditControllerTrait};
        let mut out = Vec::new();
        let mut seen = Vec::new();
        unsafe {
            let mut cid = [0i8; 16];
            let controller: Option<ComPtr<IEditController>> = if self.component.getControllerClassId(&mut cid) == kResultOk {
                let id = cid.map(|c| c as u8);
                self._library.make::<IEditController>(&id).ok()
            } else {
                None
            };
            let Some(controller) = controller.or_else(|| self.component.cast()) else { return out };
            if controller.initialize(std::ptr::null_mut()) != kResultOk {
                return out;
            }
            let count = controller.getParameterCount();
            for index in 0..count.min(512) {
                let mut about: vst3::Steinberg::Vst::ParameterInfo = std::mem::zeroed();
                if controller.getParameterInfo(index, &mut about) != kResultOk {
                    continue;
                }
                let raw: Vec<u16> = about.title.iter().take_while(|unit| **unit != 0).copied().collect();
                out.push(String::from_utf16_lossy(&raw));
                seen.push(about.id);
            }
        }
        self.ids = seen;
        out
    }

    pub fn turn(&mut self, knob: usize, value: f32) {
        if self.ids.is_empty() {
            let _ = self.knobs();
        }
        let Some(id) = self.ids.get(knob).copied() else { return };
        self.turns.set(id, value.clamp(0.0, 1.0) as f64);
    }

    pub fn save(&self) -> Result<Vec<u8>, String> {
        let wrapper = crate::stream::Bytes::empty();
        let stream = wrapper.as_com_ref::<vst3::Steinberg::IBStream>().ok_or("no stream")?;
        unsafe {
            if self.component.getState(stream.as_ptr()) != kResultOk {
                return Err("the plugin would not hand over its settings".into());
            }
        }
        Ok(wrapper.taken())
    }

    pub fn restore(&mut self, state: &[u8]) -> Result<(), String> {
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
                processContext: std::ptr::null_mut(),
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
            self.processor.setProcessing(0);
            self.component.setActive(0);
            self.component.terminate();
        }
    }
}
