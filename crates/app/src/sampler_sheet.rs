use std::path::PathBuf;
use std::sync::Arc;

use iced::futures::channel::oneshot;
use iced::widget::canvas::{self, Frame, Geometry, Path};
use iced::widget::{button, checkbox, column, container, row, slider, text, Space};
use iced::{mouse, Alignment, Element, Length, Point, Rectangle, Renderer, Size, Task, Theme};
use loupe_engine::{key_name, Command, Instrument, Sampler, Source, TrackId, HIGHEST_KEY};

use crate::theme::Palette;
use crate::{App, Message, Overlay, Run};

const WAVE_HEIGHT: f32 = 120.0;
const SHEET_WIDTH: f32 = 620.0;
const LONGEST_ATTACK_MS: f32 = 1_000.0;
const LONGEST_RELEASE_MS: f32 = 3_000.0;
const WIDEST_TUNE: f32 = 24.0;

struct Wave<'a> {
    sample: Option<&'a Source>,
    palette: &'a Palette,
}

impl canvas::Program<Message> for Wave<'_> {
    type State = ();

    fn draw(&self, _state: &(), renderer: &Renderer, _theme: &Theme, bounds: Rectangle, _cursor: mouse::Cursor) -> Vec<Geometry> {
        let p = self.palette;
        let size = bounds.size();
        let mut frame = Frame::new(renderer, size);
        frame.fill(&Path::rounded_rectangle(Point::ORIGIN, size, 8.0.into()), p.raised);
        let middle = size.height / 2.0;
        match self.sample.filter(|sample| !sample.frames.is_empty()) {
            Some(sample) => {
                let columns = size.width.max(1.0) as usize;
                let per = sample.frames.len() as f32 / columns as f32;
                for x in 0..columns {
                    let from = (x as f32 * per) as usize;
                    let to = (((x + 1) as f32 * per) as usize).max(from + 1);
                    let (lo, hi) = sample.peak(from, to);
                    let top = middle - hi.clamp(-1.0, 1.0) * (middle - 6.0);
                    let bottom = middle - lo.clamp(-1.0, 1.0) * (middle - 6.0);
                    frame.fill_rectangle(Point::new(x as f32, top), Size::new(1.0, (bottom - top).max(1.0)), p.accent);
                }
            }
            None => {
                frame.fill_text(canvas::Text {
                    content: "Drop a sound here, or load one".into(),
                    position: Point::new(size.width / 2.0, middle),
                    color: p.text_dim,
                    size: 13.0.into(),
                    horizontal_alignment: iced::alignment::Horizontal::Center,
                    vertical_alignment: iced::alignment::Vertical::Center,
                    ..canvas::Text::default()
                });
            }
        }
        vec![frame.into_geometry()]
    }
}

impl App {
    pub(crate) fn sampler_of(&self, track: TrackId) -> Sampler {
        match self.project.track(track).map(|t| t.instrument) {
            Some(Instrument::Sampler(sampler)) => sampler,
            _ => Sampler::default(),
        }
    }

    pub(crate) fn open_sampler(&mut self, track: TrackId) {
        if !matches!(self.project.track(track).map(|t| t.instrument), Some(Instrument::Sampler(_))) {
            self.edit(None, Command::SetInstrument { track, instrument: Instrument::Sampler(Sampler::default()) });
        }
        self.overlay = Overlay::Sampler(track);
    }

    pub(crate) fn change_sampler(&mut self, track: TrackId, sampler: Sampler) {
        self.edit(Some(Run::Sampler(track)), Command::SetInstrument { track, instrument: Instrument::Sampler(sampler) });
    }

    pub(crate) fn pick_sample(&mut self, track: TrackId) -> Task<Message> {
        Task::perform(
            async {
                rfd::AsyncFileDialog::new()
                    .add_filter("Audio", &crate::AUDIO_TYPES)
                    .set_title("Load a sound into the sampler")
                    .pick_file()
                    .await
                    .map(|file| file.path().to_path_buf())
            },
            move |picked| match picked {
                Some(path) => Message::SampleFile(track, path),
                None => Message::Refresh,
            },
        )
    }

    pub(crate) fn load_sample(&mut self, track: TrackId, path: PathBuf) -> Task<Message> {
        self.loading += 1;
        if let Some(kept) = self.project.sources.iter().find(|kept| kept.path == path && !kept.frames.is_empty()) {
            return Task::done(Message::SampleLoaded(track, Ok(kept.clone())));
        }
        let rate = self.project.rate;
        let (done, loaded) = oneshot::channel();
        std::thread::spawn(move || {
            let _ = done.send(Source::load(&path, rate).map(Arc::new));
        });
        Task::perform(
            async move { loaded.await.unwrap_or_else(|_| Err("loading the sound stopped unexpectedly".into())) },
            move |result| Message::SampleLoaded(track, result),
        )
    }

