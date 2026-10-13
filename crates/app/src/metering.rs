use std::ops::RangeInclusive;
use std::time::Instant;

use iced::widget::{button, column, horizontal_space, row, slider, text};
use iced::{Alignment, Element};
use loupe_engine::{MeterPoint, Track, TrackId, TrackPeaks};

use crate::mixer::Level;
use crate::{App, Message};

pub const DECAY_DB_PER_SECOND: RangeInclusive<f32> = 6.0..=200.0;
pub const HOLD_SECONDS: RangeInclusive<f32> = 0.0..=5.0;
pub const REFRESH_HZ: RangeInclusive<f32> = 10.0..=60.0;
pub const DEFAULT_DECAY_DB_PER_SECOND: f32 = 82.0;
pub const DEFAULT_HOLD_SECONDS: f32 = 1.0;
pub const DEFAULT_REFRESH_HZ: f32 = 60.0;
const EARLY_SAMPLE_SECONDS: f32 = 0.004;
const STILL_LEVEL: f32 = 0.0005;
const DECAY_KEY: &str = "meter_decay_db_per_second";
const HOLD_KEY: &str = "meter_hold_seconds";
const REFRESH_KEY: &str = "meter_refresh_hz";
const SETTING_NAME_WIDTH: f32 = 72.0;
const SETTING_VALUE_WIDTH: f32 = 64.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeterSettings {
    pub decay_db_per_second: f32,
    pub hold_seconds: f32,
    pub refresh_hz: f32,
}

impl Default for MeterSettings {
    fn default() -> Self {
        Self { decay_db_per_second: DEFAULT_DECAY_DB_PER_SECOND, hold_seconds: DEFAULT_HOLD_SECONDS, refresh_hz: DEFAULT_REFRESH_HZ }
    }
}

fn kept_within(value: f32, range: &RangeInclusive<f32>, otherwise: f32) -> f32 {
    if value.is_finite() {
        value.clamp(*range.start(), *range.end())
    } else {
        otherwise
    }
}

impl MeterSettings {
    pub fn clamped(self) -> Self {
        Self {
            decay_db_per_second: kept_within(self.decay_db_per_second, &DECAY_DB_PER_SECOND, DEFAULT_DECAY_DB_PER_SECOND),
            hold_seconds: kept_within(self.hold_seconds, &HOLD_SECONDS, DEFAULT_HOLD_SECONDS),
            refresh_hz: kept_within(self.refresh_hz, &REFRESH_HZ, DEFAULT_REFRESH_HZ),
        }
    }

    pub fn from_settings(value_of: impl Fn(&str) -> Option<String>) -> Self {
        let number = |key: &str, otherwise: f32| value_of(key).and_then(|text| text.trim().parse::<f32>().ok()).unwrap_or(otherwise);
        Self {
            decay_db_per_second: number(DECAY_KEY, DEFAULT_DECAY_DB_PER_SECOND),
            hold_seconds: number(HOLD_KEY, DEFAULT_HOLD_SECONDS),
            refresh_hz: number(REFRESH_KEY, DEFAULT_REFRESH_HZ),
        }
        .clamped()
    }

