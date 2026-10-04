use vst3::Steinberg::Vst::{
    IComponent, IComponentHandler, IComponentHandlerTrait, IConnectionPoint, IConnectionPointTrait, IEditController, IEditControllerTrait,
};
use vst3::Steinberg::{kResultOk, IPlugView, IPlugViewTrait, IPluginBaseTrait, ViewRect};
use vst3::{Class, ComPtr, ComWrapper};

pub struct Quiet;

impl Class for Quiet {
    type Interfaces = (IComponentHandler,);
}

impl IComponentHandlerTrait for Quiet {
    unsafe fn beginEdit(&self, _id: u32) -> i32 {
        kResultOk
    }

    unsafe fn performEdit(&self, _id: u32, _value: f64) -> i32 {
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
}

impl Editor {
    /// For a plugin that keeps its window in a controller of its own: start the
    /// controller, show it what the processor holds, and connect the two, which is
    /// what a plugin expects of a host before it will make its window.
    /// # Safety
    /// `context` must be a live host context, or null.
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
            let handler = ComWrapper::new(Quiet);
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
            Ok(Self { view, _controller: controller, _handler: handler, links })
        }
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

    /// Whether the plugin will redraw itself at another size. A plugin that says no
    /// gets a window that cannot be dragged, so there is never a gap beside it.
    pub fn can_resize(&self) -> bool {
        unsafe { self.view.canResize() == kResultOk }
    }

    /// Tell the plugin the window is a new size, and let it answer with the size it
    /// would rather be.
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

    /// # Safety
    /// `window` must be a live window of the kind `kind` names, and must outlive this
    /// editor: the plugin draws into it until the editor is dropped.
    pub unsafe fn attach(&self, window: *mut std::ffi::c_void, kind: &[u8]) -> Result<(), String> {
        unsafe {
            if self.view.attached(window, kind.as_ptr() as *const i8) != kResultOk {
                return Err("the plugin would not draw in our window".into());
            }
        }
        Ok(())
    }
}

impl Drop for Editor {
    fn drop(&mut self) {
        unsafe {
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
    /// For a plugin where one object is both the processor and the window: it was
    /// started when the plugin was loaded, so it is only given a handler and asked
    /// for its window.
    /// # Safety
    /// `context` must be a live host context, or null.
    pub unsafe fn already_started(
        controller: ComPtr<IEditController>,
        context: *mut vst3::Steinberg::FUnknown,
    ) -> Result<Self, String> {
        unsafe {
            let handler = ComWrapper::new(Quiet);
            if let Some(reference) = handler.as_com_ref::<IComponentHandler>() {
                controller.setComponentHandler(reference.as_ptr());
            }
            let mut raw = controller.createView(b"editor\0".as_ptr() as *const i8);
            if raw.is_null() {
                // Some plugins want starting again before they will part with a window,
                // even though one object is doing both jobs.
                controller.initialize(context);
                raw = controller.createView(b"editor\0".as_ptr() as *const i8);
            }
            let view = ComPtr::from_raw(raw).ok_or("this plugin has no window")?;
            // One object doing both jobs has nothing to connect itself to.
            Ok(Self { view, _controller: controller, _handler: handler, links: None })
        }
    }
}
