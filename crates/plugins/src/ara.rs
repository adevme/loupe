#![allow(non_upper_case_globals)]

use std::ffi::{c_char, c_void};

pub type Ref = *mut c_void;
pub type Bool = i32;
pub type Text = *const c_char;

pub const TRUE: Bool = 1;
pub const FALSE: Bool = 0;

pub const GENERATION_2_0_FINAL: i32 = 4;

pub const PLAYBACK_RENDERER: i32 = 1;
pub const EDITOR_RENDERER: i32 = 2;
pub const EDITOR_VIEW: i32 = 4;
pub const EVERY_ROLE: i32 = PLAYBACK_RENDERER | EDITOR_RENDERER | EDITOR_VIEW;

pub const TIMESTRETCH: i32 = 1;

pub const CONTENT_TEMPO_ENTRIES: i32 = 20;
pub const CONTENT_BAR_SIGNATURES: i32 = 21;
pub const GRADE_APPROVED: i32 = 3;

pub const MAIN_FACTORY_CATEGORY: &str = "ARA Main Factory Class";

pub const IMainFactory: [u8; 16] = iid(0xDB2A1669, 0xFAFD42A5, 0xA82F864F, 0x7B6872EA);
pub const IPlugInEntryPoint: [u8; 16] = iid(0x12814E54, 0xA1CE4076, 0x82B96813, 0x16950BD6);
pub const IPlugInEntryPoint2: [u8; 16] = iid(0xCD9A5913, 0xC9EB46D7, 0x96CA53AD, 0xD1DB89F5);

const fn iid(a: u32, b: u32, c: u32, d: u32) -> [u8; 16] {
    let raw = vst3::uid(a, b, c, d);
    let mut out = [0u8; 16];
    let mut at = 0;
    while at < 16 {
        out[at] = raw[at] as u8;
        at += 1;
    }
    out
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

#[repr(C, packed)]
pub struct DocumentProperties {
    pub struct_size: usize,
    pub name: Text,
}

#[repr(C, packed)]
pub struct MusicalContextProperties {
    pub struct_size: usize,
    pub name: Text,
    pub order_index: i32,
    pub color: *const Color,
}

#[repr(C, packed)]
pub struct RegionSequenceProperties {
    pub struct_size: usize,
    pub name: Text,
    pub order_index: i32,
    pub musical_context: Ref,
    pub color: *const Color,
}

#[repr(C, packed)]
pub struct AudioSourceProperties {
    pub struct_size: usize,
    pub name: Text,
    pub persistent_id: Text,
    pub sample_count: i64,
    pub sample_rate: f64,
    pub channel_count: i32,
    pub merits_64_bit_samples: Bool,
    pub channel_arrangement_data_type: i32,
    pub channel_arrangement: *const c_void,
}

#[repr(C, packed)]
pub struct AudioModificationProperties {
    pub struct_size: usize,
    pub name: Text,
    pub persistent_id: Text,
}

#[repr(C, packed)]
pub struct PlaybackRegionProperties {
    pub struct_size: usize,
    pub transformation_flags: i32,
    pub start_in_modification_time: f64,
    pub duration_in_modification_time: f64,
    pub start_in_playback_time: f64,
    pub duration_in_playback_time: f64,
    pub musical_context: Ref,
    pub region_sequence: Ref,
    pub name: Text,
    pub color: *const Color,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct ContentTimeRange {
    pub start: f64,
    pub duration: f64,
}

#[repr(C, packed)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContentTempoEntry {
    pub time_position: f64,
    pub quarter_position: f64,
}

#[repr(C, packed)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContentBarSignature {
    pub numerator: i32,
    pub denominator: i32,
    pub position: f64,
}

pub type AssertFunction = unsafe extern "C" fn(category: i32, problem: *const c_void, diagnosis: *const c_char);

#[repr(C, packed)]
pub struct AudioAccessControllerInterface {
    pub struct_size: usize,
    pub create_audio_reader_for_source: Option<unsafe extern "C" fn(Ref, Ref, Bool) -> Ref>,
    pub read_audio_samples: Option<unsafe extern "C" fn(Ref, Ref, i64, i64, *const *mut c_void) -> Bool>,
    pub destroy_audio_reader: Option<unsafe extern "C" fn(Ref, Ref)>,
}

