use std::ffi::{c_char, c_void, CStr, CString};
use std::mem::size_of;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::ara::*;
use crate::ara_audio::Samples;
use crate::wire::Region;

const MARK: &[u8; 8] = b"LoupeARA";
const PACKING: u32 = 1;
const SOURCE_ID: &CStr = c"loupe audio source";
const MODIFICATION_ID: &CStr = c"loupe audio modification";

#[derive(Debug, PartialEq)]
pub struct Unpacked<'a> {
    pub settings: &'a [u8],
    pub archive_id: &'a str,
    pub archive: &'a [u8],
}

pub fn pack(settings: &[u8], archive_id: &str, archive: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(32 + settings.len() + archive_id.len() + archive.len());
    out.extend_from_slice(MARK);
    out.extend_from_slice(&PACKING.to_le_bytes());
    out.extend_from_slice(&(settings.len() as u64).to_le_bytes());
    out.extend_from_slice(settings);
    out.extend_from_slice(&(archive_id.len() as u64).to_le_bytes());
    out.extend_from_slice(archive_id.as_bytes());
    out.extend_from_slice(&(archive.len() as u64).to_le_bytes());
    out.extend_from_slice(archive);
    out
}

pub fn unpack(state: &[u8]) -> Option<Unpacked<'_>> {
    let rest = state.strip_prefix(MARK)?;
    let (packing, rest) = rest.split_first_chunk::<4>()?;
    if u32::from_le_bytes(*packing) != PACKING {
        return None;
    }
    let (settings, rest) = take_counted(rest)?;
    let (archive_id, rest) = take_counted(rest)?;
    let (archive, rest) = take_counted(rest)?;
    if !rest.is_empty() {
        return None;
    }
    Some(Unpacked { settings, archive_id: std::str::from_utf8(archive_id).ok()?, archive })
}

fn take_counted(bytes: &[u8]) -> Option<(&[u8], &[u8])> {
    let (count, rest) = bytes.split_first_chunk::<8>()?;
    let count = usize::try_from(u64::from_le_bytes(*count)).ok()?;
    if count > rest.len() {
        return None;
    }
    Some(rest.split_at(count))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Times {
    pub flags: i32,
    pub start_in_modification: f64,
    pub duration_in_modification: f64,
    pub start_in_playback: f64,
    pub duration_in_playback: f64,
}

pub fn times_of(region: &Region, can_stretch: bool) -> Times {
    let stretched = can_stretch && region.stretch > 0.0 && region.stretch != 1.0;
    let duration_in_modification = if stretched { region.length / region.stretch } else { region.length };
    Times {
        flags: if stretched { TIMESTRETCH } else { 0 },
        start_in_modification: region.offset.max(0.0),
        duration_in_modification: duration_in_modification.max(0.0),
        start_in_playback: region.start,
        duration_in_playback: region.length.max(0.0),
    }
}

pub fn tempo_map(tempo: f64) -> [ContentTempoEntry; 2] {
    let tempo = if tempo > 0.0 { tempo } else { 120.0 };
    [
        ContentTempoEntry { time_position: 0.0, quarter_position: 0.0 },
        ContentTempoEntry { time_position: 60.0 / tempo, quarter_position: 1.0 },
    ]
}

pub fn bars() -> [ContentBarSignature; 1] {
    [ContentBarSignature { numerator: 4, denominator: 4, position: 0.0 }]
}

struct Shelf {
    tempo: AtomicU64,
}

impl Shelf {
    fn tempo(&self) -> f64 {
        f64::from_bits(self.tempo.load(Ordering::Relaxed))
    }
}

struct Reader {
    samples: Arc<Samples>,
    wide: bool,
    scratch: Vec<f32>,
}

enum Content {
    Tempo([ContentTempoEntry; 2]),
    Bars([ContentBarSignature; 1]),
}

impl Content {
    fn count(&self) -> i32 {
        match self {
            Content::Tempo(entries) => entries.len() as i32,
            Content::Bars(entries) => entries.len() as i32,
        }
    }

    fn event(&self, index: i32) -> *const c_void {
        let Ok(at) = usize::try_from(index) else { return std::ptr::null() };
        match self {
            Content::Tempo(entries) => entries.get(at).map_or(std::ptr::null(), |entry| entry as *const _ as *const c_void),
            Content::Bars(entries) => entries.get(at).map_or(std::ptr::null(), |entry| entry as *const _ as *const c_void),
        }
    }
}

struct Reading<'a> {
    bytes: &'a [u8],
    id: CString,
}

