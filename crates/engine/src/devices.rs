use std::sync::Mutex;

use cpal::traits::{DeviceTrait, HostTrait};

pub const RATES: [u32; 4] = [44_100, 48_000, 88_200, 96_000];
pub const BUFFERS: [u32; 7] = [32, 64, 128, 256, 512, 1024, 2048];

static DRIVER: Mutex<Option<String>> = Mutex::new(None);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Device {
    pub driver: Option<String>,
    pub output: Option<String>,
    pub rate: Option<u32>,
    pub buffer: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Choices {
    pub rates: Vec<u32>,
    pub buffers: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Running {
    pub driver: String,
    pub output: String,
    pub rate: u32,
    pub buffer: Option<u32>,
}

pub fn drivers() -> Vec<String> {
    cpal::available_hosts().into_iter().map(|id| id.name().to_string()).collect()
}

pub fn default_driver() -> String {
    cpal::default_host().id().name().to_string()
}

pub(crate) fn use_driver(driver: Option<&str>) {
    if let Ok(mut chosen) = DRIVER.lock() {
        *chosen = driver.map(str::to_string);
    }
}

pub(crate) fn host() -> cpal::Host {
    let chosen = DRIVER.lock().ok().and_then(|chosen| chosen.clone());
    host_named(chosen.as_deref())
}

pub(crate) fn host_named(driver: Option<&str>) -> cpal::Host {
    driver
        .and_then(|name| cpal::available_hosts().into_iter().find(|id| id.name() == name))
        .and_then(|id| cpal::host_from_id(id).ok())
        .unwrap_or_else(cpal::default_host)
}

pub fn outputs(driver: Option<&str>) -> Vec<String> {
    if driver == Some("ASIO") {
        return asio_drivers();
    }
    let host = host_named(driver);
    let mut names: Vec<String> = host
        .output_devices()
        .map(|devices| devices.filter_map(|device| device.name().ok()).collect())
        .unwrap_or_default();
    names.dedup();
    names
}

static IN_USE: Mutex<Option<cpal::Device>> = Mutex::new(None);

pub(crate) fn output_named(host: &cpal::Host, name: Option<&str>) -> Option<cpal::Device> {
    let found = match name {
        Some(name) => host
            .output_devices()
            .ok()
            .and_then(|mut devices| devices.find(|device| device.name().is_ok_and(|found| found == name)))
            .or_else(|| host.default_output_device()),
        None => host.default_output_device(),
    };
    if let Ok(mut held) = IN_USE.lock() {
        *held = found.clone();
    }
    found
}

pub(crate) fn in_use() -> Option<cpal::Device> {
    IN_USE.lock().ok().and_then(|held| held.clone())
}

pub fn one_device_both_ways() -> bool {
    DRIVER.lock().ok().and_then(|chosen| chosen.clone()).is_some_and(|name| name == "ASIO")
}

pub fn choices(driver: Option<&str>, output: Option<&str>) -> Choices {
    let host = host_named(driver);
    let Some(device) = output_named(&host, output) else {
        return Choices::default();
    };
    let configs: Vec<cpal::SupportedStreamConfigRange> =
        device.supported_output_configs().map(|configs| configs.collect()).unwrap_or_default();
    let rates = RATES
        .into_iter()
        .filter(|rate| configs.iter().any(|c| (c.min_sample_rate().0..=c.max_sample_rate().0).contains(rate)))
        .collect();
    let buffers = BUFFERS
        .into_iter()
        .filter(|size| {
            configs.iter().any(|c| match c.buffer_size() {
                cpal::SupportedBufferSize::Range { min, max } => (*min..=*max).contains(size),
                cpal::SupportedBufferSize::Unknown => false,
            })
        })
        .collect();
    Choices { rates, buffers }
}

#[cfg(windows)]
fn asio_drivers() -> Vec<String> {
    registry::subkeys("SOFTWARE\\ASIO")
}

#[cfg(not(windows))]
fn asio_drivers() -> Vec<String> {
    Vec::new()
}

#[cfg(windows)]
mod registry {
    const LOCAL_MACHINE: isize = 0x8000_0002u32 as i32 as isize;
    const READ: u32 = 0x20019;

    #[link(name = "advapi32")]
    extern "system" {
        fn RegOpenKeyExW(key: isize, name: *const u16, options: u32, access: u32, opened: *mut isize) -> i32;
        fn RegEnumKeyExW(key: isize, index: u32, name: *mut u16, length: *mut u32, reserved: *mut u32, class: *mut u16, class_length: *mut u32, written: *mut u64) -> i32;
        fn RegCloseKey(key: isize) -> i32;
    }

    pub fn subkeys(path: &str) -> Vec<String> {
        let wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
        let mut names = Vec::new();
        unsafe {
            let mut key = 0isize;
            if RegOpenKeyExW(LOCAL_MACHINE, wide.as_ptr(), 0, READ, &mut key) != 0 {
                return names;
            }
            for index in 0.. {
                let mut buffer = [0u16; 256];
                let mut length = buffer.len() as u32;
                let found = RegEnumKeyExW(key, index, buffer.as_mut_ptr(), &mut length, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut());
                if found != 0 {
                    break;
                }
                names.push(String::from_utf16_lossy(&buffer[..length as usize]));
            }
            RegCloseKey(key);
        }
        names
    }
}

pub fn milliseconds(buffer: u32, rate: u32) -> f32 {
    buffer as f32 * 1000.0 / rate.max(1) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_driver_falls_back_to_the_default() {
        assert_eq!(host_named(Some("No such driver")).id(), cpal::default_host().id());
        assert!(drivers().contains(&default_driver()));
    }

    #[test]
    fn asio_devices_are_listed_without_loading_drivers() {
        let names = outputs(Some("ASIO"));
        eprintln!("ASIO drivers: {names:?}");
        if cfg!(not(windows)) {
            assert!(names.is_empty());
        }
    }

    #[test]
    fn buffer_time_is_worked_out_from_the_rate() {
        assert!((milliseconds(256, 48_000) - 5.333).abs() < 0.01);
        assert_eq!(milliseconds(441, 44_100), 10.0);
    }
}