#[repr(C, packed)]
pub struct ArchivingControllerInterface {
    pub struct_size: usize,
    pub get_archive_size: Option<unsafe extern "C" fn(Ref, Ref) -> usize>,
    pub read_bytes_from_archive: Option<unsafe extern "C" fn(Ref, Ref, usize, usize, *mut u8) -> Bool>,
    pub write_bytes_to_archive: Option<unsafe extern "C" fn(Ref, Ref, usize, usize, *const u8) -> Bool>,
    pub notify_document_archiving_progress: Option<unsafe extern "C" fn(Ref, f32)>,
    pub notify_document_unarchiving_progress: Option<unsafe extern "C" fn(Ref, f32)>,
    pub get_document_archive_id: Option<unsafe extern "C" fn(Ref, Ref) -> Text>,
}

#[repr(C, packed)]
pub struct ContentAccessControllerInterface {
    pub struct_size: usize,
    pub is_musical_context_content_available: Option<unsafe extern "C" fn(Ref, Ref, i32) -> Bool>,
    pub get_musical_context_content_grade: Option<unsafe extern "C" fn(Ref, Ref, i32) -> i32>,
    pub create_musical_context_content_reader: Option<unsafe extern "C" fn(Ref, Ref, i32, *const ContentTimeRange) -> Ref>,
    pub is_audio_source_content_available: Option<unsafe extern "C" fn(Ref, Ref, i32) -> Bool>,
    pub get_audio_source_content_grade: Option<unsafe extern "C" fn(Ref, Ref, i32) -> i32>,
    pub create_audio_source_content_reader: Option<unsafe extern "C" fn(Ref, Ref, i32, *const ContentTimeRange) -> Ref>,
    pub get_content_reader_event_count: Option<unsafe extern "C" fn(Ref, Ref) -> i32>,
    pub get_content_reader_data_for_event: Option<unsafe extern "C" fn(Ref, Ref, i32) -> *const c_void>,
    pub destroy_content_reader: Option<unsafe extern "C" fn(Ref, Ref)>,
}

#[repr(C, packed)]
pub struct ModelUpdateControllerInterface {
    pub struct_size: usize,
    pub notify_audio_source_analysis_progress: Option<unsafe extern "C" fn(Ref, Ref, i32, f32)>,
    pub notify_audio_source_content_changed: Option<unsafe extern "C" fn(Ref, Ref, *const ContentTimeRange, i32)>,
    pub notify_audio_modification_content_changed: Option<unsafe extern "C" fn(Ref, Ref, *const ContentTimeRange, i32)>,
    pub notify_playback_region_content_changed: Option<unsafe extern "C" fn(Ref, Ref, *const ContentTimeRange, i32)>,
    pub notify_document_data_changed: Option<unsafe extern "C" fn(Ref)>,
}

#[repr(C, packed)]
pub struct PlaybackControllerInterface {
    pub struct_size: usize,
    pub request_start_playback: Option<unsafe extern "C" fn(Ref)>,
    pub request_stop_playback: Option<unsafe extern "C" fn(Ref)>,
    pub request_set_playback_position: Option<unsafe extern "C" fn(Ref, f64)>,
    pub request_set_cycle_range: Option<unsafe extern "C" fn(Ref, f64, f64)>,
    pub request_enable_cycle: Option<unsafe extern "C" fn(Ref, Bool)>,
}

#[repr(C, packed)]
pub struct DocumentControllerHostInstance {
    pub struct_size: usize,
    pub audio_access_controller: Ref,
    pub audio_access_controller_interface: *const AudioAccessControllerInterface,
    pub archiving_controller: Ref,
    pub archiving_controller_interface: *const ArchivingControllerInterface,
    pub content_access_controller: Ref,
    pub content_access_controller_interface: *const ContentAccessControllerInterface,
    pub model_update_controller: Ref,
    pub model_update_controller_interface: *const ModelUpdateControllerInterface,
    pub playback_controller: Ref,
    pub playback_controller_interface: *const PlaybackControllerInterface,
}