unsafe extern "C" fn create_audio_reader(_controller: Ref, source: Ref, wide: Bool) -> Ref {
    if source.is_null() {
        return std::ptr::null_mut();
    }
    let samples = Arc::clone(&*(source as *const Arc<Samples>));
    Box::into_raw(Box::new(Reader { samples, wide: wide != FALSE, scratch: Vec::new() })) as Ref
}

unsafe extern "C" fn read_audio_samples(_controller: Ref, reader: Ref, position: i64, count: i64, buffers: *const *mut c_void) -> Bool {
    if reader.is_null() || buffers.is_null() || count < 0 {
        return FALSE;
    }
    let reader = &mut *(reader as *mut Reader);
    let count = count as usize;
    for channel in 0..reader.samples.channels.len() {
        let target = *buffers.add(channel);
        if target.is_null() {
            return FALSE;
        }
        if reader.wide {
            reader.scratch.resize(count, 0.0);
            reader.samples.copy_into(channel, position, &mut reader.scratch);
            let out = std::slice::from_raw_parts_mut(target as *mut f64, count);
            for (into, from) in out.iter_mut().zip(&reader.scratch) {
                *into = *from as f64;
            }
        } else {
            let out = std::slice::from_raw_parts_mut(target as *mut f32, count);
            reader.samples.copy_into(channel, position, out);
        }
    }
    TRUE
}

unsafe extern "C" fn destroy_audio_reader(_controller: Ref, reader: Ref) {
    if !reader.is_null() {
        drop(Box::from_raw(reader as *mut Reader));
    }
}

unsafe extern "C" fn get_archive_size(_controller: Ref, reader: Ref) -> usize {
    if reader.is_null() {
        return 0;
    }
    let reading = &*(reader as *const Reading);
    reading.bytes.len()
}

unsafe extern "C" fn read_bytes_from_archive(_controller: Ref, reader: Ref, position: usize, length: usize, buffer: *mut u8) -> Bool {
    if reader.is_null() || buffer.is_null() {
        return FALSE;
    }
    let bytes = (*(reader as *const Reading)).bytes;
    let Some(end) = position.checked_add(length) else { return FALSE };
    let Some(wanted) = bytes.get(position..end) else { return FALSE };
    std::ptr::copy_nonoverlapping(wanted.as_ptr(), buffer, length);
    TRUE
}

unsafe extern "C" fn write_bytes_to_archive(_controller: Ref, writer: Ref, position: usize, length: usize, buffer: *const u8) -> Bool {
    if writer.is_null() || (buffer.is_null() && length > 0) {
        return FALSE;
    }
    let out = &mut *(writer as *mut Vec<u8>);
    let Some(end) = position.checked_add(length) else { return FALSE };
    if out.len() < end {
        out.resize(end, 0);
    }
    if length > 0 {
        std::ptr::copy_nonoverlapping(buffer, out[position..end].as_mut_ptr(), length);
    }
    TRUE
}

unsafe extern "C" fn archiving_progress(_controller: Ref, _value: f32) {}

unsafe extern "C" fn get_document_archive_id(_controller: Ref, reader: Ref) -> Text {
    if reader.is_null() {
        return std::ptr::null();
    }
    (*(reader as *const Reading)).id.as_ptr()
}

unsafe extern "C" fn musical_content_available(_controller: Ref, _context: Ref, kind: i32) -> Bool {
    if kind == CONTENT_TEMPO_ENTRIES || kind == CONTENT_BAR_SIGNATURES {
        TRUE
    } else {
        FALSE
    }
}

unsafe extern "C" fn musical_content_grade(_controller: Ref, _context: Ref, _kind: i32) -> i32 {
    GRADE_APPROVED
}

