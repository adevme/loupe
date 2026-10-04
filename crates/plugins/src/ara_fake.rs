use super::*;
use std::cell::{Cell, RefCell};

thread_local! {
    static SAID: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static HOST: Cell<*const DocumentControllerHostInstance> = const { Cell::new(std::ptr::null()) };
    static SOURCE_HOST: Cell<Ref> = const { Cell::new(std::ptr::null_mut()) };
    static NEXT: Cell<usize> = const { Cell::new(1) };
}

fn said(what: impl Into<String>) {
    SAID.with(|said| said.borrow_mut().push(what.into()));
}

fn heard() -> Vec<String> {
    SAID.with(|said| said.borrow_mut().drain(..).collect())
}

fn handle() -> Ref {
    NEXT.with(|next| {
        let now = next.get();
        next.set(now + 1);
        (now * 16) as Ref
    })
}

fn host() -> &'static DocumentControllerHostInstance {
    HOST.with(|host| unsafe { &*host.get() })
}

unsafe extern "C" fn initialize(configuration: *const InterfaceConfiguration) {
    let generation = (*configuration).desired_api_generation;
    let assert_set = !(*configuration).assert_function_address.is_null();
    said(format!("initialize {generation} {assert_set}"));
}

unsafe extern "C" fn uninitialize() {
    said("uninitialize");
}

unsafe extern "C" fn create_document(host: *const DocumentControllerHostInstance, properties: *const DocumentProperties) -> *const DocumentControllerInstance {
    HOST.with(|held| held.set(host));
    let name = CStr::from_ptr((*properties).name).to_string_lossy().into_owned();
    said(format!("document {name}"));
    Box::into_raw(Box::new(DocumentControllerInstance {
        struct_size: size_of::<DocumentControllerInstance>(),
        document_controller: handle(),
        document_controller_interface: Box::into_raw(Box::new(calls())),
    }))
}

unsafe extern "C" fn destroy_controller(_controller: Ref) {
    said("destroy document");
}

unsafe extern "C" fn begin(_controller: Ref) {
    said("begin");
}

unsafe extern "C" fn end(_controller: Ref) {
    said("end");
}

unsafe extern "C" fn notify(_controller: Ref) {
    said("notify");
}

unsafe extern "C" fn create_context(_controller: Ref, context_host: Ref, _properties: *const MusicalContextProperties) -> Ref {
    let host = host();
    let content = &*host.content_access_controller_interface;
    let available = (content.is_musical_context_content_available.unwrap())(host.content_access_controller, context_host, CONTENT_TEMPO_ENTRIES);
    let reader = (content.create_musical_context_content_reader.unwrap())(host.content_access_controller, context_host, CONTENT_TEMPO_ENTRIES, std::ptr::null());
    let count = (content.get_content_reader_event_count.unwrap())(host.content_access_controller, reader);
    let second = (content.get_content_reader_data_for_event.unwrap())(host.content_access_controller, reader, 1) as *const ContentTempoEntry;
    let beat = (*second).time_position;
    (content.destroy_content_reader.unwrap())(host.content_access_controller, reader);
    said(format!("context tempo {available} {count} {beat}"));
    handle()
}

unsafe extern "C" fn update_context_content(_controller: Ref, _context: Ref, _range: *const ContentTimeRange, _flags: i32) {
    said("tempo changed");
}

unsafe extern "C" fn destroy_object(_controller: Ref, _object: Ref) {
    said("destroy");
}

unsafe extern "C" fn create_sequence(_controller: Ref, _sequence_host: Ref, properties: *const RegionSequenceProperties) -> Ref {
    let context = (*properties).musical_context;
    said(format!("sequence {}", !context.is_null()));
    handle()
}

unsafe extern "C" fn create_source(_controller: Ref, source_host: Ref, properties: *const AudioSourceProperties) -> Ref {
    SOURCE_HOST.with(|held| held.set(source_host));
    let id = CStr::from_ptr((*properties).persistent_id).to_string_lossy().into_owned();
    let (count, rate, channels) = ((*properties).sample_count, (*properties).sample_rate, (*properties).channel_count);
    said(format!("source {id} {count} {rate} {channels}"));
    handle()
}