    pub(crate) fn sample_loaded(&mut self, track: TrackId, result: Result<Arc<Source>, String>) {
        self.loading = self.loading.saturating_sub(1);
        match result {
            Ok(sample) => {
                self.transact(None, |project| {
                    if !matches!(project.track(track).map(|t| t.instrument), Some(Instrument::Sampler(_))) {
                        project.apply(Command::SetInstrument { track, instrument: Instrument::Sampler(Sampler::default()) })?;
                    }
                    project.apply(Command::SetSample { track, sample: Some(sample) })
                });
            }
            Err(why) => self.problem = Some(format!("Could not load that sound: {why}")),
        }
    }

    pub(crate) fn sampler_sheet(&self, track: TrackId) -> Element<'_, Message> {
        let palette = self.palette;
        let Some(found) = self.project.track(track) else {
            return self.window("Loupe Sampler".into(), text("This track is gone.").into(), SHEET_WIDTH);
        };
        let sampler = self.sampler_of(track);
        let sample = found.sample.as_deref();
        let wave = canvas::Canvas::new(Wave { sample, palette: &self.palette }).width(Length::Fill).height(WAVE_HEIGHT);
        let name = sample.map_or("No sound loaded".to_string(), |sample| {
            format!("{}  ·  {:.2} s", sample.name, sample.frames.len() as f32 / self.project.rate.max(1) as f32)
        });
        let label = |words: String| text(words).size(12.5).color(palette.text_dim);
        let step_root = |by: i32| {
            let root = (sampler.root as i32 + by).clamp(0, HIGHEST_KEY as i32) as u8;
            Message::SamplerChanged(track, Sampler { root, ..sampler })
        };
        let small = |words: &'static str, message: Message| {
            button(text(words).size(12.5).font(palette.medium)).padding([4, 10]).style(move |_, status| palette.outlined(status)).on_press(message)
        };
        let root = row![
            label("Root key".into()).width(110),
            small("−12", step_root(-12)),
            small("−1", step_root(-1)),
            text(key_name(sampler.root)).size(14).font(palette.medium).width(56).align_x(iced::alignment::Horizontal::Center),
            small("+1", step_root(1)),
            small("+12", step_root(12)),
        ]
        .spacing(6)
        .align_y(Alignment::Center);
        let tune = row![
            label("Tune".into()).width(110),
            slider(-WIDEST_TUNE..=WIDEST_TUNE, sampler.tune, move |tune| Message::SamplerChanged(track, Sampler { tune, ..sampler }))
                .step(0.1)
                .default(0.0)
                .on_release(Message::DragEnd)
                .style(move |_, status| palette.slider(status)),
            label(format!("{:+.1} st", sampler.tune)).width(70),
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        let attack = row![
            label("Attack".into()).width(110),
            slider(0.0..=LONGEST_ATTACK_MS, sampler.attack * 1000.0, move |ms: f32| Message::SamplerChanged(track, Sampler { attack: ms / 1000.0, ..sampler }))
                .step(1.0)
                .on_release(Message::DragEnd)
                .style(move |_, status| palette.slider(status)),
            label(format!("{:.0} ms", sampler.attack * 1000.0)).width(70),
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        let release = row![
            label("Release".into()).width(110),
            slider(0.0..=LONGEST_RELEASE_MS, sampler.release * 1000.0, move |ms: f32| Message::SamplerChanged(track, Sampler { release: ms / 1000.0, ..sampler }))
                .step(5.0)
                .on_release(Message::DragEnd)
                .style(move |_, status| palette.slider(status)),
            label(format!("{:.0} ms", sampler.release * 1000.0)).width(70),
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        let switches = row![
            checkbox("One shot: play the whole sound", sampler.one_shot)
                .on_toggle(move |one_shot| Message::SamplerChanged(track, Sampler { one_shot, ..sampler }))
                .text_size(13),
            Space::with_width(18),
            checkbox("Key tracking", sampler.keytrack)
                .on_toggle(move |keytrack| Message::SamplerChanged(track, Sampler { keytrack, ..sampler }))
                .text_size(13),
        ]
        .align_y(Alignment::Center);
        let top = row![
            text(name).size(13).font(palette.medium),
            Space::with_width(Length::Fill),
            button(text("Load sound…").size(12.5).font(palette.medium))
                .padding([6, 12])
                .style(move |_, status| palette.solid(status))
                .on_press(Message::PickSample(track)),
        ]
        .align_y(Alignment::Center);
        let hint = label("Play it from note clips on this track, a MIDI keyboard or your computer keyboard. The root key plays the sound as it is.".into());
        let body = column![top, wave, root, tune, attack, release, switches, hint].spacing(14);
        self.window(format!("Loupe Sampler: {}", found.name), container(body).into(), SHEET_WIDTH)
    }
}