unsafe extern "C" fn create_musical_content_reader(_controller: Ref, context: Ref, kind: i32, _range: *const ContentTimeRange) -> Ref {
    if context.is_null() {
        return std::ptr::null_mut();
    }
    let shelf = &*(context as *const Shelf);
    let content = match kind {
        CONTENT_TEMPO_ENTRIES => Content::Tempo(tempo_map(shelf.tempo())),
        CONTENT_BAR_SIGNATURES => Content::Bars(bars()),
        _ => return std::ptr::null_mut(),
    };
    Box::into_raw(Box::new(content)) as Ref
}

unsafe extern "C" fn source_content_available(_controller: Ref, _source: Ref, _kind: i32) -> Bool {
    FALSE
}

unsafe extern "C" fn source_content_grade(_controller: Ref, _source: Ref, _kind: i32) -> i32 {
    0
}

unsafe extern "C" fn create_source_content_reader(_controller: Ref, _source: Ref, _kind: i32, _range: *const ContentTimeRange) -> Ref {
    std::ptr::null_mut()
}

unsafe extern "C" fn content_event_count(_controller: Ref, reader: Ref) -> i32 {
    if reader.is_null() {
        return 0;
    }
    (*(reader as *const Content)).count()
}

unsafe extern "C" fn content_event(_controller: Ref, reader: Ref, index: i32) -> *const c_void {
    if reader.is_null() {
        return std::ptr::null();
    }
    (*(reader as *const Content)).event(index)
}

unsafe extern "C" fn destroy_content_reader(_controller: Ref, reader: Ref) {
    if !reader.is_null() {
        drop(Box::from_raw(reader as *mut Content));
    }
}

unsafe extern "C" fn analysis_progress(_controller: Ref, _source: Ref, _state: i32, _value: f32) {}

unsafe extern "C" fn content_changed(_controller: Ref, _object: Ref, _range: *const ContentTimeRange, _flags: i32) {}

unsafe extern "C" fn document_data_changed(_controller: Ref) {}

unsafe extern "C" fn transport_request(_controller: Ref) {}

unsafe extern "C" fn position_request(_controller: Ref, _at: f64) {}

unsafe extern "C" fn cycle_request(_controller: Ref, _start: f64, _length: f64) {}

unsafe extern "C" fn cycle_switch(_controller: Ref, _on: Bool) {}

unsafe extern "C" fn report_assert(category: i32, _problem: *const c_void, diagnosis: *const c_char) {
    let said = if diagnosis.is_null() { String::new() } else { CStr::from_ptr(diagnosis).to_string_lossy().into_owned() };
    eprintln!("ARA assert {category}: {said}");
}

static mut ASSERT: Option<AssertFunction> = Some(report_assert);

static AUDIO_ACCESS: AudioAccessControllerInterface = AudioAccessControllerInterface {
    struct_size: size_of::<AudioAccessControllerInterface>(),
    create_audio_reader_for_source: Some(create_audio_reader),
    read_audio_samples: Some(read_audio_samples),
    destroy_audio_reader: Some(destroy_audio_reader),
};

static ARCHIVING: ArchivingControllerInterface = ArchivingControllerInterface {
    struct_size: size_of::<ArchivingControllerInterface>(),
    get_archive_size: Some(get_archive_size),
    read_bytes_from_archive: Some(read_bytes_from_archive),
    write_bytes_to_archive: Some(write_bytes_to_archive),
    notify_document_archiving_progress: Some(archiving_progress),
    notify_document_unarchiving_progress: Some(archiving_progress),
    get_document_archive_id: Some(get_document_archive_id),
};

static CONTENT_ACCESS: ContentAccessControllerInterface = ContentAccessControllerInterface {
    struct_size: size_of::<ContentAccessControllerInterface>(),
    is_musical_context_content_available: Some(musical_content_available),
    get_musical_context_content_grade: Some(musical_content_grade),
    create_musical_context_content_reader: Some(create_musical_content_reader),
    is_audio_source_content_available: Some(source_content_available),
    get_audio_source_content_grade: Some(source_content_grade),
    create_audio_source_content_reader: Some(create_source_content_reader),
    get_content_reader_event_count: Some(content_event_count),
    get_content_reader_data_for_event: Some(content_event),
    destroy_content_reader: Some(destroy_content_reader),
};

