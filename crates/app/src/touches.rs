use std::time::{Duration, Instant};

use loupe_engine::{Chains, Command, Engine, Outcome, Project, Touched};

use crate::App;

pub const QUIET_FOR: Duration = Duration::from_millis(700);

#[derive(Default)]
pub struct Touches {
    spots: Vec<Touched>,
    last: Option<Instant>,
    quiet: bool,
}

impl Touches {
    pub fn noticed(&mut self, found: &[Touched]) {
        self.quiet = found.is_empty();
        if self.quiet {
            return;
        }
        for spot in found {
            if !self.spots.contains(spot) {
                self.spots.push(*spot);
            }
        }
        self.last = Some(Instant::now());
    }

    pub fn waiting(&self) -> bool {
        !self.spots.is_empty()
    }

    pub fn due(&self) -> bool {
        self.waiting() && self.quiet && self.last.is_some_and(|at| at.elapsed() >= QUIET_FOR)
    }

    pub fn take(&mut self) -> Vec<Touched> {
        self.last = None;
        self.quiet = false;
        std::mem::take(&mut self.spots)
    }

    pub fn forget(&mut self) {
        let _ = self.take();
    }
}

fn state_in(project: &Project, which: Touched) -> Option<&Vec<u8>> {
    match which {
        Touched::Track(track, slot) => project.tracks.iter().find(|found| found.id == track)?.fx.get(slot).map(|fx| &fx.state),
        Touched::Clip(clip, slot) => {
            project.tracks.iter().flat_map(|track| track.clips.iter()).find(|found| found.id == clip)?.fx.get(slot).map(|fx| &fx.state)
        }
        Touched::Master(slot) => project.master_fx.get(slot).map(|fx| &fx.state),
        Touched::Monitor(slot) => project.monitor_fx.get(slot).map(|fx| &fx.state),
    }
}

fn change_for(which: Touched, state: Vec<u8>) -> Command {
    match which {
        Touched::Track(track, slot) => Command::SetFxState { track, slot, state },
        Touched::Clip(clip, slot) => Command::SetClipFxState { clip, slot, state },
        Touched::Master(slot) => Command::SetMasterFxState { slot, state },
        Touched::Monitor(slot) => Command::SetMonitorFxState { slot, state },
    }
}

pub fn racks_must_stay(engine: &Engine) -> bool {
    engine.is_playing() || engine.is_hearing()
}

pub fn edits(racks: &mut dyn Chains, project: &Project, spots: &[Touched]) -> Vec<Command> {
    let mut changes = Vec::new();
    for which in spots {
        let Some(already) = state_in(project, *which) else { continue };
        let Some(state) = racks.state_of(*which) else { continue };
        if state.is_empty() || &state == already {
            continue;
        }
        changes.push(change_for(*which, state));
    }
    changes
}

impl App {
    pub(crate) fn look_for_touches(&mut self) {
        if racks_must_stay(&self.engine) {
            return;
        }
        let Some(mut racks) = self.borrow_racks() else { return };
        let found = racks.plugins_touched();
        self.racks.keep(racks);
        self.hand_racks_over();
        if !found.is_empty() {
            self.dirty = true;
        }
        self.touches.noticed(&found);
        if self.touches.due() {
            self.keep_plugin_edits();
        }
    }