    pub fn entries(&self) -> [(&'static str, String); 3] {
        [
            (DECAY_KEY, format!("{:.0}", self.decay_db_per_second)),
            (HOLD_KEY, format!("{:.1}", self.hold_seconds)),
            (REFRESH_KEY, format!("{:.0}", self.refresh_hz)),
        ]
    }

    pub fn fall_over(&self, seconds: f32) -> f32 {
        10f32.powf(-self.decay_db_per_second * seconds.max(0.0) / 20.0)
    }

    fn sample_period(&self) -> f32 {
        1.0 / self.refresh_hz
    }

    pub fn due(&self, last_sample: Option<Instant>, at: Instant) -> Option<f32> {
        let Some(last) = last_sample else {
            return Some(0.0);
        };
        let seconds = at.saturating_duration_since(last).as_secs_f32();
        (seconds + EARLY_SAMPLE_SECONDS >= self.sample_period()).then_some(seconds)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Meter {
    pub shown: f32,
    pub hold: f32,
    hold_left: f32,
    pub max: f32,
}

impl Meter {
    pub fn take(&mut self, peak: f32, seconds: f32, settings: &MeterSettings) {
        let seconds = seconds.max(0.0);
        self.shown = peak.max(self.shown * settings.fall_over(seconds));
        if peak >= self.hold {
            self.hold = peak;
            self.hold_left = settings.hold_seconds;
        } else if self.hold_left >= seconds {
            self.hold_left -= seconds;
        } else {
            let falling = seconds - self.hold_left;
            self.hold_left = 0.0;
            self.hold *= settings.fall_over(falling);
        }
        self.hold = self.hold.max(self.shown);
        self.max = self.max.max(peak);
    }

    pub fn reset_max(&mut self) {
        self.max = 0.0;
    }

    pub fn moving(&self) -> bool {
        self.shown > STILL_LEVEL || self.hold > STILL_LEVEL
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeterSetting {
    Decay,
    Hold,
    Refresh,
}

impl MeterSetting {
    pub fn set(self, settings: MeterSettings, value: f32) -> MeterSettings {
        match self {
            Self::Decay => MeterSettings { decay_db_per_second: value, ..settings },
            Self::Hold => MeterSettings { hold_seconds: value, ..settings },
            Self::Refresh => MeterSettings { refresh_hz: value, ..settings },
        }
        .clamped()
    }
}

pub fn point_label(point: MeterPoint) -> &'static str {
    match point {
        MeterPoint::PreFx => "Pre-FX",
        MeterPoint::PostFx => "Post-FX, pre-fader",
        MeterPoint::PostFader => "Post-fader",
    }
}

pub fn hold_colour(palette: &crate::theme::Palette, hold: f32, hold_db: f32, zones: [(f32, iced::Color); 3]) -> iced::Color {
    if hold >= 1.0 {
        return palette.danger;
    }
    zones.iter().rev().find(|(from, _)| hold_db >= *from).map_or(zones[0].1, |(_, colour)| *colour)
}

pub fn track_reading(track: &Track, peaks: TrackPeaks, armed: bool, hearing: bool, input_peaks: &[f32]) -> f32 {
    let listening = armed && !track.records_notes;
    match listening && !hearing {
        true => track.input.level(input_peaks),
        false => peaks[track.meter.index()],
    }
}

impl App {
    pub(crate) fn sample_meters_when_due(&mut self, at: Instant) {
        let Some(seconds) = self.meter_settings.due(self.meters_sampled_at, at) else {
            return;
        };
        self.meters_sampled_at = Some(at);
        let (tracks, master) = self.engine.take_levels();
        let input_peaks = self.input.as_ref().map(|input| input.take_peaks()).unwrap_or_default();
        self.sample_meters(seconds, tracks, master, input_peaks);
    }

    pub(crate) fn sample_meters(&mut self, seconds: f32, tracks: [TrackPeaks; loupe_engine::METERS], master: f32, input_peaks: Vec<f32>) {
        let settings = self.meter_settings;
        let hearing = self.engine.is_hearing();
        for (index, (meter, peaks)) in self.track_meters.iter_mut().zip(tracks).enumerate() {
            let peak = match self.project.tracks.get(index) {
                Some(track) => track_reading(track, peaks, self.armed.contains(&track.id), hearing, &input_peaks),
                None => peaks[MeterPoint::PostFader.index()],
            };
            meter.take(peak, seconds, &settings);
        }
        self.master_meter.take(master, seconds, &settings);
        self.input_peaks = input_peaks;
    }

    pub(crate) fn meters_moving(&self) -> bool {
        self.master_meter.moving() || self.track_meters.iter().any(Meter::moving)
    }

    pub(crate) fn track_meter(&self, index: usize) -> Meter {
        self.track_meters.get(index).copied().unwrap_or_default()
    }

    pub(crate) fn start_meters_afresh(&mut self) {
        self.master_meter.reset_max();
        for meter in self.track_meters.iter_mut() {
            meter.reset_max();
        }
    }

    pub(crate) fn reset_max_peak(&mut self, level: Level) {
        match level {
            Level::Master => self.master_meter.reset_max(),
            Level::Track(track) => {
                if let Some(meter) = self.project.tracks.iter().position(|found| found.id == track).and_then(|index| self.track_meters.get_mut(index)) {
                    meter.reset_max();
                }
            }
            Level::Clip(_) => {}
        }
    }

    pub(crate) fn forget_track_meter(&mut self, track: TrackId) {
        if let Some(meter) = self.project.tracks.iter().position(|found| found.id == track).and_then(|index| self.track_meters.get_mut(index)) {
            *meter = Meter::default();
        }
    }

    pub(crate) fn keep_meter_settings(&mut self) {
        self.meter_settings = self.meter_settings.clamped();
        let failed = self.meter_settings.entries().iter().find_map(|(key, value)| crate::settings::save(key, value).err());
        if let Some(why) = failed {
            self.problem = Some(format!("Could not save the meter settings: {why}"));
        }
    }

    pub(crate) fn meter_menu(&self, track: TrackId) -> Element<'_, Message> {
        let current = self.project.track(track).map(|found| found.meter);
        let items = MeterPoint::ALL
            .into_iter()
            .map(|point| {
                let label = match current == Some(point) {
                    true => format!("{} ✓", point_label(point)),
                    false => point_label(point).to_string(),
                };
                self.item(label, "", Some(Message::MeterPointChosen { track, point }))
            })
            .collect();
        self.menu(items)
    }

    pub(crate) fn meter_settings_section(&self) -> Element<'_, Message> {
        let palette = self.palette;
        let shown = self.meter_settings;
        let line = |name: &'static str, range: &RangeInclusive<f32>, step: f32, value: f32, words: String, which: MeterSetting| {
            row![
                text(name).size(12.5).color(palette.text_dim).width(SETTING_NAME_WIDTH),
                slider(range.clone(), value, move |value| Message::MeterSettingDragged(which, value))
                    .step(step)
                    .on_release(Message::MeterSettingsKept)
                    .style(move |_, status| palette.slider(status)),
                text(words).size(12.5).font(palette.mono).width(SETTING_VALUE_WIDTH),
            ]
            .spacing(12)
            .align_y(Alignment::Center)
        };
        column![
            text("Meters").size(13).font(palette.medium),
            text("How the level meters move. The numbers under the mixer faders hold the loudest peak until you click them or press play.")
                .size(12)
                .color(palette.text_dim),
            line("Decay", &DECAY_DB_PER_SECOND, 1.0, shown.decay_db_per_second, format!("{:.0} dB/s", shown.decay_db_per_second), MeterSetting::Decay),
            line("Peak hold", &HOLD_SECONDS, 0.1, shown.hold_seconds, format!("{:.1} s", shown.hold_seconds), MeterSetting::Hold),
            line("Refresh", &REFRESH_HZ, 1.0, shown.refresh_hz, format!("{:.0} Hz", shown.refresh_hz), MeterSetting::Refresh),
            row![
                horizontal_space(),
                button(text("Reset").size(12.5).font(palette.medium))
                    .padding([6, 12])
                    .style(move |_, status| palette.outlined(status))
                    .on_press_maybe((shown != MeterSettings::default()).then_some(Message::MeterSettingsReset)),
            ],
        ]
        .spacing(8)
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn two_tracks() -> (App, [TrackId; 2]) {
        let mut app = App::bare();
        let mut add = |name: &str| match app.project.apply(loupe_engine::Command::AddTrack { name: name.into() }) {
            Ok(loupe_engine::Outcome::Track(id)) => id,
            other => panic!("{other:?}"),
        };
        let ids = [add("Vox"), add("Beat")];
        (app, ids)
    }

    fn peaks_of(first: TrackPeaks) -> [TrackPeaks; loupe_engine::METERS] {
        let mut all = [[0.0; loupe_engine::METER_POINTS]; loupe_engine::METERS];
        all[0] = first;
        all[1] = [0.3; loupe_engine::METER_POINTS];
        all
    }

    #[test]
    fn the_max_number_holds_until_it_is_clicked_and_play_clears_them_all() {
        let (mut app, [vox, _]) = two_tracks();
        app.sample_meters(0.016, peaks_of([0.1, 0.2, 0.8]), 0.9, Vec::new());
        for _ in 0..300 {
            app.sample_meters(0.016, peaks_of([0.0, 0.0, 0.05]), 0.05, Vec::new());
        }
        assert_eq!(app.track_meters[0].max, 0.8, "the post-fader peak is the one held by default");
        assert_eq!(app.master_meter.max, 0.9);
        assert!(app.track_meters[0].shown < 0.06, "the bar should have fallen, it shows {}", app.track_meters[0].shown);
        let _ = app.update(Message::ResetMaxPeak(Level::Track(vox)));
        assert_eq!(app.track_meters[0].max, 0.0, "clicking the number did not reset it");
        assert_eq!(app.track_meters[1].max, 0.3, "clicking one number reset another track");
        assert_eq!(app.master_meter.max, 0.9, "clicking a track number reset the master");
        let _ = app.update(Message::ResetMaxPeak(Level::Master));
        assert_eq!(app.master_meter.max, 0.0);
        app.sample_meters(0.016, peaks_of([0.0, 0.0, 0.6]), 0.7, Vec::new());
        let _ = app.update(Message::TogglePlay);
        assert!(app.playing);
        assert_eq!(app.master_meter.max, 0.0, "pressing play kept the master number");
        assert!(app.track_meters.iter().all(|meter| meter.max == 0.0), "pressing play kept a track number");
        let _ = app.update(Message::TogglePlay);
    }

    #[test]
    fn switching_a_meters_point_starts_its_reading_afresh_from_the_new_point() {
        let (mut app, [vox, beat]) = two_tracks();
        app.sample_meters(0.016, peaks_of([0.2, 0.5, 0.9]), 0.9, Vec::new());
        assert_eq!(app.track_meters[0].max, 0.9);
        let _ = app.update(Message::MeterMenu { track: vox, at: Some(iced::Point::new(10.0, 10.0)) });
        assert!(matches!(app.overlay, crate::Overlay::MeterMenu { track, .. } if track == vox));
        let _ = app.meter_menu(vox);
        let _ = app.update(Message::MeterPointChosen { track: vox, point: MeterPoint::PreFx });
        assert_eq!(app.project.track(vox).unwrap().meter, MeterPoint::PreFx);
        assert_eq!(app.track_meters[0], Meter::default(), "the old point's bar, hold line or number survived the switch");
        assert_eq!(app.track_meters[1].max, 0.3, "switching one track touched another");
        app.sample_meters(0.016, peaks_of([0.2, 0.5, 0.9]), 0.9, Vec::new());
        assert_eq!((app.track_meters[0].shown, app.track_meters[0].hold, app.track_meters[0].max), (0.2, 0.2, 0.2));
        let _ = app.update(Message::MeterPointChosen { track: beat, point: MeterPoint::PostFx });
        app.sample_meters(0.016, peaks_of([0.2, 0.5, 0.9]), 0.9, Vec::new());
        assert_eq!(app.track_meters[1].max, 0.3);
    }

    #[test]
    fn an_armed_track_that_is_not_heard_shows_what_arrives_at_its_input() {
        let (mut app, [vox, _]) = two_tracks();
        let track = app.project.track(vox).unwrap().clone();
        assert_eq!(track_reading(&track, [0.1, 0.2, 0.3], false, false, &[0.7, 0.4]), 0.3);
        assert_eq!(track_reading(&track, [0.1, 0.2, 0.3], true, true, &[0.7, 0.4]), 0.3, "a heard track reads its own point");
        assert_eq!(track_reading(&track, [0.1, 0.2, 0.3], true, false, &[0.7, 0.4]), 0.7, "an armed track with nothing through it shows its input");
        app.armed.insert(vox);
        app.sample_meters(0.016, peaks_of([0.0; 3]), 0.0, vec![0.6, 0.1]);
        assert_eq!(app.track_meters[0].max, 0.6);
    }

    #[test]
    fn the_meter_settings_page_is_laid_out_and_a_drag_is_clamped_then_kept() {
        let (mut app, _) = two_tracks();
        let _ = app.meter_settings_section();
        let _ = app.update(Message::MeterSettingDragged(MeterSetting::Decay, 500.0));
        assert_eq!(app.meter_settings.decay_db_per_second, 200.0);
        let _ = app.update(Message::MeterSettingDragged(MeterSetting::Hold, 2.5));
        let _ = app.update(Message::MeterSettingDragged(MeterSetting::Refresh, 3.0));
        assert_eq!(app.meter_settings, MeterSettings { decay_db_per_second: 200.0, hold_seconds: 2.5, refresh_hz: 10.0 });
    }

    fn db(gain: f32) -> f32 {
        20.0 * gain.log10()
    }

    #[test]
    fn the_defaults_match_the_old_meter_and_reaper() {
        let fresh = MeterSettings::default();
        assert_eq!(fresh.decay_db_per_second, 82.0);
        assert_eq!(fresh.hold_seconds, 1.0);
        assert_eq!(fresh.refresh_hz, 60.0);
        let old_fall_per_second = db(0.86) / 0.016;
        assert!((old_fall_per_second + fresh.decay_db_per_second).abs() < 0.5, "the old bar fell {old_fall_per_second} dB/s");
        assert_eq!(MeterSettings::from_settings(|_| None), fresh);
    }

    #[test]
    fn the_fall_in_decibels_does_not_depend_on_how_long_a_tick_is() {
        let settings = MeterSettings { decay_db_per_second: 50.0, hold_seconds: 0.0, refresh_hz: 60.0 };
        let mut fallen = Vec::new();
        for steps in [1usize, 7, 10, 60, 250] {
            let mut meter = Meter::default();
            meter.take(1.0, 0.0, &settings);
            for _ in 0..steps {
                meter.take(0.0, 0.5 / steps as f32, &settings);
            }
            fallen.push(db(meter.shown));
        }
        for drop in &fallen {
            assert!((drop + 25.0).abs() < 0.01, "half a second at 50 dB/s fell to {drop} dB, all: {fallen:?}");
        }
    }

    #[test]
    fn the_hold_line_waits_its_time_then_falls_at_the_decay_rate() {
        let settings = MeterSettings { decay_db_per_second: 40.0, hold_seconds: 1.0, refresh_hz: 60.0 };
        let mut meter = Meter::default();
        meter.take(0.5, 0.0, &settings);
        for _ in 0..9 {
            meter.take(0.0, 0.1, &settings);
        }
        assert_eq!(meter.hold, 0.5, "the hold line moved before its second was up");
        assert!(meter.shown < meter.hold);
        meter.take(0.0, 0.35, &settings);
        assert!((db(meter.hold) - (db(0.5) - 10.0)).abs() < 0.01, "a quarter second after the hold, got {} dB", db(meter.hold));
        meter.take(0.8, 0.1, &settings);
        assert_eq!(meter.hold, 0.8, "a louder peak lifts the hold line straight away");
    }

    #[test]
    fn the_max_holds_until_it_is_reset() {
        let settings = MeterSettings::default();
        let mut meter = Meter::default();
        meter.take(0.7, 0.0, &settings);
        for _ in 0..600 {
            meter.take(0.01, 0.016, &settings);
        }
        assert_eq!(meter.max, 0.7, "the highest peak fell away");
        meter.take(1.2, 0.016, &settings);
        assert_eq!(meter.max, 1.2);
        meter.reset_max();
        assert_eq!(meter.max, 0.0);
        meter.take(0.3, 0.016, &settings);
        assert_eq!(meter.max, 0.3);
    }

    #[test]
    fn meters_are_sampled_at_the_refresh_rate_with_the_real_time_between() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let sixty = MeterSettings::default();
        assert_eq!(sixty.due(None, start), Some(0.0));
        assert!(sixty.due(Some(start), at(16)).is_some(), "a 16 ms tick misses the 60 Hz refresh");
        let ten = MeterSettings { refresh_hz: 10.0, ..sixty };
        assert_eq!(ten.due(Some(start), at(48)), None);
        assert_eq!(ten.due(Some(start), at(80)), None);
        let seconds = ten.due(Some(start), at(112)).unwrap();
        assert!((seconds - 0.112).abs() < 1e-4, "{seconds}");
    }

    #[test]
    fn meter_settings_are_clamped_and_come_back_as_saved() {
        let chosen = MeterSettings { decay_db_per_second: 24.0, hold_seconds: 2.5, refresh_hz: 30.0 };
        let text: String = chosen.entries().iter().map(|(key, value)| format!("{key} = {value}\n")).collect();
        let back = MeterSettings::from_settings(|wanted| crate::settings::entries(&text).find(|(_, key, _)| *key == wanted).map(|(_, _, value)| value.to_string()));
        assert_eq!(back, chosen);
        let wild = MeterSettings::from_settings(|key| Some(if key == HOLD_KEY { "-3" } else if key == DECAY_KEY { "900" } else { "1" }.to_string()));
        assert_eq!(wild, MeterSettings { decay_db_per_second: 200.0, hold_seconds: 0.0, refresh_hz: 10.0 });
        let low = MeterSettings { decay_db_per_second: 1.0, hold_seconds: 9.0, refresh_hz: 500.0 }.clamped();
        assert_eq!(low, MeterSettings { decay_db_per_second: 6.0, hold_seconds: 5.0, refresh_hz: 60.0 });
        let junk = MeterSettings::from_settings(|_| Some("loud".to_string()));
        assert_eq!(junk, MeterSettings::default());
        let not_a_number = MeterSettings { decay_db_per_second: f32::NAN, hold_seconds: f32::INFINITY, refresh_hz: f32::NAN }.clamped();
        assert_eq!(not_a_number, MeterSettings::default());
    }
}