static MODEL_UPDATES: ModelUpdateControllerInterface = ModelUpdateControllerInterface {
    struct_size: size_of::<ModelUpdateControllerInterface>(),
    notify_audio_source_analysis_progress: Some(analysis_progress),
    notify_audio_source_content_changed: Some(content_changed),
    notify_audio_modification_content_changed: Some(content_changed),
    notify_playback_region_content_changed: Some(content_changed),
    notify_document_data_changed: Some(document_data_changed),
};

static PLAYBACK: PlaybackControllerInterface = PlaybackControllerInterface {
    struct_size: size_of::<PlaybackControllerInterface>(),
    request_start_playback: Some(transport_request),
    request_stop_playback: Some(transport_request),
    request_set_playback_position: Some(position_request),
    request_set_cycle_range: Some(cycle_request),
    request_enable_cycle: Some(cycle_switch),
};

pub struct Document {
    factory: *const Factory,
    controller: Ref,
    calls: *const DocumentControllerInterface,
    _host: Box<DocumentControllerHostInstance>,
    shelf: Box<Shelf>,
    source_host: Box<Arc<Samples>>,
    context: Ref,
    sequence: Ref,
    source: Ref,
    modification: Ref,
    region: Ref,
    extension: *const PlugInExtensionInstance,
    placed: Region,
    can_stretch: bool,
    reading: bool,
}

unsafe impl Send for Document {}

impl Document {
    pub unsafe fn open(factory: *const Factory, region: &Region) -> Result<Self, String> {
        check(factory)?;
        let samples = Samples::read(Path::new(&region.file)).map_err(|why| format!("ARA needs the clip's audio file: {why}"))?;
        Self::open_with(factory, region, samples)
    }

    pub unsafe fn open_with(factory: *const Factory, region: &Region, samples: Samples) -> Result<Self, String> {
        check(factory)?;
        let lowest = (*factory).lowest_supported_api_generation;
        let configuration = InterfaceConfiguration {
            struct_size: size_of::<InterfaceConfiguration>(),
            desired_api_generation: GENERATION_2_0_FINAL.max(lowest),
            assert_function_address: std::ptr::addr_of_mut!(ASSERT),
        };
        let initialize = (*factory).initialize_ara_with_configuration.ok_or("the ARA plugin cannot be started")?;
        initialize(&configuration);
        let shelf = Box::new(Shelf { tempo: AtomicU64::new(region.tempo.to_bits()) });
        let shelf_ref = &*shelf as *const Shelf as Ref;
        let host = Box::new(DocumentControllerHostInstance {
            struct_size: size_of::<DocumentControllerHostInstance>(),
            audio_access_controller: shelf_ref,
            audio_access_controller_interface: &AUDIO_ACCESS,
            archiving_controller: shelf_ref,
            archiving_controller_interface: &ARCHIVING,
            content_access_controller: shelf_ref,
            content_access_controller_interface: &CONTENT_ACCESS,
            model_update_controller: shelf_ref,
            model_update_controller_interface: &MODEL_UPDATES,
            playback_controller: shelf_ref,
            playback_controller_interface: &PLAYBACK,
        });
        let name = CString::new("Loupe").unwrap_or_default();
        let properties = DocumentProperties { struct_size: size_of::<DocumentProperties>(), name: name.as_ptr() };
        let create = (*factory).create_document_controller_with_document;
        let made = match create {
            Some(create) => create(&*host, &properties),
            None => std::ptr::null(),
        };
        if made.is_null() || (*made).document_controller_interface.is_null() {
            if let Some(stop) = (*factory).uninitialize_ara {
                stop();
            }
            return Err("the ARA plugin would not make a document".into());
        }
        let can_stretch = (*factory).supported_playback_transformation_flags & TIMESTRETCH != 0;
        let mut document = Self {
            factory,
            controller: (*made).document_controller,
            calls: (*made).document_controller_interface,
            _host: host,
            shelf,
            source_host: Box::new(Arc::new(samples)),
            context: std::ptr::null_mut(),
            sequence: std::ptr::null_mut(),
            source: std::ptr::null_mut(),
            modification: std::ptr::null_mut(),
            region: std::ptr::null_mut(),
            extension: std::ptr::null(),
            placed: region.clone(),
            can_stretch,
            reading: false,
        };
        document.build()?;
        Ok(document)
    }