unsafe extern "C" fn reading(_controller: Ref, _source: Ref, on: Bool) {
    if on == FALSE {
        said("reading off");
        return;
    }
    let host = host();
    let access = &*host.audio_access_controller_interface;
    let source = SOURCE_HOST.with(|held| held.get());
    let narrow = (access.create_audio_reader_for_source.unwrap())(host.audio_access_controller, source, FALSE);
    let mut left = [7.0f32; 4];
    let mut right = [7.0f32; 4];
    let buffers = [left.as_mut_ptr() as *mut c_void, right.as_mut_ptr() as *mut c_void];
    let read = (access.read_audio_samples.unwrap())(host.audio_access_controller, narrow, -1, 4, buffers.as_ptr());
    (access.destroy_audio_reader.unwrap())(host.audio_access_controller, narrow);
    let wide = (access.create_audio_reader_for_source.unwrap())(host.audio_access_controller, source, TRUE);
    let mut both = [[7.0f64; 2]; 2];
    let buffers = [both[0].as_mut_ptr() as *mut c_void, both[1].as_mut_ptr() as *mut c_void];
    (access.read_audio_samples.unwrap())(host.audio_access_controller, wide, 2, 2, buffers.as_ptr());
    (access.destroy_audio_reader.unwrap())(host.audio_access_controller, wide);
    said(format!("reading {read} {left:?} {right:?} {both:?}"));
}

unsafe extern "C" fn create_modification(_controller: Ref, source: Ref, _modification_host: Ref, properties: *const AudioModificationProperties) -> Ref {
    let id = CStr::from_ptr((*properties).persistent_id).to_string_lossy().into_owned();
    said(format!("modification {id} {}", !source.is_null()));
    handle()
}

fn region_text(properties: *const PlaybackRegionProperties) -> String {
    unsafe {
        let flags = (*properties).transformation_flags;
        let (from, long) = ((*properties).start_in_modification_time, (*properties).duration_in_modification_time);
        let (at, lasts) = ((*properties).start_in_playback_time, (*properties).duration_in_playback_time);
        let sequence = !(*properties).region_sequence.is_null();
        format!("{flags} {from} {long} {at} {lasts} {sequence}")
    }
}

unsafe extern "C" fn create_region(_controller: Ref, _modification: Ref, _region_host: Ref, properties: *const PlaybackRegionProperties) -> Ref {
    said(format!("region {}", region_text(properties)));
    handle()
}

unsafe extern "C" fn update_region(_controller: Ref, _region: Ref, properties: *const PlaybackRegionProperties) {
    said(format!("moved {}", region_text(properties)));
}

unsafe extern "C" fn store(_controller: Ref, writer: Ref, _filter: *const StoreObjectsFilter) -> Bool {
    let host = host();
    let archiving = &*host.archiving_controller_interface;
    let write = archiving.write_bytes_to_archive.unwrap();
    let tail = b"moved";
    let head = b"notes ";
    write(host.archiving_controller, writer, 6, tail.len(), tail.as_ptr());
    write(host.archiving_controller, writer, 0, head.len(), head.as_ptr());
    said("stored");
    TRUE
}

unsafe extern "C" fn restore(_controller: Ref, reader: Ref, filter: *const RestoreObjectsFilter) -> Bool {
    let host = host();
    let archiving = &*host.archiving_controller_interface;
    let size = (archiving.get_archive_size.unwrap())(host.archiving_controller, reader);
    let mut bytes = vec![0u8; size];
    let read = (archiving.read_bytes_from_archive.unwrap())(host.archiving_controller, reader, 0, size, bytes.as_mut_ptr());
    let past_end = (archiving.read_bytes_from_archive.unwrap())(host.archiving_controller, reader, size, 1, bytes.as_mut_ptr());
    let id = CStr::from_ptr((archiving.get_document_archive_id.unwrap())(host.archiving_controller, reader)).to_string_lossy().into_owned();
    said(format!("restored {} from {id} {read} {past_end} {}", String::from_utf8_lossy(&bytes), filter.is_null()));
    TRUE
}

unsafe extern "C" fn playback_add(_renderer: Ref, region: Ref) {
    said(format!("playback gets region {}", !region.is_null()));
}

unsafe extern "C" fn playback_remove(_renderer: Ref, _region: Ref) {
    said("playback loses region");
}

unsafe extern "C" fn editor_add(_renderer: Ref, _region: Ref) {
    said("editor gets region");
}

unsafe extern "C" fn editor_remove(_renderer: Ref, _region: Ref) {
    said("editor loses region");
}

unsafe extern "C" fn sequence_change(_renderer: Ref, _sequence: Ref) {}

unsafe extern "C" fn selection(_view: Ref, chosen: *const ViewSelection) {
    let count = (*chosen).playback_region_refs_count;
    said(format!("selected {count}"));
}

