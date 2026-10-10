use vst3::Steinberg::Vst::{
    IComponent, IComponentHandler, IComponentHandlerTrait, IConnectionPoint, IConnectionPointTrait, IEditController, IEditControllerTrait,
};
use vst3::Steinberg::{kResultOk, IPlugView, IPlugViewTrait, IPluginBaseTrait, ViewRect};
use std::ffi::CStr;
use vst3::Steinberg::kResultFalse;
use vst3::{Class, ComPtr, ComWrapper};

pub struct Frame {
    wanted: std::sync::Mutex<Option<(i32, i32)>>,
}

impl Class for Frame {
    type Interfaces = (vst3::Steinberg::IPlugFrame,);
}

impl vst3::Steinberg::IPlugFrameTrait for Frame {
    unsafe fn resizeView(&self, view: *mut IPlugView, new_size: *mut ViewRect) -> i32 {
        if new_size.is_null() {
            return vst3::Steinberg::kInvalidArgument;
        }
        let asked = unsafe { *new_size };
        let (width, height) = (asked.right - asked.left, asked.bottom - asked.top);
        if let Ok(mut held) = self.wanted.lock() {
            *held = Some((width.max(1), height.max(1)));
        }
        if let Some(view) = unsafe { ComPtr::from_raw(view) } {
            unsafe { view.onSize(new_size) };
            std::mem::forget(view);
        }
        kResultOk
    }
}


pub struct Quiet {
    turns: std::cell::RefCell<Option<vst3::ComWrapper<crate::changes::Turns>>>,
    touched: std::cell::Cell<bool>,
}

impl Quiet {
    pub fn new() -> Self {
        Self { turns: std::cell::RefCell::new(None), touched: std::cell::Cell::new(false) }
    }

    pub fn was_touched(&self) -> bool {
        self.touched.replace(false)
    }

    pub fn passes_edits_to(&self, turns: vst3::ComWrapper<crate::changes::Turns>) {
        *self.turns.borrow_mut() = Some(turns);
    }
}

impl Default for Quiet {
    fn default() -> Self {
        Self::new()
    }
}

impl Class for Quiet {
    type Interfaces = (IComponentHandler,);
}

impl IComponentHandlerTrait for Quiet {
    unsafe fn beginEdit(&self, _id: u32) -> i32 {
        kResultOk
    }

    unsafe fn performEdit(&self, id: u32, value: f64) -> i32 {
        self.touched.set(true);
        if let Some(turns) = self.turns.borrow().as_ref() {
            turns.set(id, value);
        }
        kResultOk
    }

    unsafe fn endEdit(&self, _id: u32) -> i32 {
        kResultOk
    }

    unsafe fn restartComponent(&self, _flags: i32) -> i32 {
        kResultOk
    }
}

pub struct Editor {
    view: ComPtr<IPlugView>,
    _controller: ComPtr<IEditController>,
    _handler: ComWrapper<Quiet>,
    links: Option<(ComPtr<IConnectionPoint>, ComPtr<IConnectionPoint>)>,
    frame: ComWrapper<Frame>,
}

impl Editor {
    pub unsafe fn joined(
        controller: ComPtr<IEditController>,
        component: &ComPtr<IComponent>,
        settings: &[u8],
        context: *mut vst3::Steinberg::FUnknown,
    ) -> Result<Self, String> {
        unsafe {
            if controller.initialize(context) != kResultOk {
                return Err("the plugin's window would not start up".into());
            }
            let links = match (component.cast::<IConnectionPoint>(), controller.cast::<IConnectionPoint>()) {
                (Some(one), Some(other)) => {
                    one.connect(other.as_ptr());
                    other.connect(one.as_ptr());
                    Some((one, other))
                }
                _ => None,
            };
            if !settings.is_empty() {
                let wrapper = crate::stream::Bytes::holding(settings.to_vec());
                if let Some(stream) = wrapper.as_com_ref::<vst3::Steinberg::IBStream>() {
                    controller.setComponentState(stream.as_ptr());
                }
            }
            Self::viewed(controller, links)
        }
    }