    unsafe fn build(&mut self) -> Result<(), String> {
        let calls = &*self.calls;
        let track = CString::new("Loupe").unwrap_or_default();
        self.begin();
        if let Some(create) = calls.create_musical_context {
            let properties = MusicalContextProperties {
                struct_size: size_of::<MusicalContextProperties>(),
                name: track.as_ptr(),
                order_index: 0,
                color: std::ptr::null(),
            };
            self.context = create(self.controller, &*self.shelf as *const Shelf as Ref, &properties);
        }
        if let Some(create) = calls.create_region_sequence {
            let properties = RegionSequenceProperties {
                struct_size: size_of::<RegionSequenceProperties>(),
                name: track.as_ptr(),
                order_index: 0,
                musical_context: self.context,
                color: std::ptr::null(),
            };
            self.sequence = create(self.controller, &*self.shelf as *const Shelf as Ref, &properties);
        }
        let made = self.make_source();
        self.end();
        if self.context.is_null() || self.sequence.is_null() {
            return Err("the ARA plugin would not make a place for the clip".into());
        }
        made
    }

    unsafe fn make_source(&mut self) -> Result<(), String> {
        let calls = &*self.calls;
        let name = CString::new(self.placed.name.clone()).unwrap_or_default();
        let samples: &Arc<Samples> = &self.source_host;
        let source_properties = AudioSourceProperties {
            struct_size: size_of::<AudioSourceProperties>(),
            name: name.as_ptr(),
            persistent_id: SOURCE_ID.as_ptr(),
            sample_count: samples.frames() as i64,
            sample_rate: samples.rate,
            channel_count: samples.channels.len() as i32,
            merits_64_bit_samples: FALSE,
            channel_arrangement_data_type: 0,
            channel_arrangement: std::ptr::null(),
        };
        let source_host = &*self.source_host as *const Arc<Samples> as Ref;
        if let Some(create) = calls.create_audio_source {
            self.source = create(self.controller, source_host, &source_properties);
        }
        if self.source.is_null() {
            return Err("the ARA plugin would not take the clip's audio".into());
        }
        let modification_properties = AudioModificationProperties {
            struct_size: size_of::<AudioModificationProperties>(),
            name: name.as_ptr(),
            persistent_id: MODIFICATION_ID.as_ptr(),
        };
        if let Some(create) = calls.create_audio_modification {
            self.modification = create(self.controller, self.source, source_host, &modification_properties);
        }
        if self.modification.is_null() {
            return Err("the ARA plugin would not make an edit of the clip".into());
        }
        let properties = self.region_properties(&name);
        if let Some(create) = calls.create_playback_region {
            self.region = create(self.controller, self.modification, source_host, &properties);
        }
        if self.region.is_null() {
            return Err("the ARA plugin would not place the clip".into());
        }
        Ok(())
    }

    fn region_properties(&self, name: &CString) -> PlaybackRegionProperties {
        let times = times_of(&self.placed, self.can_stretch);
        PlaybackRegionProperties {
            struct_size: size_of::<PlaybackRegionProperties>(),
            transformation_flags: times.flags,
            start_in_modification_time: times.start_in_modification,
            duration_in_modification_time: times.duration_in_modification,
            start_in_playback_time: times.start_in_playback,
            duration_in_playback_time: times.duration_in_playback,
            musical_context: self.context,
            region_sequence: self.sequence,
            name: name.as_ptr(),
            color: std::ptr::null(),
        }
    }

    pub fn controller(&self) -> Ref {
        self.controller
    }

