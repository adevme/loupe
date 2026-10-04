use vst3::Steinberg::Vst::{IComponentHandler, IComponentHandlerTrait, IEditController, IEditControllerTrait};
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
}

impl Editor {
    pub fn from(controller: ComPtr<IEditController>) -> Result<Self, String> {
        unsafe {
            if controller.initialize(std::ptr::null_mut()) != kResultOk {
                return Err("the plugin's window would not start up".into());
            }
            let handler = ComWrapper::new(Quiet);
            if let Some(reference) = handler.as_com_ref::<IComponentHandler>() {
                controller.setComponentHandler(reference.as_ptr());
            }
            let raw = controller.createView(b"editor\0".as_ptr() as *const i8);
            let view = ComPtr::from_raw(raw).ok_or("this plugin has no window")?;
            Ok(Self { view, _controller: controller, _handler: handler })
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

    pub fn attach(&self, window: *mut std::ffi::c_void, kind: &[u8]) -> Result<(), String> {
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