#[repr(C, packed)]
pub struct RestoreObjectsFilter {
    pub struct_size: usize,
    pub document_data: Bool,
    pub audio_source_ids_count: usize,
    pub audio_source_archive_ids: *const Text,
    pub audio_source_current_ids: *const Text,
    pub audio_modification_ids_count: usize,
    pub audio_modification_archive_ids: *const Text,
    pub audio_modification_current_ids: *const Text,
}

#[repr(C, packed)]
pub struct StoreObjectsFilter {
    pub struct_size: usize,
    pub document_data: Bool,
    pub audio_source_refs_count: usize,
    pub audio_source_refs: *const Ref,
    pub audio_modification_refs_count: usize,
    pub audio_modification_refs: *const Ref,
}

#[repr(C, packed)]
pub struct DocumentControllerInterface {
    pub struct_size: usize,
    pub destroy_document_controller: Option<unsafe extern "C" fn(Ref)>,
    pub get_factory: Option<unsafe extern "C" fn(Ref) -> *const Factory>,
    pub begin_editing: Option<unsafe extern "C" fn(Ref)>,
    pub end_editing: Option<unsafe extern "C" fn(Ref)>,
    pub notify_model_updates: Option<unsafe extern "C" fn(Ref)>,
    pub begin_restoring_document_from_archive: Option<unsafe extern "C" fn(Ref, Ref) -> Bool>,
    pub end_restoring_document_from_archive: Option<unsafe extern "C" fn(Ref, Ref) -> Bool>,
    pub store_document_to_archive: Option<unsafe extern "C" fn(Ref, Ref) -> Bool>,
    pub update_document_properties: Option<unsafe extern "C" fn(Ref, *const DocumentProperties)>,
    pub create_musical_context: Option<unsafe extern "C" fn(Ref, Ref, *const MusicalContextProperties) -> Ref>,
    pub update_musical_context_properties: Option<unsafe extern "C" fn(Ref, Ref, *const MusicalContextProperties)>,
    pub update_musical_context_content: Option<unsafe extern "C" fn(Ref, Ref, *const ContentTimeRange, i32)>,
    pub destroy_musical_context: Option<unsafe extern "C" fn(Ref, Ref)>,
    pub create_audio_source: Option<unsafe extern "C" fn(Ref, Ref, *const AudioSourceProperties) -> Ref>,
    pub update_audio_source_properties: Option<unsafe extern "C" fn(Ref, Ref, *const AudioSourceProperties)>,
    pub update_audio_source_content: Option<unsafe extern "C" fn(Ref, Ref, *const ContentTimeRange, i32)>,
    pub enable_audio_source_samples_access: Option<unsafe extern "C" fn(Ref, Ref, Bool)>,
    pub deactivate_audio_source_for_undo_history: Option<unsafe extern "C" fn(Ref, Ref, Bool)>,
    pub destroy_audio_source: Option<unsafe extern "C" fn(Ref, Ref)>,
    pub create_audio_modification: Option<unsafe extern "C" fn(Ref, Ref, Ref, *const AudioModificationProperties) -> Ref>,
    pub clone_audio_modification: Option<unsafe extern "C" fn(Ref, Ref, Ref, *const AudioModificationProperties) -> Ref>,
    pub update_audio_modification_properties: Option<unsafe extern "C" fn(Ref, Ref, *const AudioModificationProperties)>,
    pub deactivate_audio_modification_for_undo_history: Option<unsafe extern "C" fn(Ref, Ref, Bool)>,
    pub destroy_audio_modification: Option<unsafe extern "C" fn(Ref, Ref)>,
    pub create_playback_region: Option<unsafe extern "C" fn(Ref, Ref, Ref, *const PlaybackRegionProperties) -> Ref>,
    pub update_playback_region_properties: Option<unsafe extern "C" fn(Ref, Ref, *const PlaybackRegionProperties)>,
    pub destroy_playback_region: Option<unsafe extern "C" fn(Ref, Ref)>,
    pub is_audio_source_content_available: Option<unsafe extern "C" fn(Ref, Ref, i32) -> Bool>,
    pub is_audio_source_content_analysis_incomplete: Option<unsafe extern "C" fn(Ref, Ref, i32) -> Bool>,
    pub request_audio_source_content_analysis: Option<unsafe extern "C" fn(Ref, Ref, usize, *const i32)>,
    pub get_audio_source_content_grade: Option<unsafe extern "C" fn(Ref, Ref, i32) -> i32>,
    pub create_audio_source_content_reader: Option<unsafe extern "C" fn(Ref, Ref, i32, *const ContentTimeRange) -> Ref>,
    pub is_audio_modification_content_available: Option<unsafe extern "C" fn(Ref, Ref, i32) -> Bool>,
    pub get_audio_modification_content_grade: Option<unsafe extern "C" fn(Ref, Ref, i32) -> i32>,
    pub create_audio_modification_content_reader: Option<unsafe extern "C" fn(Ref, Ref, i32, *const ContentTimeRange) -> Ref>,
    pub is_playback_region_content_available: Option<unsafe extern "C" fn(Ref, Ref, i32) -> Bool>,
    pub get_playback_region_content_grade: Option<unsafe extern "C" fn(Ref, Ref, i32) -> i32>,
    pub create_playback_region_content_reader: Option<unsafe extern "C" fn(Ref, Ref, i32, *const ContentTimeRange) -> Ref>,
    pub get_content_reader_event_count: Option<unsafe extern "C" fn(Ref, Ref) -> i32>,
    pub get_content_reader_data_for_event: Option<unsafe extern "C" fn(Ref, Ref, i32) -> *const c_void>,
    pub destroy_content_reader: Option<unsafe extern "C" fn(Ref, Ref)>,
    pub create_region_sequence: Option<unsafe extern "C" fn(Ref, Ref, *const RegionSequenceProperties) -> Ref>,
    pub update_region_sequence_properties: Option<unsafe extern "C" fn(Ref, Ref, *const RegionSequenceProperties)>,
    pub destroy_region_sequence: Option<unsafe extern "C" fn(Ref, Ref)>,
    pub get_playback_region_head_and_tail_time: Option<unsafe extern "C" fn(Ref, Ref, *mut f64, *mut f64)>,
    pub restore_objects_from_archive: Option<unsafe extern "C" fn(Ref, Ref, *const RestoreObjectsFilter) -> Bool>,
    pub store_objects_to_archive: Option<unsafe extern "C" fn(Ref, Ref, *const StoreObjectsFilter) -> Bool>,
    pub get_processing_algorithms_count: Option<unsafe extern "C" fn(Ref) -> i32>,
    pub get_processing_algorithm_properties: Option<unsafe extern "C" fn(Ref, i32) -> *const c_void>,
    pub get_processing_algorithm_for_audio_source: Option<unsafe extern "C" fn(Ref, Ref) -> i32>,
    pub request_processing_algorithm_for_audio_source: Option<unsafe extern "C" fn(Ref, Ref, i32)>,
    pub is_licensed_for_capabilities: Option<unsafe extern "C" fn(Ref, Bool, usize, *const i32, i32) -> Bool>,
    pub store_audio_source_to_audio_file_chunk: Option<unsafe extern "C" fn(Ref, Ref, Ref, *mut Text, *mut Bool) -> Bool>,
    pub is_audio_modification_preserving_audio_source_signal: Option<unsafe extern "C" fn(Ref, Ref) -> Bool>,
}

