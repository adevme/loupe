pub mod asio_import;
#[macro_use]
pub mod errors;

use self::errors::{AsioError, AsioErrorWrapper, LoadDriverError};
use num_traits::FromPrimitive;

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_double, c_void};
use std::ptr::null_mut;
use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    Arc, Mutex, MutexGuard, Weak,
};

#[cfg(target_os = "windows")]
use std::os::raw::c_long;
#[cfg(not(target_os = "windows"))]
type c_long = i32;

use self::asio_import as ai;

#[derive(Debug, Default)]
pub struct Asio {
    loaded_driver: Mutex<Weak<DriverInner>>,
}

#[derive(Clone, Debug)]
pub struct Driver {
    inner: Arc<DriverInner>,
}

#[derive(Debug)]
struct DriverInner {
    state: Mutex<DriverState>,
    name: String,
    destroyed: bool,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DriverState {
    Initialized,
    Prepared,
    Running,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Channels {
    pub ins: c_long,
    pub outs: c_long,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SampleRate {
    pub rate: u32,
}

#[derive(Debug)]
pub struct CallbackInfo {
    pub buffer_index: i32,
    pub system_time: ai::ASIOTimeStamp,
    pub callback_flag: u32,
}

struct BufferCallback(Box<dyn FnMut(&CallbackInfo) + Send>);

pub struct AsioStreams {
    pub input: Option<AsioStream>,
    pub output: Option<AsioStream>,
}

pub struct AsioStream {
    pub buffer_infos: Vec<AsioBufferInfo>,
    pub buffer_size: i32,
}

#[derive(Debug, FromPrimitive)]
#[repr(C)]
pub enum AsioSampleType {
    ASIOSTInt16MSB = 0,
    ASIOSTInt24MSB = 1, // used for 20 bits as well
    ASIOSTInt32MSB = 2,
    ASIOSTFloat32MSB = 3, // IEEE 754 32 bit float
    ASIOSTFloat64MSB = 4, // IEEE 754 64 bit double float

    ASIOSTInt32MSB16 = 8,  // 32 bit data with 16 bit alignment
    ASIOSTInt32MSB18 = 9,  // 32 bit data with 18 bit alignment
    ASIOSTInt32MSB20 = 10, // 32 bit data with 20 bit alignment
    ASIOSTInt32MSB24 = 11, // 32 bit data with 24 bit alignment

    ASIOSTInt16LSB = 16,
    ASIOSTInt24LSB = 17, // used for 20 bits as well
    ASIOSTInt32LSB = 18,
    ASIOSTFloat32LSB = 19, // IEEE 754 32 bit float, as found on Intel x86 architecture
    ASIOSTFloat64LSB = 20, // IEEE 754 64 bit double float, as found on Intel x86 architecture

    ASIOSTInt32LSB16 = 24, // 32 bit data with 18 bit alignment
    ASIOSTInt32LSB18 = 25, // 32 bit data with 18 bit alignment
    ASIOSTInt32LSB20 = 26, // 32 bit data with 20 bit alignment
    ASIOSTInt32LSB24 = 27, // 32 bit data with 24 bit alignment

    ASIOSTDSDInt8LSB1 = 32, // DSD 1 bit data, 8 samples per byte. First sample in Least significant bit.
    ASIOSTDSDInt8MSB1 = 33, // DSD 1 bit data, 8 samples per byte. First sample in Most significant bit.
    ASIOSTDSDInt8NER8 = 40, // DSD 8 bit data, 1 sample per byte. No Endianness required.

    ASIOSTLastEntry,
}

#[derive(Debug, Copy, Clone)]
#[repr(C, packed(4))]
pub struct AsioBufferInfo {
    pub is_input: c_long,
    pub channel_num: c_long,
    pub buffers: [*mut c_void; 2],
}

#[repr(C, packed(4))]
struct AsioCallbacks {
    buffer_switch: extern "C" fn(double_buffer_index: c_long, direct_process: c_long) -> (),
    sample_rate_did_change: extern "C" fn(s_rate: c_double) -> (),
    asio_message: extern "C" fn(
        selector: c_long,
        value: c_long,
        message: *mut (),
        opt: *mut c_double,
    ) -> c_long,
    buffer_switch_time_info: extern "C" fn(
        params: *mut ai::ASIOTime,
        double_buffer_index: c_long,
        direct_process: c_long,
    ) -> *mut ai::ASIOTime,
}

static ASIO_CALLBACKS: AsioCallbacks = AsioCallbacks {
    buffer_switch,
    sample_rate_did_change,
    asio_message,
    buffer_switch_time_info,
};

#[rustfmt::skip]
#[derive(Debug, FromPrimitive)]
#[repr(C)]
pub enum AsioMessageSelectors {
    kAsioSelectorSupported = 1, // selector in <value>, returns 1L if supported,
    kAsioEngineVersion,         // returns engine (host) asio implementation version,
    kAsioResetRequest,          // request driver reset. if accepted, this
    kAsioBufferSizeChange,      // not yet supported, will currently always return 0L.
    kAsioResyncRequest,         // the driver went out of sync, such that
    kAsioLatenciesChanged,      // the drivers latencies have changed. The engine
    kAsioSupportsTimeInfo,      // if host returns true here, it will expect the
    kAsioSupportsTimeCode,      //
    kAsioMMCCommand,            // unused - value: number of commands, message points to mmc commands
    kAsioSupportsInputMonitor,  // kAsioSupportsXXX return 1 if host supports this
    kAsioSupportsInputGain,     // unused and undefined
    kAsioSupportsInputMeter,    // unused and undefined
    kAsioSupportsOutputGain,    // unused and undefined
    kAsioSupportsOutputMeter,   // unused and undefined
    kAsioOverload,              // driver detected an overload

    kAsioNumMessageSelectors
}

#[repr(C, packed(4))]
pub struct AsioTime {
    pub reserved: [c_long; 4],
    pub time_info: AsioTimeInfo,
    pub time_code: AsioTimeCode,
}

#[repr(C, packed(4))]
pub struct AsioTimeInfo {
    pub speed: c_double,
    pub system_time: ai::ASIOTimeStamp,
    pub sample_position: ai::ASIOSamples,
    pub sample_rate: AsioSampleRate,
    pub flags: c_long,
    pub reserved: [c_char; 12],
}

#[repr(C, packed(4))]
pub struct AsioTimeCode {
    pub speed: c_double,
    pub time_code_samples: ai::ASIOSamples,
    pub flags: c_long,
    pub future: [c_char; 64],
}

pub type AsioSampleRate = f64;

#[derive(Default)]
struct BufferSizes {
    min: c_long,
    max: c_long,
    pref: c_long,
    grans: c_long,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallbackId(usize);

static BUFFER_CALLBACK: Mutex<Vec<(CallbackId, BufferCallback)>> = Mutex::new(Vec::new());

static CALLBACK_FLAG: AtomicU32 = AtomicU32::new(0);

static CALL_OUTPUT_READY: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessageCallbackId(usize);

struct MessageCallback(Arc<dyn Fn(AsioMessageSelectors) + Send + Sync>);

static MESSAGE_CALLBACKS: Mutex<Vec<(MessageCallbackId, MessageCallback)>> = Mutex::new(Vec::new());

impl Asio {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn driver_names(&self) -> Vec<String> {
        const MAX_DRIVERS: usize = 100;
        const MAX_DRIVER_NAME_LEN: usize = 32;

        let mut driver_names: [[c_char; MAX_DRIVER_NAME_LEN]; MAX_DRIVERS] =
            [[0; MAX_DRIVER_NAME_LEN]; MAX_DRIVERS];
        let mut driver_name_ptrs: [*mut i8; MAX_DRIVERS] = [null_mut(); MAX_DRIVERS];
        for (ptr, name) in driver_name_ptrs.iter_mut().zip(&mut driver_names[..]) {
            *ptr = (*name).as_mut_ptr();
        }

        unsafe {
            let num_drivers =
                ai::get_driver_names(driver_name_ptrs.as_mut_ptr(), MAX_DRIVERS as i32);
            (0..num_drivers)
                .map(|i| driver_name_to_utf8(&driver_names[i as usize]).to_string())
                .collect()
        }
    }

    pub fn loaded_driver(&self) -> Option<Driver> {
        self.loaded_driver
            .lock()
            .expect("failed to acquire loaded driver lock")
            .upgrade()
            .map(|inner| Driver { inner })
    }

    pub fn load_driver(&self, driver_name: &str) -> Result<Driver, LoadDriverError> {
        if let Some(driver) = self.loaded_driver() {
            if driver.name() == driver_name {
                return Ok(driver);
            } else {
                return Err(LoadDriverError::DriverAlreadyExists);
            }
        }

        let driver_name_cstring =
            CString::new(driver_name).expect("failed to create `CString` from driver name");
        let mut driver_info = std::mem::MaybeUninit::<ai::ASIODriverInfo>::uninit();

        unsafe {
            match ai::load_asio_driver(driver_name_cstring.as_ptr() as *mut i8) {
                false => Err(LoadDriverError::LoadDriverFailed),
                true => {
                    asio_result!(ai::ASIOInit(driver_info.as_mut_ptr()))?;
                    let _driver_info = driver_info.assume_init();
                    let state = Mutex::new(DriverState::Initialized);
                    let name = driver_name.to_string();
                    let destroyed = false;
                    let inner = Arc::new(DriverInner {
                        name,
                        state,
                        destroyed,
                    });
                    *self
                        .loaded_driver
                        .lock()
                        .expect("failed to acquire loaded driver lock") = Arc::downgrade(&inner);
                    let driver = Driver { inner };
                    Ok(driver)
                }
            }
        }
    }
}

impl BufferCallback {
    fn run(&mut self, callback_info: &CallbackInfo) {
        let cb = &mut self.0;
        cb(callback_info);
    }
}

impl Driver {
    pub fn name(&self) -> &str {
        &self.inner.name
    }

    pub fn channels(&self) -> Result<Channels, AsioError> {
        let mut ins: c_long = 0;
        let mut outs: c_long = 0;
        unsafe {
            asio_result!(ai::ASIOGetChannels(&mut ins, &mut outs))?;
        }
        let channel = Channels { ins, outs };
        Ok(channel)
    }

    pub fn buffersize_range(&self) -> Result<(c_long, c_long), AsioError> {
        let buffer_sizes = asio_get_buffer_sizes()?;
        let min = buffer_sizes.min;
        let max = buffer_sizes.max;
        Ok((min, max))
    }

    pub fn sample_rate(&self) -> Result<c_double, AsioError> {
        let mut rate: c_double = 0.0;
        unsafe {
            asio_result!(ai::get_sample_rate(&mut rate))?;
        }
        Ok(rate)
    }

    pub fn can_sample_rate(&self, sample_rate: c_double) -> Result<bool, AsioError> {
        unsafe {
            match asio_result!(ai::can_sample_rate(sample_rate)) {
                Ok(()) => Ok(true),
                Err(AsioError::NoRate) => Ok(false),
                Err(err) => Err(err),
            }
        }
    }

    pub fn set_sample_rate(&self, sample_rate: c_double) -> Result<(), AsioError> {
        unsafe {
            asio_result!(ai::set_sample_rate(sample_rate))?;
        }
        Ok(())
    }

    pub fn input_data_type(&self) -> Result<AsioSampleType, AsioError> {
        stream_data_type(true)
    }

    pub fn output_data_type(&self) -> Result<AsioSampleType, AsioError> {
        stream_data_type(false)
    }

    fn create_buffers(
        &self,
        buffer_infos: &mut [AsioBufferInfo],
        buffer_size: Option<i32>,
    ) -> Result<c_long, AsioError> {
        let num_channels = buffer_infos.len();

        let mut state = self.inner.lock_state();

        let buffer_sizes = asio_get_buffer_sizes()?;
        if buffer_sizes.pref <= 0 {
            panic!(
                "`ASIOGetBufferSize` produced unusable preferred buffer size of {}",
                buffer_sizes.pref,
            );
        }

        let buffer_size = match buffer_size {
            Some(v) => {
                if v <= buffer_sizes.max {
                    v
                } else {
                    return Err(AsioError::InvalidBufferSize);
                }
            }
            None => buffer_sizes.pref,
        };

        CALL_OUTPUT_READY.store(
            asio_result!(unsafe { ai::ASIOOutputReady() }).is_ok(),
            Ordering::Release,
        );

        if let DriverState::Running = *state {
            state.stop()?;
        }
        if let DriverState::Prepared = *state {
            state.dispose_buffers()?;
        }
        unsafe {
            asio_result!(ai::ASIOCreateBuffers(
                buffer_infos.as_mut_ptr() as *mut _,
                num_channels as i32,
                buffer_size,
                &ASIO_CALLBACKS as *const _ as *mut _,
            ))?;
        }
        *state = DriverState::Prepared;

        Ok(buffer_size)
    }

    fn create_streams(
        &self,
        mut input_buffer_infos: Vec<AsioBufferInfo>,
        mut output_buffer_infos: Vec<AsioBufferInfo>,
        buffer_size: Option<i32>,
    ) -> Result<AsioStreams, AsioError> {
        let (input, output) = match (
            input_buffer_infos.is_empty(),
            output_buffer_infos.is_empty(),
        ) {
            (false, false) => {
                let split_point = input_buffer_infos.len();
                let mut all_buffer_infos = input_buffer_infos;
                all_buffer_infos.append(&mut output_buffer_infos);
                let buffer_size = self.create_buffers(&mut all_buffer_infos, buffer_size)?;
                let output_buffer_infos = all_buffer_infos.split_off(split_point);
                let input_buffer_infos = all_buffer_infos;
                let input = Some(AsioStream {
                    buffer_infos: input_buffer_infos,
                    buffer_size,
                });
                let output = Some(AsioStream {
                    buffer_infos: output_buffer_infos,
                    buffer_size,
                });
                (input, output)
            }
            (false, true) => {
                let buffer_size = self.create_buffers(&mut input_buffer_infos, buffer_size)?;
                let input = Some(AsioStream {
                    buffer_infos: input_buffer_infos,
                    buffer_size,
                });
                let output = None;
                (input, output)
            }
            (true, false) => {
                let buffer_size = self.create_buffers(&mut output_buffer_infos, buffer_size)?;
                let input = None;
                let output = Some(AsioStream {
                    buffer_infos: output_buffer_infos,
                    buffer_size,
                });
                (input, output)
            }
            (true, true) => unreachable!("Trying to create streams without preparing"),
        };
        Ok(AsioStreams { input, output })
    }

    pub fn prepare_input_stream(
        &self,
        output: Option<AsioStream>,
        num_channels: usize,
        buffer_size: Option<i32>,
    ) -> Result<AsioStreams, AsioError> {
        let input_buffer_infos = prepare_buffer_infos(true, num_channels);
        let output_buffer_infos = output.map(|output| output.buffer_infos).unwrap_or_default();
        self.create_streams(input_buffer_infos, output_buffer_infos, buffer_size)
    }

    pub fn prepare_output_stream(
        &self,
        input: Option<AsioStream>,
        num_channels: usize,
        buffer_size: Option<i32>,
    ) -> Result<AsioStreams, AsioError> {
        let input_buffer_infos = input.map(|input| input.buffer_infos).unwrap_or_default();
        let output_buffer_infos = prepare_buffer_infos(false, num_channels);
        self.create_streams(input_buffer_infos, output_buffer_infos, buffer_size)
    }

    pub fn dispose_buffers(&self) -> Result<(), AsioError> {
        self.inner.dispose_buffers_inner()
    }

    pub fn start(&self) -> Result<(), AsioError> {
        let mut state = self.inner.lock_state();
        if let DriverState::Running = *state {
            return Ok(());
        }
        unsafe {
            asio_result!(ai::ASIOStart())?;
        }
        *state = DriverState::Running;
        Ok(())
    }

    pub fn stop(&self) -> Result<(), AsioError> {
        self.inner.stop_inner()
    }

    pub fn add_callback<F>(&self, callback: F) -> CallbackId
    where
        F: 'static + FnMut(&CallbackInfo) + Send,
    {
        let mut bc = BUFFER_CALLBACK.lock().unwrap();
        let id = bc
            .last()
            .map(|&(id, _)| CallbackId(id.0.checked_add(1).expect("stream ID overflowed")))
            .unwrap_or(CallbackId(0));
        let cb = BufferCallback(Box::new(callback));
        bc.push((id, cb));
        id
    }

    pub fn remove_callback(&self, rem_id: CallbackId) {
        let mut bc = BUFFER_CALLBACK.lock().unwrap();
        bc.retain(|&(id, _)| id != rem_id);
    }

    pub fn destroy(self) -> Result<bool, AsioError> {
        let Driver { inner } = self;
        match Arc::try_unwrap(inner) {
            Err(_) => Ok(false),
            Ok(mut inner) => {
                inner.destroy_inner()?;
                Ok(true)
            }
        }
    }

    pub fn add_message_callback<F>(&self, callback: F) -> MessageCallbackId
    where
        F: Fn(AsioMessageSelectors) + Send + Sync + 'static,
    {
        let mut mcb = MESSAGE_CALLBACKS.lock().unwrap();
        let id = mcb
            .last()
            .map(|&(id, _)| {
                MessageCallbackId(id.0.checked_add(1).expect("MessageCallbackId overflowed"))
            })
            .unwrap_or(MessageCallbackId(0));

        let cb = MessageCallback(Arc::new(callback));
        mcb.push((id, cb));
        id
    }

    pub fn remove_message_callback(&self, rem_id: MessageCallbackId) {
        let mut mcb = MESSAGE_CALLBACKS.lock().unwrap();
        mcb.retain(|&(id, _)| id != rem_id);
    }
}

impl DriverState {
    fn stop(&mut self) -> Result<(), AsioError> {
        if let DriverState::Running = *self {
            unsafe {
                asio_result!(ai::ASIOStop())?;
            }
            *self = DriverState::Prepared;
        }
        Ok(())
    }

    fn dispose_buffers(&mut self) -> Result<(), AsioError> {
        if let DriverState::Initialized = *self {
            return Ok(());
        }
        if let DriverState::Running = *self {
            self.stop()?;
        }
        unsafe {
            asio_result!(ai::ASIODisposeBuffers())?;
        }
        *self = DriverState::Initialized;
        Ok(())
    }

    fn destroy(&mut self) -> Result<(), AsioError> {
        if let DriverState::Running = *self {
            self.stop()?;
        }
        if let DriverState::Prepared = *self {
            self.dispose_buffers()?;
        }
        unsafe {
            asio_result!(ai::ASIOExit())?;
            ai::remove_current_driver();
        }
        Ok(())
    }
}

impl DriverInner {
    fn lock_state(&self) -> MutexGuard<'_, DriverState> {
        self.state.lock().expect("failed to lock `DriverState`")
    }

    fn stop_inner(&self) -> Result<(), AsioError> {
        let mut state = self.lock_state();
        state.stop()
    }

    fn dispose_buffers_inner(&self) -> Result<(), AsioError> {
        let mut state = self.lock_state();
        state.dispose_buffers()
    }

    fn destroy_inner(&mut self) -> Result<(), AsioError> {
        {
            let mut state = self.lock_state();
            state.destroy()?;

            if let Ok(mut bcs) = BUFFER_CALLBACK.lock() {
                bcs.clear();
            }
        }

        self.destroyed = true;

        Ok(())
    }
}

impl Drop for DriverInner {
    fn drop(&mut self) {
        if !self.destroyed {
            self.destroy_inner().ok();
        }
    }
}

unsafe impl Send for AsioStream {}

fn prepare_buffer_infos(is_input: bool, n_channels: usize) -> Vec<AsioBufferInfo> {
    let is_input = if is_input { 1 } else { 0 };
    (0..n_channels)
        .map(|ch| {
            let channel_num = ch as c_long;
            let buffers = [std::ptr::null_mut(); 2];
            AsioBufferInfo {
                is_input,
                channel_num,
                buffers,
            }
        })
        .collect()
}

fn asio_get_buffer_sizes() -> Result<BufferSizes, AsioError> {
    let mut b = BufferSizes::default();
    unsafe {
        let res = ai::ASIOGetBufferSize(&mut b.min, &mut b.max, &mut b.pref, &mut b.grans);
        asio_result!(res)?;
    }
    Ok(b)
}

fn asio_channel_info(channel: c_long, is_input: bool) -> Result<ai::ASIOChannelInfo, AsioError> {
    let mut channel_info = ai::ASIOChannelInfo {
        channel,
        isInput: if is_input { 1 } else { 0 },
        isActive: 0,
        channelGroup: 0,
        type_: 0,
        name: [0 as c_char; 32],
    };
    unsafe {
        asio_result!(ai::ASIOGetChannelInfo(&mut channel_info))?;
        Ok(channel_info)
    }
}

fn stream_data_type(is_input: bool) -> Result<AsioSampleType, AsioError> {
    let channel_info = asio_channel_info(0, is_input)?;
    Ok(FromPrimitive::from_i32(channel_info.type_).expect("unknown `ASIOSampletype` value"))
}

fn driver_name_to_utf8(bytes: &[c_char]) -> std::borrow::Cow<'_, str> {
    unsafe { CStr::from_ptr(bytes.as_ptr()).to_string_lossy() }
}

fn _channel_name_to_utf8(bytes: &[c_char]) -> std::borrow::Cow<'_, str> {
    unsafe { CStr::from_ptr(bytes.as_ptr()).to_string_lossy() }
}

extern "C" fn sample_rate_did_change(s_rate: c_double) {
    eprintln!("unhandled sample rate change to {}", s_rate);
}

extern "C" fn asio_message(
    selector: c_long,
    value: c_long,
    _message: *mut (),
    _opt: *mut c_double,
) -> c_long {
    match AsioMessageSelectors::from_i64(selector as i64) {
        Some(AsioMessageSelectors::kAsioSelectorSupported) => {
            match AsioMessageSelectors::from_i64(value as i64) {
                | Some(AsioMessageSelectors::kAsioResetRequest)
                | Some(AsioMessageSelectors::kAsioEngineVersion)
                | Some(AsioMessageSelectors::kAsioResyncRequest)
                | Some(AsioMessageSelectors::kAsioLatenciesChanged)
                | Some(AsioMessageSelectors::kAsioSupportsTimeInfo)
                | Some(AsioMessageSelectors::kAsioSupportsTimeCode)
                | Some(AsioMessageSelectors::kAsioSupportsInputMonitor) => 1,
                _ => 0,
            }
        }

        Some(AsioMessageSelectors::kAsioResetRequest) => {

            let callbacks: Vec<_> = {
                let lock = MESSAGE_CALLBACKS.lock().unwrap();
                lock.iter().map(|(_, cb)| cb.0.clone()).collect()
            };
            for cb in callbacks {
                cb(AsioMessageSelectors::kAsioResetRequest);
            }

            1
        }

        Some(AsioMessageSelectors::kAsioResyncRequest) => {
            1
        }

        Some(AsioMessageSelectors::kAsioLatenciesChanged) => {
            1
        }

        Some(AsioMessageSelectors::kAsioEngineVersion) => {
            2
        }

        Some(AsioMessageSelectors::kAsioSupportsTimeInfo) => {
            1
        }

        Some(AsioMessageSelectors::kAsioSupportsTimeCode) => {
            1
        }

        _ => 0, // Unknown/unhandled message type.
    }
}

extern "C" fn buffer_switch_time_info(
    time: *mut ai::ASIOTime,
    double_buffer_index: c_long,
    _direct_process: c_long,
) -> *mut ai::ASIOTime {
    let mut bcs = BUFFER_CALLBACK.lock().unwrap();
    let asio_time: &mut AsioTime = unsafe { &mut *(time as *mut AsioTime) };
    let callback_flag = CALLBACK_FLAG.fetch_xor(1, Ordering::Relaxed);

    let callback_info = CallbackInfo {
        buffer_index: double_buffer_index,
        system_time: asio_time.time_info.system_time,
        callback_flag,
    };
    for &mut (_, ref mut bc) in bcs.iter_mut() {
        bc.run(&callback_info);
    }

    if CALL_OUTPUT_READY.load(Ordering::Acquire) {
        unsafe { ai::ASIOOutputReady() };
    }

    time
}

extern "C" fn buffer_switch(double_buffer_index: c_long, direct_process: c_long) {
    let mut time = unsafe {
        let mut time: AsioTime = std::mem::zeroed();
        let res = ai::ASIOGetSamplePosition(
            &mut time.time_info.sample_position,
            &mut time.time_info.system_time,
        );
        if let Ok(()) = asio_result!(res) {
            time.time_info.flags = (ai::AsioTimeInfoFlags::kSystemTimeValid
                | ai::AsioTimeInfoFlags::kSamplePositionValid)
                .0 as _;
        }
        time
    };

    let asio_time_ptr = &mut time as *mut AsioTime as *mut ai::ASIOTime;
    buffer_switch_time_info(asio_time_ptr, double_buffer_index, direct_process);
}

#[test]
fn check_type_sizes() {
    assert_eq!(
        std::mem::size_of::<AsioSampleRate>(),
        std::mem::size_of::<ai::ASIOSampleRate>()
    );
    assert_eq!(
        std::mem::size_of::<AsioTimeCode>(),
        std::mem::size_of::<ai::ASIOTimeCode>()
    );
    assert_eq!(
        std::mem::size_of::<AsioTimeInfo>(),
        std::mem::size_of::<ai::AsioTimeInfo>(),
    );
    assert_eq!(
        std::mem::size_of::<AsioTime>(),
        std::mem::size_of::<ai::ASIOTime>()
    );
}

pub fn current_latencies() -> Result<(c_long, c_long), AsioError> {
    let (mut input, mut output): (c_long, c_long) = (0, 0);
    unsafe {
        asio_result!(ai::ASIOGetLatencies(&mut input, &mut output))?;
    }
    Ok((input, output))
}

pub fn preferred_buffer_size() -> Result<c_long, AsioError> {
    asio_get_buffer_sizes().map(|sizes| sizes.pref)
}