fn calls() -> DocumentControllerInterface {
    let mut calls: DocumentControllerInterface = unsafe { std::mem::zeroed() };
    calls.struct_size = size_of::<DocumentControllerInterface>();
    calls.destroy_document_controller = Some(destroy_controller);
    calls.begin_editing = Some(begin);
    calls.end_editing = Some(end);
    calls.notify_model_updates = Some(notify);
    calls.create_musical_context = Some(create_context);
    calls.update_musical_context_content = Some(update_context_content);
    calls.destroy_musical_context = Some(destroy_object);
    calls.create_region_sequence = Some(create_sequence);
    calls.destroy_region_sequence = Some(destroy_object);
    calls.create_audio_source = Some(create_source);
    calls.enable_audio_source_samples_access = Some(reading);
    calls.destroy_audio_source = Some(destroy_object);
    calls.create_audio_modification = Some(create_modification);
    calls.destroy_audio_modification = Some(destroy_object);
    calls.create_playback_region = Some(create_region);
    calls.update_playback_region_properties = Some(update_region);
    calls.destroy_playback_region = Some(destroy_object);
    calls.store_objects_to_archive = Some(store);
    calls.restore_objects_from_archive = Some(restore);
    calls
}

fn factory(highest: i32, stretches: bool) -> *const Factory {
    let leak = |text: &str| CString::new(text).unwrap().into_raw() as Text;
    let older = Box::leak(Box::new([leak("fake.archive.0")]));
    Box::into_raw(Box::new(Factory {
        struct_size: size_of::<Factory>(),
        lowest_supported_api_generation: 3,
        highest_supported_api_generation: highest,
        factory_id: leak("fake.factory"),
        initialize_ara_with_configuration: Some(initialize),
        uninitialize_ara: Some(uninitialize),
        plug_in_name: leak("Fake"),
        manufacturer_name: leak("Loupe"),
        information_url: leak(""),
        version: leak("1"),
        create_document_controller_with_document: Some(create_document),
        document_archive_id: leak("fake.archive.1"),
        compatible_document_archive_ids_count: 1,
        compatible_document_archive_ids: older.as_ptr(),
        analyzeable_content_types_count: 0,
        analyzeable_content_types: std::ptr::null(),
        supported_playback_transformation_flags: if stretches { TIMESTRETCH } else { 0 },
        supports_storing_audio_file_chunks: FALSE,
    }))
}

fn extension() -> *const PlugInExtensionInstance {
    let playback = Box::into_raw(Box::new(PlaybackRendererInterface {
        struct_size: size_of::<PlaybackRendererInterface>(),
        add_playback_region: Some(playback_add),
        remove_playback_region: Some(playback_remove),
    }));
    let editor = Box::into_raw(Box::new(EditorRendererInterface {
        struct_size: size_of::<EditorRendererInterface>(),
        add_playback_region: Some(editor_add),
        remove_playback_region: Some(editor_remove),
        add_region_sequence: Some(sequence_change),
        remove_region_sequence: Some(sequence_change),
    }));
    let view = Box::into_raw(Box::new(EditorViewInterface {
        struct_size: size_of::<EditorViewInterface>(),
        notify_selection: Some(selection),
        notify_hide_region_sequences: None,
    }));
    Box::into_raw(Box::new(PlugInExtensionInstance {
        struct_size: size_of::<PlugInExtensionInstance>(),
        plug_in_extension: std::ptr::null_mut(),
        plug_in_extension_interface: std::ptr::null(),
        playback_renderer: handle(),
        playback_renderer_interface: playback,
        editor_renderer: handle(),
        editor_renderer_interface: editor,
        editor_view: handle(),
        editor_view_interface: view,
    }))
}

fn vocal() -> Region {
    Region { file: "vocal.wav".into(), name: "Vocal".into(), start: 2.0, offset: 0.5, length: 4.0, stretch: 1.0, tempo: 120.0 }
}

fn ramp() -> Samples {
    Samples { rate: 48_000.0, channels: vec![vec![0.1, 0.2, 0.3, 0.4], vec![-0.1, -0.2, -0.3, -0.4]] }
}

fn opened(region: &Region) -> Document {
    unsafe {
        let mut document = Document::open_with(factory(4, true), region, ramp()).expect("the fake plugin makes a document");
        document.attach(extension()).expect("it binds");
        document
    }
}

#[test]
fn the_clip_is_laid_out_and_handed_to_both_renderers() {
    let mut document = opened(&vocal());
    document.start_reading();
    document.select();
    assert_eq!(
        heard(),
        vec![
            "initialize 4 true",
            "document Loupe",
            "begin",
            "context tempo 1 2 0.5",
            "sequence true",
            "source loupe audio source 4 48000 2",
            "modification loupe audio modification true",
            "region 0 0.5 4 2 4 true",
            "end",
            "playback gets region true",
            "editor gets region",
            "reading 1 [0.0, 0.1, 0.2, 0.3] [0.0, -0.1, -0.2, -0.3] [[0.30000001192092896, 0.4000000059604645], [-0.30000001192092896, -0.4000000059604645]]",
            "selected 1",
        ]
    );
    drop(document);
    assert_eq!(
        heard(),
        vec!["reading off", "begin", "destroy", "destroy", "destroy", "destroy", "destroy", "end", "destroy document", "uninitialize"]
    );
}