#[repr(C, packed)]
pub struct DocumentControllerInstance {
    pub struct_size: usize,
    pub document_controller: Ref,
    pub document_controller_interface: *const DocumentControllerInterface,
}

#[repr(C, packed)]
pub struct InterfaceConfiguration {
    pub struct_size: usize,
    pub desired_api_generation: i32,
    pub assert_function_address: *const Option<AssertFunction>,
}

#[repr(C, packed)]
pub struct Factory {
    pub struct_size: usize,
    pub lowest_supported_api_generation: i32,
    pub highest_supported_api_generation: i32,
    pub factory_id: Text,
    pub initialize_ara_with_configuration: Option<unsafe extern "C" fn(*const InterfaceConfiguration)>,
    pub uninitialize_ara: Option<unsafe extern "C" fn()>,
    pub plug_in_name: Text,
    pub manufacturer_name: Text,
    pub information_url: Text,
    pub version: Text,
    pub create_document_controller_with_document:
        Option<unsafe extern "C" fn(*const DocumentControllerHostInstance, *const DocumentProperties) -> *const DocumentControllerInstance>,
    pub document_archive_id: Text,
    pub compatible_document_archive_ids_count: usize,
    pub compatible_document_archive_ids: *const Text,
    pub analyzeable_content_types_count: usize,
    pub analyzeable_content_types: *const i32,
    pub supported_playback_transformation_flags: i32,
    pub supports_storing_audio_file_chunks: Bool,
}