    pub fn archive_id(&self) -> String {
        unsafe {
            let id = (*self.factory).document_archive_id;
            if id.is_null() {
                return String::new();
            }
            CStr::from_ptr(id).to_string_lossy().into_owned()
        }
    }

    pub fn reads_archive(&self, id: &str) -> bool {
        if id == self.archive_id() {
            return true;
        }
        unsafe {
            let count = (*self.factory).compatible_document_archive_ids_count;
            let ids = (*self.factory).compatible_document_archive_ids;
            if ids.is_null() {
                return false;
            }
            (0..count).any(|at| {
                let one = *ids.add(at);
                !one.is_null() && CStr::from_ptr(one).to_bytes() == id.as_bytes()
            })
        }
    }

    pub fn placed(&self) -> &Region {
        &self.placed
    }

    pub fn same_audio(&self, region: &Region) -> bool {
        self.placed.file == region.file
    }

    pub unsafe fn attach(&mut self, extension: *const PlugInExtensionInstance) -> Result<(), String> {
        if extension.is_null() {
            return Err("the plugin would not join the ARA document".into());
        }
        self.extension = extension;
        self.add_to_renderers();
        Ok(())
    }

    unsafe fn add_to_renderers(&self) {
        if self.extension.is_null() || self.region.is_null() {
            return;
        }
        let extension = &*self.extension;
        let playback = extension.playback_renderer_interface;
        if !playback.is_null() {
            if let Some(add) = (*playback).add_playback_region {
                add(extension.playback_renderer, self.region);
            }
        }
        let editor = extension.editor_renderer_interface;
        if !editor.is_null() {
            if let Some(add) = (*editor).add_playback_region {
                add(extension.editor_renderer, self.region);
            }
        }
    }

    unsafe fn remove_from_renderers(&self) {
        if self.extension.is_null() || self.region.is_null() {
            return;
        }
        let extension = &*self.extension;
        let playback = extension.playback_renderer_interface;
        if !playback.is_null() {
            if let Some(remove) = (*playback).remove_playback_region {
                remove(extension.playback_renderer, self.region);
            }
        }
        let editor = extension.editor_renderer_interface;
        if !editor.is_null() {
            if let Some(remove) = (*editor).remove_playback_region {
                remove(extension.editor_renderer, self.region);
            }
        }
    }

    pub fn start_reading(&mut self) {
        if self.reading || self.source.is_null() {
            return;
        }
        self.reading = true;
        unsafe {
            if let Some(enable) = (*self.calls).enable_audio_source_samples_access {
                enable(self.controller, self.source, TRUE);
            }
        }
    }

    pub fn place(&mut self, region: &Region) {
        let tempo_changed = region.tempo != self.placed.tempo;
        self.placed = region.clone();
        self.shelf.tempo.store(region.tempo.to_bits(), Ordering::Relaxed);
        let name = CString::new(region.name.clone()).unwrap_or_default();
        let properties = self.region_properties(&name);
        unsafe {
            self.begin();
            if tempo_changed && !self.context.is_null() {
                if let Some(update) = (*self.calls).update_musical_context_content {
                    update(self.controller, self.context, std::ptr::null(), 0);
                }
            }
            if let Some(update) = (*self.calls).update_playback_region_properties {
                update(self.controller, self.region, &properties);
            }
            self.end();
        }
    }

    pub unsafe fn swap_audio(&mut self, region: &Region, samples: Samples) -> Result<(), String> {
        self.remove_from_renderers();
        self.stop_reading();
        self.begin();
        self.destroy_source();
        self.placed = region.clone();
        self.shelf.tempo.store(region.tempo.to_bits(), Ordering::Relaxed);
        self.source_host = Box::new(Arc::new(samples));
        let made = self.make_source();
        self.end();
        made?;
        self.add_to_renderers();
        self.start_reading();
        Ok(())
    }

    fn stop_reading(&mut self) {
        if !self.reading || self.source.is_null() {
            return;
        }
        self.reading = false;
        unsafe {
            if let Some(enable) = (*self.calls).enable_audio_source_samples_access {
                enable(self.controller, self.source, FALSE);
            }
        }
    }