    pub(crate) fn keep_plugin_edits(&mut self) {
        if racks_must_stay(&self.engine) {
            return;
        }
        let spots = self.touches.take();
        if spots.is_empty() {
            return;
        }
        let Some(mut racks) = self.borrow_racks() else { return };
        let changes = edits(racks.as_mut(), &self.project, &spots);
        self.racks.keep(racks);
        self.hand_racks_over();
        if changes.is_empty() {
            return;
        }
        self.transact(None, |project| {
            for change in changes {
                project.apply(change)?;
            }
            Ok(Outcome::Done)
        });
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use loupe_engine::{Fx, Source, TrackId};

    use super::*;

    fn fx(path: &str, state: &[u8]) -> Fx {
        Fx { path: PathBuf::from(path), index: 0, name: "Knob".into(), bypassed: false, state: state.to_vec(), record: false, mix: 1.0 }
    }

    fn song() -> (Project, TrackId) {
        let mut project = Project::new(48_000);
        let Ok(Outcome::Track(track)) = project.apply(Command::AddTrack { name: "Vox".into() }) else { panic!("no track") };
        let source = Arc::new(Source::from_frames("take", vec![[0.1, 0.1]; 480]));
        project.apply(Command::AddClip { track, source, start: 0 }).unwrap();
        project.apply(Command::AddFx { track, fx: fx("/nowhere/Q.vst3", &[1]) }).unwrap();
        project.apply(Command::AddFx { track, fx: fx("/nowhere/R.vst3", &[2]) }).unwrap();
        project.apply(Command::AddMasterFx(fx("/nowhere/Glue.vst3", &[3]))).unwrap();
        (project, track)
    }

    #[derive(Default)]
    struct Pretend {
        states: Vec<(Touched, Vec<u8>)>,
        asked: Vec<Touched>,
    }

    impl Chains for Pretend {
        fn state_of(&mut self, which: Touched) -> Option<Vec<u8>> {
            self.asked.push(which);
            self.states.iter().find(|(spot, _)| *spot == which).map(|(_, state)| state.clone())
        }
    }

    #[test]
    fn a_knob_that_has_not_moved_leaves_nothing_to_keep() {
        let mut touches = Touches::default();
        assert!(!touches.waiting() && !touches.due());
        touches.noticed(&[]);
        assert!(!touches.waiting() && !touches.due());
        assert!(touches.take().is_empty());
    }

    #[test]
    fn a_touched_slot_is_not_kept_before_the_plugin_goes_quiet() {
        let mut touches = Touches::default();
        touches.noticed(&[Touched::Track(TrackId(1), 0)]);
        assert!(touches.waiting());
        assert!(!touches.due(), "it was kept while the knob was still moving");
        std::thread::sleep(QUIET_FOR);
        assert!(!touches.due(), "it was kept without a quiet look at the plugin");
        touches.noticed(&[]);
        assert!(touches.due(), "it was never kept after the plugin went quiet");
    }

    #[test]
    fn a_whole_sweep_is_kept_as_one_step() {
        let mut touches = Touches::default();
        let spot = Touched::Track(TrackId(1), 0);
        for _ in 0..50 {
            touches.noticed(&[spot]);
            assert!(!touches.due());
        }
        std::thread::sleep(QUIET_FOR);
        touches.noticed(&[]);
        assert!(touches.due());
        assert_eq!(touches.take(), vec![spot], "a sweep of fifty reports came out as more than one step");
        assert!(!touches.waiting() && !touches.due(), "the sweep was still waiting after it was kept");
    }

    #[test]
    fn two_knobs_moved_together_are_kept_as_one_step() {
        let mut touches = Touches::default();
        let one = Touched::Track(TrackId(1), 0);
        let two = Touched::Master(0);
        touches.noticed(&[one]);
        touches.noticed(&[two, one]);
        std::thread::sleep(QUIET_FOR);
        touches.noticed(&[]);
        assert!(touches.due());
        assert_eq!(touches.take(), vec![one, two]);
    }

    #[test]
    fn only_the_slots_that_moved_are_asked_for_their_settings() {
        let (project, track) = song();
        let moved = Touched::Track(track, 1);
        let mut racks = Pretend { states: vec![(moved, vec![9, 9])], asked: Vec::new() };
        let changes = edits(&mut racks, &project, &[moved]);
        assert_eq!(racks.asked, vec![moved], "a plugin that was not touched was asked to save");
        assert_eq!(changes.len(), 1);
        assert!(matches!(&changes[0], Command::SetFxState { track: at, slot: 1, state } if *at == track && state == &vec![9, 9]));
    }

    #[test]
    fn a_knob_that_came_back_to_where_it_was_leaves_no_change() {
        let (project, track) = song();
        let moved = Touched::Track(track, 0);
        let mut racks = Pretend { states: vec![(moved, vec![1])], asked: Vec::new() };
        assert!(edits(&mut racks, &project, &[moved]).is_empty(), "an unchanged plugin still made a change");
    }

    #[test]
    fn a_plugin_that_went_away_before_the_quiet_leaves_no_change() {
        let (project, track) = song();
        let gone = Touched::Track(track, 7);
        let mut racks = Pretend { states: vec![(gone, vec![5])], asked: Vec::new() };
        assert!(edits(&mut racks, &project, &[gone]).is_empty());
        assert!(racks.asked.is_empty(), "a slot the song no longer has was still asked to save");
    }

    #[test]
    fn a_clip_and_the_master_are_kept_with_their_own_commands() {
        let (mut project, track) = song();
        let clip = project.tracks.iter().flat_map(|found| found.clips.iter()).map(|found| found.id).next().expect("a clip");
        project.apply(Command::AddClipFx { clip, fx: fx("/nowhere/Tape.clap", &[4]) }).unwrap();
        let on_clip = Touched::Clip(clip, 0);
        let on_master = Touched::Master(0);
        let mut racks = Pretend { states: vec![(on_clip, vec![40]), (on_master, vec![30])], asked: Vec::new() };
        let changes = edits(&mut racks, &project, &[on_clip, on_master]);
        assert_eq!(changes.len(), 2);
        assert!(matches!(&changes[0], Command::SetClipFxState { clip: at, slot: 0, state } if *at == clip && state == &vec![40]));
        assert!(matches!(&changes[1], Command::SetMasterFxState { slot: 0, state } if state == &vec![30]));
        let _ = track;
    }

    #[test]
    fn the_monitoring_chain_is_kept_with_its_own_command() {
        let (mut project, _) = song();
        project.apply(Command::AddMonitorFx(fx("/nowhere/Denoiser.vst3", &[9]))).unwrap();
        let on_monitor = Touched::Monitor(0);
        let gone = Touched::Monitor(4);
        let mut racks = Pretend { states: vec![(on_monitor, vec![90]), (gone, vec![91])], asked: Vec::new() };
        let changes = edits(&mut racks, &project, &[on_monitor, gone]);
        assert_eq!(changes.len(), 1);
        assert!(matches!(&changes[0], Command::SetMonitorFxState { slot: 0, state } if state == &vec![90]));
        assert_eq!(racks.asked, vec![on_monitor], "a monitoring slot the song has not got was still asked to save");
    }
}

#[cfg(all(test, target_os = "linux"))]
mod running_tests {
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::time::Instant;