#[repr(C, packed)]
pub struct PlaybackRendererInterface {
    pub struct_size: usize,
    pub add_playback_region: Option<unsafe extern "C" fn(Ref, Ref)>,
    pub remove_playback_region: Option<unsafe extern "C" fn(Ref, Ref)>,
}

#[repr(C, packed)]
pub struct EditorRendererInterface {
    pub struct_size: usize,
    pub add_playback_region: Option<unsafe extern "C" fn(Ref, Ref)>,
    pub remove_playback_region: Option<unsafe extern "C" fn(Ref, Ref)>,
    pub add_region_sequence: Option<unsafe extern "C" fn(Ref, Ref)>,
    pub remove_region_sequence: Option<unsafe extern "C" fn(Ref, Ref)>,
}

#[repr(C, packed)]
pub struct ViewSelection {
    pub struct_size: usize,
    pub playback_region_refs_count: usize,
    pub playback_region_refs: *const Ref,
    pub region_sequence_refs_count: usize,
    pub region_sequence_refs: *const Ref,
    pub time_range: *const ContentTimeRange,
}

#[repr(C, packed)]
pub struct EditorViewInterface {
    pub struct_size: usize,
    pub notify_selection: Option<unsafe extern "C" fn(Ref, *const ViewSelection)>,
    pub notify_hide_region_sequences: Option<unsafe extern "C" fn(Ref, usize, *const Ref)>,
}

#[repr(C, packed)]
pub struct PlugInExtensionInstance {
    pub struct_size: usize,
    pub plug_in_extension: Ref,
    pub plug_in_extension_interface: *const c_void,
    pub playback_renderer: Ref,
    pub playback_renderer_interface: *const PlaybackRendererInterface,
    pub editor_renderer: Ref,
    pub editor_renderer_interface: *const EditorRendererInterface,
    pub editor_view: Ref,
    pub editor_view_interface: *const EditorViewInterface,
}

#[repr(C)]
pub struct EntryPointVtbl {
    pub base: vst3::Steinberg::FUnknownVtbl,
    pub get_factory: unsafe extern "system" fn(this: *mut c_void) -> *const Factory,
    pub bind_to_document_controller: unsafe extern "system" fn(this: *mut c_void, controller: Ref) -> *const PlugInExtensionInstance,
}

#[repr(C)]
pub struct EntryPoint2Vtbl {
    pub base: vst3::Steinberg::FUnknownVtbl,
    pub bind_to_document_controller_with_roles:
        unsafe extern "system" fn(this: *mut c_void, controller: Ref, known: i32, assigned: i32) -> *const PlugInExtensionInstance,
}

#[repr(C)]
pub struct MainFactoryVtbl {
    pub base: vst3::Steinberg::FUnknownVtbl,
    pub get_factory: unsafe extern "system" fn(this: *mut c_void) -> *const Factory,
}

#[repr(C)]
pub struct Object<V> {
    pub vtbl: *const V,
}

pub unsafe fn ask_for<V>(object: *mut vst3::Steinberg::FUnknown, id: &[u8; 16]) -> Option<*mut Object<V>> {
    if object.is_null() {
        return None;
    }
    let mut found: *mut c_void = std::ptr::null_mut();
    let tuid = id.map(|byte| byte as i8);
    let asked = ((*(*object).vtbl).queryInterface)(object, &tuid, &mut found);
    if asked != vst3::Steinberg::kResultOk || found.is_null() {
        return None;
    }
    Some(found as *mut Object<V>)
}