    fn viewed(
        controller: ComPtr<IEditController>,
        links: Option<(ComPtr<IConnectionPoint>, ComPtr<IConnectionPoint>)>,
    ) -> Result<Self, String> {
        unsafe {
            let handler = ComWrapper::new(Quiet::new());
            if let Some(reference) = handler.as_com_ref::<IComponentHandler>() {
                controller.setComponentHandler(reference.as_ptr());
            }
            let raw = controller.createView(b"editor\0".as_ptr() as *const i8);
            let Some(view) = ComPtr::from_raw(raw) else {
                if let Some((one, other)) = links {
                    one.disconnect(other.as_ptr());
                    other.disconnect(one.as_ptr());
                }
                return Err("this plugin has no window".into());
            };
            let frame = ComWrapper::new(Frame { wanted: std::sync::Mutex::new(None) });
            if let Some(reference) = frame.as_com_ref::<vst3::Steinberg::IPlugFrame>() {
                view.setFrame(reference.as_ptr());
            }
            Ok(Self { view, _controller: controller, _handler: handler, links, frame })
        }
    }

    pub fn passes_edits_to(&self, turns: vst3::ComWrapper<crate::changes::Turns>) {
        self._handler.passes_edits_to(turns);
    }

    pub fn fits(&self, kind: &[u8]) -> bool {
        unsafe { self.view.isPlatformTypeSupported(kind.as_ptr() as *const i8) == kResultOk }
    }

    pub fn size(&self) -> (i32, i32) {
        let mut rect = ViewRect { left: 0, top: 0, right: 600, bottom: 400 };
        unsafe {
            self.view.getSize(&mut rect);
        }
        ((rect.right - rect.left).max(80), (rect.bottom - rect.top).max(60))
    }

    pub fn was_touched(&self) -> bool {
        self._handler.was_touched()
    }

    pub fn told_its_track(&self, name: &str, index: i64) -> bool {
        use vst3::Steinberg::Vst::ChannelContext::IInfoListenerTrait;
        let Some(ears) = self._controller.cast::<vst3::Steinberg::Vst::ChannelContext::IInfoListener>() else {
            return false;
        };
        let facts = ComWrapper::new(Facts::new(name, index));
        let Some(list) = facts.to_com_ptr::<vst3::Steinberg::Vst::IAttributeList>() else {
            return false;
        };
        unsafe { ears.setChannelContextInfos(list.as_ptr()) == kResultOk }
    }

    pub fn told_its_scale(&self, scale: f32) -> bool {
        use vst3::Steinberg::IPlugViewContentScaleSupportTrait;
        let Some(aware) = self.view.cast::<vst3::Steinberg::IPlugViewContentScaleSupport>() else {
            return false;
        };
        unsafe { aware.setContentScaleFactor(scale) == kResultOk }
    }

    pub fn can_resize(&self) -> bool {
        unsafe { self.view.canResize() == kResultOk }
    }

    pub fn resized(&self, width: i32, height: i32) -> (i32, i32) {
        let mut rect = ViewRect { left: 0, top: 0, right: width, bottom: height };
        unsafe {
            if self.view.checkSizeConstraint(&mut rect) != kResultOk {
                rect = ViewRect { left: 0, top: 0, right: width, bottom: height };
            }
            self.view.onSize(&mut rect);
        }
        ((rect.right - rect.left).max(80), (rect.bottom - rect.top).max(60))
    }

    pub unsafe fn attach(&self, window: *mut std::ffi::c_void, kind: &[u8]) -> Result<(), String> {
        unsafe {
            if self.view.attached(window, kind.as_ptr() as *const i8) != kResultOk {
                return Err("the plugin would not draw in our window".into());
            }
        }
        Ok(())
    }

    pub fn wanted_size(&self) -> Option<(i32, i32)> {
        self.frame.wanted.lock().ok().and_then(|mut held| held.take())
    }
}

impl Drop for Editor {
    fn drop(&mut self) {
        unsafe {
            self.view.setFrame(std::ptr::null_mut());
            self.view.removed();
            if let Some((one, other)) = self.links.take() {
                one.disconnect(other.as_ptr());
                other.disconnect(one.as_ptr());
            }
        }
    }
}