    use loupe_engine::{Command, Fx, Outcome, TrackId};
    use loupe_plugins::wire::{bytes_of, hex_of};

    use super::*;
    use crate::{racks, App, Message};

    struct Fake {
        folder: PathBuf,
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.folder);
        }
    }

    impl Fake {
        fn host(&self) -> PathBuf {
            self.folder.join("loupe-host")
        }

        fn turned_to(&self, state: &[u8]) {
            std::fs::write(self.folder.join("state"), hex_of(state)).unwrap();
            std::fs::write(self.folder.join("touch"), "").unwrap();
        }

        fn holding(&self) -> Vec<u8> {
            let text = std::fs::read_to_string(self.folder.join("state")).unwrap_or_default();
            bytes_of(text.trim()).unwrap_or_default()
        }

        fn says_a_knob_moved(&self) {
            std::fs::write(self.folder.join("touch"), "").unwrap();
        }
    }

    fn a_plugin_with_a_knob(name: &str, state: &[u8]) -> Fake {
        let folder = std::env::temp_dir().join(format!("loupe-touches-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let fake = Fake { folder };
        std::fs::write(fake.folder.join("state"), hex_of(state)).unwrap();
        let state_file = fake.folder.join("state").display().to_string();
        let touch_file = fake.folder.join("touch").display().to_string();
        let script = format!(
            "#!/bin/sh\nS={state_file}\nT={touch_file}\nwhile IFS= read -r ask ; do\n  case \"$ask\" in\n    load*) printf 'loaded\\t2\\t2\\t0\\t0\\n' ;;\n    knobs) printf 'knobs\\tGain\\n' ;;\n    touched) if [ -f \"$T\" ] ; then rm -f \"$T\" ; printf 'edited\\t1\\n' ; else printf 'edited\\t0\\n' ; fi ;;\n    save) printf 'state\\t%s\\n' \"$(cat $S)\" ;;\n    restore*) printf '%s' \"${{ask#restore?}}\" > $S ; : > $T ; printf 'fine\\n' ; dd bs=1 count=13 of=/dev/null 2>/dev/null ; printf 'B\\001' ; dd if=/dev/zero bs=1 count=11 2>/dev/null ;;\n    quit) exit 0 ;;\n    *) printf 'fine\\n' ;;\n  esac\ndone\n"
        );
        std::fs::write(fake.host(), script).unwrap();
        std::fs::set_permissions(fake.host(), std::fs::Permissions::from_mode(0o755)).unwrap();
        fake
    }

    fn a_song_with_that_plugin(fake: &Fake) -> (App, TrackId) {
        let mut app = App::bare();
        let Ok(Outcome::Track(track)) = app.project.apply(Command::AddTrack { name: "Vox".into() }) else { panic!("no track") };
        let fx = Fx {
            path: PathBuf::from("/nowhere/Knob.vst3"),
            index: 0,
            name: "Knob".into(),
            bypassed: false,
            state: Vec::new(),
            record: false,
            mix: 1.0,
        };
        app.project.apply(Command::AddFx { track, fx }).unwrap();
        let rate = app.engine.rate();
        let mut racks = racks::Racks::new(rate, 1, racks::Peeks::default(), Default::default(), Default::default());
        racks.host = fake.host();
        racks.follow(&app.project);
        let gave_up = Instant::now();
        while racks.still_opening() {
            assert!(gave_up.elapsed() < Duration::from_secs(60), "the fake plugin never started");
            racks.follow(&app.project);
            std::thread::sleep(Duration::from_millis(5));
        }
        app.racks.keep(Box::new(racks));
        app.engine.set_project(&app.project);
        app.fx_was = app.fx_shape();
        (app, track)
    }

    fn state_on(app: &App, track: TrackId) -> Vec<u8> {
        app.project.tracks.iter().find(|found| found.id == track).expect("the track").fx[0].state.clone()
    }

    fn go_quiet(app: &mut App) {
        app.look_for_touches();
        std::thread::sleep(QUIET_FOR);
        app.look_for_touches();
    }

    #[test]
    fn a_knob_moved_in_the_plugin_becomes_one_undo_step() {
        let fake = a_plugin_with_a_knob("one-step", &[0xaa]);
        let (mut app, track) = a_song_with_that_plugin(&fake);
        fake.turned_to(&[0xaa]);
        app.look_for_touches();
        assert!(app.undo.is_empty(), "the edit was kept before the plugin went quiet");
        assert!(app.dirty, "a knob that moved did not mark the song unsaved");
        go_quiet(&mut app);
        assert_eq!(app.undo.len(), 1, "a quiet plugin did not leave exactly one undo step");
        assert_eq!(state_on(&app, track), vec![0xaa]);
        go_quiet(&mut app);
        assert_eq!(app.undo.len(), 1, "a second quiet look left another step");
    }

    #[test]
    fn a_playing_song_keeps_its_plugins_on_the_audio_thread() {
        let fake = a_plugin_with_a_knob("while-playing", &[0x44]);
        let (mut app, track) = a_song_with_that_plugin(&fake);
        app.engine.play();
        fake.turned_to(&[0x55]);
        go_quiet(&mut app);
        assert!(app.undo.is_empty(), "the plugins were taken off the audio thread while the song played");
        assert!(state_on(&app, track).is_empty(), "the plugins were asked to save while the song played");
        app.engine.stop();
        go_quiet(&mut app);
        assert_eq!(app.undo.len(), 1, "the edit was never kept once the song stopped");
        assert_eq!(state_on(&app, track), vec![0x55]);
    }

    #[test]
    fn a_whole_sweep_of_the_knob_becomes_one_undo_step() {
        let fake = a_plugin_with_a_knob("one-sweep", &[0x01]);
        let (mut app, track) = a_song_with_that_plugin(&fake);
        for step in 1..=30u8 {
            fake.turned_to(&[step]);
            app.look_for_touches();
            assert!(app.undo.is_empty(), "the sweep was cut into steps while the knob was moving");
        }
        go_quiet(&mut app);
        assert_eq!(app.undo.len(), 1, "a sweep of thirty reports left {} steps", app.undo.len());
        assert_eq!(state_on(&app, track), vec![30]);
    }

    #[test]
    fn undo_puts_the_earlier_settings_back_into_the_running_plugin() {
        let fake = a_plugin_with_a_knob("undo-back", &[0x11]);
        let (mut app, track) = a_song_with_that_plugin(&fake);
        fake.turned_to(&[0x11]);
        go_quiet(&mut app);
        fake.turned_to(&[0x22]);
        go_quiet(&mut app);
        assert_eq!(app.undo.len(), 2);
        assert_eq!(state_on(&app, track), vec![0x22]);
        let _ = app.update(Message::Undo);
        assert_eq!(state_on(&app, track), vec![0x11], "undo did not put the earlier settings in the song");
        assert_eq!(fake.holding(), vec![0x11], "undo never reached the running plugin");
        assert_eq!(app.undo.len(), 1);
        assert_eq!(app.redo.len(), 1);
        let _ = app.update(Message::Redo);
        assert_eq!(state_on(&app, track), vec![0x22]);
        assert_eq!(fake.holding(), vec![0x22], "redo never reached the running plugin");
    }

    #[test]
    fn a_plugin_that_calls_an_undo_an_edit_leaves_no_new_step() {
        let fake = a_plugin_with_a_knob("no-spiral", &[0x11]);
        let (mut app, track) = a_song_with_that_plugin(&fake);
        fake.turned_to(&[0x11]);
        go_quiet(&mut app);
        fake.turned_to(&[0x22]);
        go_quiet(&mut app);
        assert_eq!(app.undo.len(), 2);
        let _ = app.update(Message::Undo);
        let steps = app.undo.len();
        assert_eq!(steps, 1);
        fake.says_a_knob_moved();
        app.look_for_touches();
        go_quiet(&mut app);
        assert_eq!(app.undo.len(), steps, "the undo fed itself a new undo step");
        assert_eq!(app.redo.len(), 1, "the redo was thrown away by the plugin reporting the undo");
        assert_eq!(state_on(&app, track), vec![0x11]);
    }

    #[test]
    fn saving_the_song_keeps_no_undo_step() {
        let fake = a_plugin_with_a_knob("on-save", &[0x33]);
        let (mut app, track) = a_song_with_that_plugin(&fake);
        app.gather_fx_state();
        assert!(app.undo.is_empty(), "saving the song left an undo step");
        assert_eq!(state_on(&app, track), vec![0x33], "saving the song did not pick the plugin settings up");
    }

    #[test]
    fn the_racks_stay_with_the_engine_while_it_plays_or_hears_the_input() {
        let mut engine = Engine::start(loupe_engine::Output::Silent);
        assert!(!racks_must_stay(&engine), "a quiet engine can lend its racks");
        let mut input = loupe_engine::Input::open(loupe_engine::InputChoice::Practice).expect("the practice input opens");
        engine.hear(&mut input);
        engine.hear_on(&[TrackId(7)]);
        assert!(racks_must_stay(&engine), "taking the racks while the input is heard drops the rec plugins out of what he hears");
        engine.hear_on(&[]);
        assert!(!racks_must_stay(&engine), "once nothing is armed the racks can be looked at again");
        engine.play();
        assert!(racks_must_stay(&engine), "the racks stay while the song plays");
    }
}