pub unsafe fn let_go<V>(object: *mut Object<V>) {
    let unknown = object as *mut vst3::Steinberg::FUnknown;
    ((*(*unknown).vtbl).release)(unknown);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    #[test]
    fn every_struct_is_the_size_the_ara_headers_make_it() {
        assert_eq!(size_of::<Color>(), 12);
        assert_eq!(size_of::<DocumentProperties>(), 16);
        assert_eq!(size_of::<MusicalContextProperties>(), 28);
        assert_eq!(size_of::<RegionSequenceProperties>(), 36);
        assert_eq!(size_of::<AudioSourceProperties>(), 60);
        assert_eq!(offset_of!(AudioSourceProperties, channel_count), 40);
        assert_eq!(offset_of!(AudioSourceProperties, channel_arrangement), 52);
        assert_eq!(size_of::<AudioModificationProperties>(), 24);
        assert_eq!(size_of::<PlaybackRegionProperties>(), 76);
        assert_eq!(offset_of!(PlaybackRegionProperties, start_in_modification_time), 12);
        assert_eq!(offset_of!(PlaybackRegionProperties, color), 68);
        assert_eq!(size_of::<ContentTimeRange>(), 16);
        assert_eq!(size_of::<ContentTempoEntry>(), 16);
        assert_eq!(size_of::<ContentBarSignature>(), 16);
        assert_eq!(size_of::<AudioAccessControllerInterface>(), 32);
        assert_eq!(size_of::<ArchivingControllerInterface>(), 56);
        assert_eq!(size_of::<ContentAccessControllerInterface>(), 80);
        assert_eq!(size_of::<ModelUpdateControllerInterface>(), 48);
        assert_eq!(size_of::<PlaybackControllerInterface>(), 48);
        assert_eq!(size_of::<DocumentControllerHostInstance>(), 88);
        assert_eq!(size_of::<RestoreObjectsFilter>(), 60);
        assert_eq!(size_of::<StoreObjectsFilter>(), 44);
        assert_eq!(size_of::<DocumentControllerInterface>(), 440);
        assert_eq!(offset_of!(DocumentControllerInterface, create_audio_source), 112);
        assert_eq!(offset_of!(DocumentControllerInterface, create_playback_region), 200);
        assert_eq!(offset_of!(DocumentControllerInterface, create_region_sequence), 336);
        assert_eq!(offset_of!(DocumentControllerInterface, restore_objects_from_archive), 368);
        assert_eq!(offset_of!(DocumentControllerInterface, store_objects_to_archive), 376);
        assert_eq!(size_of::<DocumentControllerInstance>(), 24);
        assert_eq!(size_of::<InterfaceConfiguration>(), 20);
        assert_eq!(size_of::<Factory>(), 128);
        assert_eq!(offset_of!(Factory, create_document_controller_with_document), 72);
        assert_eq!(offset_of!(Factory, document_archive_id), 80);
        assert_eq!(offset_of!(Factory, supported_playback_transformation_flags), 120);
        assert_eq!(size_of::<PlaybackRendererInterface>(), 24);
        assert_eq!(size_of::<EditorRendererInterface>(), 40);
        assert_eq!(size_of::<ViewSelection>(), 48);
        assert_eq!(size_of::<EditorViewInterface>(), 24);
        assert_eq!(size_of::<PlugInExtensionInstance>(), 72);
    }

    #[test]
    fn interface_ids_follow_the_vst3_byte_order() {
        if cfg!(windows) {
            assert_eq!(IPlugInEntryPoint2[..4], [0x13, 0x59, 0x9A, 0xCD]);
            assert_eq!(IPlugInEntryPoint2[4..8], [0xEB, 0xC9, 0xD7, 0x46]);
        } else {
            assert_eq!(IPlugInEntryPoint2[..4], [0xCD, 0x9A, 0x59, 0x13]);
            assert_eq!(IPlugInEntryPoint2[4..8], [0xC9, 0xEB, 0x46, 0xD7]);
        }
        assert_eq!(IPlugInEntryPoint2[8..], [0x96, 0xCA, 0x53, 0xAD, 0xD1, 0xDB, 0x89, 0xF5]);
        assert_ne!(IPlugInEntryPoint, IMainFactory);
    }
}