pub fn platform_kind() -> &'static [u8] {
    if cfg!(windows) {
        b"HWND\0"
    } else if cfg!(target_os = "macos") {
        b"NSView\0"
    } else {
        b"X11EmbedWindowID\0"
    }
}

impl Editor {
    pub unsafe fn already_started(
        controller: ComPtr<IEditController>,
        context: *mut vst3::Steinberg::FUnknown,
    ) -> Result<Self, String> {
        unsafe {
            let handler = ComWrapper::new(Quiet::new());
            if let Some(reference) = handler.as_com_ref::<IComponentHandler>() {
                controller.setComponentHandler(reference.as_ptr());
            }
            let mut raw = controller.createView(b"editor\0".as_ptr() as *const i8);
            if raw.is_null() {
                controller.initialize(context);
                raw = controller.createView(b"editor\0".as_ptr() as *const i8);
            }
            let view: ComPtr<IPlugView> = ComPtr::from_raw(raw).ok_or("this plugin has no window")?;
            let frame = ComWrapper::new(Frame { wanted: std::sync::Mutex::new(None) });
            if let Some(reference) = frame.as_com_ref::<vst3::Steinberg::IPlugFrame>() {
                view.setFrame(reference.as_ptr());
            }
            Ok(Self { view, _controller: controller, _handler: handler, links: None, frame })
        }
    }
}

pub struct Facts {
    name: Vec<u16>,
    index: i64,
}

impl Facts {
    pub fn new(name: &str, index: i64) -> Self {
        let mut wide: Vec<u16> = name.encode_utf16().collect();
        wide.push(0);
        Self { name: wide, index }
    }
}

impl Class for Facts {
    type Interfaces = (vst3::Steinberg::Vst::IAttributeList,);
}

fn key_is(id: vst3::Steinberg::Vst::IAttributeList_::AttrID, want: &str) -> bool {
    if id.is_null() {
        return false;
    }
    let raw = unsafe { CStr::from_ptr(id) };
    raw.to_str().map(|text| text == want).unwrap_or(false)
}

impl vst3::Steinberg::Vst::IAttributeListTrait for Facts {
    unsafe fn setInt(&self, _id: vst3::Steinberg::Vst::IAttributeList_::AttrID, _value: i64) -> i32 {
        kResultOk
    }

    unsafe fn getInt(&self, id: vst3::Steinberg::Vst::IAttributeList_::AttrID, value: *mut i64) -> i32 {
        if value.is_null() {
            return kResultFalse;
        }
        if key_is(id, "channel index") {
            *value = self.index;
            return kResultOk;
        }
        if key_is(id, "channel index namespace length") {
            *value = 0;
            return kResultOk;
        }
        kResultFalse
    }

    unsafe fn setFloat(&self, _id: vst3::Steinberg::Vst::IAttributeList_::AttrID, _value: f64) -> i32 {
        kResultOk
    }

    unsafe fn getFloat(&self, _id: vst3::Steinberg::Vst::IAttributeList_::AttrID, _value: *mut f64) -> i32 {
        kResultFalse
    }

    unsafe fn setString(&self, _id: vst3::Steinberg::Vst::IAttributeList_::AttrID, _string: *const u16) -> i32 {
        kResultOk
    }

    unsafe fn getString(&self, id: vst3::Steinberg::Vst::IAttributeList_::AttrID, string: *mut u16, size: u32) -> i32 {
        if string.is_null() || !key_is(id, "channel name") {
            return kResultFalse;
        }
        let room = (size as usize / 2).saturating_sub(1);
        let count = self.name.len().saturating_sub(1).min(room);
        for (at, letter) in self.name.iter().take(count).enumerate() {
            *string.add(at) = *letter;
        }
        *string.add(count) = 0;
        kResultOk
    }

    unsafe fn setBinary(&self, _id: vst3::Steinberg::Vst::IAttributeList_::AttrID, _data: *const std::ffi::c_void, _size: u32) -> i32 {
        kResultOk
    }

    unsafe fn getBinary(&self, _id: vst3::Steinberg::Vst::IAttributeList_::AttrID, _data: *mut *const std::ffi::c_void, _size: *mut u32) -> i32 {
        kResultFalse
    }
}