    unsafe fn destroy_source(&mut self) {
        let calls = &*self.calls;
        if !self.region.is_null() {
            if let Some(destroy) = calls.destroy_playback_region {
                destroy(self.controller, self.region);
            }
            self.region = std::ptr::null_mut();
        }
        if !self.modification.is_null() {
            if let Some(destroy) = calls.destroy_audio_modification {
                destroy(self.controller, self.modification);
            }
            self.modification = std::ptr::null_mut();
        }
        if !self.source.is_null() {
            if let Some(destroy) = calls.destroy_audio_source {
                destroy(self.controller, self.source);
            }
            self.source = std::ptr::null_mut();
        }
    }

    pub fn store(&self) -> Result<Vec<u8>, String> {
        let mut written: Vec<u8> = Vec::new();
        unsafe {
            let store = (*self.calls).store_objects_to_archive.ok_or("the ARA plugin cannot save its edits")?;
            if store(self.controller, &mut written as *mut Vec<u8> as Ref, std::ptr::null()) == FALSE {
                return Err("the ARA plugin would not save its edits".into());
            }
        }
        Ok(written)
    }

    pub fn restore(&mut self, id: &str, archive: &[u8]) -> Result<(), String> {
        if archive.is_empty() {
            return Ok(());
        }
        if !self.reads_archive(id) {
            return Err(format!("these edits were saved by a version of the plugin this one cannot read ({id})"));
        }
        let reading = Reading { bytes: archive, id: CString::new(id).unwrap_or_default() };
        unsafe {
            let restore = (*self.calls).restore_objects_from_archive.ok_or("the ARA plugin cannot load its edits")?;
            self.begin();
            let done = restore(self.controller, &reading as *const Reading as Ref, std::ptr::null());
            self.end();
            if done == FALSE {
                return Err("the ARA plugin could not read back all of its edits".into());
            }
        }
        Ok(())
    }

    pub fn idle(&self) {
        unsafe {
            if let Some(notify) = (*self.calls).notify_model_updates {
                notify(self.controller);
            }
        }
    }

    pub fn select(&self) {
        if self.extension.is_null() || self.region.is_null() {
            return;
        }
        unsafe {
            let extension = &*self.extension;
            let view = extension.editor_view_interface;
            if view.is_null() {
                return;
            }
            let Some(notify) = (*view).notify_selection else { return };
            let regions = [self.region];
            let selection = ViewSelection {
                struct_size: size_of::<ViewSelection>(),
                playback_region_refs_count: 1,
                playback_region_refs: regions.as_ptr(),
                region_sequence_refs_count: 0,
                region_sequence_refs: std::ptr::null(),
                time_range: std::ptr::null(),
            };
            notify(extension.editor_view, &selection);
        }
    }

    unsafe fn begin(&self) {
        if let Some(begin) = (*self.calls).begin_editing {
            begin(self.controller);
        }
    }

    unsafe fn end(&self) {
        if let Some(end) = (*self.calls).end_editing {
            end(self.controller);
        }
    }
}

impl Drop for Document {
    fn drop(&mut self) {
        unsafe {
            self.stop_reading();
            self.begin();
            self.destroy_source();
            let calls = &*self.calls;
            if !self.sequence.is_null() {
                if let Some(destroy) = calls.destroy_region_sequence {
                    destroy(self.controller, self.sequence);
                }
            }
            if !self.context.is_null() {
                if let Some(destroy) = calls.destroy_musical_context {
                    destroy(self.controller, self.context);
                }
            }
            self.end();
            if let Some(destroy) = calls.destroy_document_controller {
                destroy(self.controller);
            }
            if let Some(stop) = (*self.factory).uninitialize_ara {
                stop();
            }
        }
    }
}

unsafe fn check(factory: *const Factory) -> Result<(), String> {
    if factory.is_null() {
        return Err("the plugin says it is ARA but gave no ARA factory".into());
    }
    if (*factory).highest_supported_api_generation < GENERATION_2_0_FINAL {
        return Err("this plugin only speaks ARA 1, and Loupe needs ARA 2".into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "ara_fake.rs"]
mod tests;