#[test]
fn edits_survive_being_packed_into_the_plugin_state() {
    let document = opened(&vocal());
    let archive = document.store().expect("it stores");
    assert_eq!(archive, b"notes moved");
    let state = pack(b"plugin settings", &document.archive_id(), &archive);
    drop(document);
    heard();
    let unpacked = unpack(&state).expect("it unpacks");
    assert_eq!(unpacked.settings, b"plugin settings");
    let mut fresh = opened(&vocal());
    heard();
    fresh.restore(unpacked.archive_id, unpacked.archive).expect("it restores");
    assert_eq!(heard(), vec!["begin", "restored notes moved from fake.archive.1 1 0 true", "end"]);
}

#[test]
fn an_older_archive_the_plugin_lists_is_read_and_a_strange_one_is_not() {
    let mut document = opened(&vocal());
    heard();
    assert!(document.restore("fake.archive.0", b"old").is_ok());
    assert_eq!(heard(), vec!["begin", "restored old from fake.archive.0 1 0 true", "end"]);
    assert!(document.restore("someone.else", b"strange").is_err());
    assert!(heard().is_empty());
    assert!(document.restore("someone.else", b"").is_ok());
}

#[test]
fn moving_the_clip_moves_the_region_and_a_new_tempo_is_announced() {
    let mut document = opened(&vocal());
    heard();
    let moved = Region { start: 8.0, tempo: 90.0, stretch: 2.0, ..vocal() };
    document.place(&moved);
    assert_eq!(heard(), vec!["begin", "tempo changed", "moved 1 0.5 2 8 4 true", "end"]);
    assert_eq!(document.placed(), &moved);
}

#[test]
fn a_new_take_swaps_the_audio_under_the_region() {
    let mut document = opened(&vocal());
    document.start_reading();
    heard();
    let other = Region { file: "take two.wav".into(), ..vocal() };
    assert!(!document.same_audio(&other));
    unsafe {
        document.swap_audio(&other, Samples { rate: 44_100.0, channels: vec![vec![0.5; 8]] }).expect("it swaps");
    }
    let said = heard();
    assert_eq!(
        said[..11],
        [
            "playback loses region",
            "editor loses region",
            "reading off",
            "begin",
            "destroy",
            "destroy",
            "destroy",
            "source loupe audio source 8 44100 1",
            "modification loupe audio modification true",
            "region 0 0.5 4 2 4 true",
            "end",
        ]
    );
    assert_eq!(said[11..13], ["playback gets region true", "editor gets region"]);
}

#[test]
fn a_plugin_without_timestretch_plays_the_clip_unstretched() {
    let stretched = Region { stretch: 2.0, ..vocal() };
    assert_eq!(times_of(&stretched, false).duration_in_modification, 4.0);
    assert_eq!(times_of(&stretched, false).flags, 0);
    assert_eq!(times_of(&stretched, true).duration_in_modification, 2.0);
    assert_eq!(times_of(&stretched, true).flags, TIMESTRETCH);
    assert_eq!(times_of(&vocal(), true).flags, 0);
}

#[test]
fn an_ara_one_plugin_is_turned_away_before_anything_starts() {
    let refused = unsafe { Document::open_with(factory(2, false), &vocal(), ramp()) };
    assert!(refused.is_err());
    assert!(heard().is_empty());
    assert!(unsafe { Document::open_with(std::ptr::null(), &vocal(), ramp()) }.is_err());
}

#[test]
fn packing_rejects_anything_that_is_not_whole() {
    let state = pack(b"abc", "id", b"archive");
    assert!(unpack(&state).is_some());
    assert!(unpack(&state[..state.len() - 1]).is_none());
    let mut longer = state.clone();
    longer.push(0);
    assert!(unpack(&longer).is_none());
    assert!(unpack(b"plain plugin settings").is_none());
    assert!(unpack(b"").is_none());
    let empty = pack(b"", "", b"");
    assert_eq!(unpack(&empty), Some(Unpacked { settings: b"", archive_id: "", archive: b"" }));
}

#[test]
fn the_tempo_map_puts_one_beat_where_the_tempo_says() {
    assert_eq!({ tempo_map(120.0)[1].time_position }, 0.5);
    assert_eq!({ tempo_map(0.0)[1].time_position }, 0.5);
    assert_eq!(tempo_map(60.0)[1], ContentTempoEntry { time_position: 1.0, quarter_position: 1.0 });
}
