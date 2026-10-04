use std::collections::HashMap;
use std::fmt;

use iced::futures::channel::oneshot;
use iced::widget::{column, pick_list, row, text};
use iced::{Element, Length, Task};
use loupe_engine::{milliseconds, Device, Engine, Output, SavedProject};

use crate::files::{read_text, Opened};
use crate::{settings, App, Message};

pub const SYSTEM_OUTPUT: &str = "System default";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Driver(pub String);

impl fmt::Display for Driver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0.as_str() {
            "WASAPI" => write!(f, "Windows audio"),
            "CoreAudio" => write!(f, "Core Audio"),
            other => write!(f, "{other}"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rate(pub Option<u32>);

impl fmt::Display for Rate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(rate) => write!(f, "{rate} Hz"),
            None => write!(f, "Device default"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Buffer {
    pub size: Option<u32>,
    rate: u32,
}

impl fmt::Display for Buffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.size {
            Some(size) => write!(f, "{size} samples ({:.1} ms)", milliseconds(size, self.rate)),
            None => write!(f, "Device default"),
        }
    }
}

#[derive(Default)]
pub struct Lists {
    drivers: Vec<Driver>,
    outputs: Vec<String>,
    rates: Vec<Rate>,
    buffers: Vec<u32>,
}

pub fn running_text(engine: &Engine) -> String {
    match engine.running() {
        None => "No sound output is open.".to_string(),
        Some(running) => {
            let buffer = match running.buffer {
                Some(size) => format!("{size} samples ({:.1} ms)", milliseconds(size, running.rate)),
                None => "the device's own buffer".to_string(),
            };
            format!("Playing through {} on {}, {} Hz, {buffer}.", running.output, Driver(running.driver.clone()), running.rate)
        }
    }
}

impl App {
    pub(crate) fn refresh_audio_lists(&mut self) {
        let driver = self.audio.driver.clone().unwrap_or_else(loupe_engine::default_driver);
        let choices = loupe_engine::choices(Some(&driver), self.audio.output.as_deref());
        let mut outputs = vec![SYSTEM_OUTPUT.to_string()];
        outputs.extend(loupe_engine::outputs(Some(&driver)));
        let mut rates = vec![Rate(None)];
        rates.extend(choices.rates.into_iter().map(|rate| Rate(Some(rate))));
        self.audio_lists = Lists {
            drivers: loupe_engine::drivers().into_iter().map(Driver).collect(),
            outputs,
            rates,
            buffers: choices.buffers,
        };
    }

    pub(crate) fn choose_audio(&mut self, chosen: Device) -> Task<Message> {
        if chosen == self.audio {
            return Task::none();
        }
        if self.recording.is_some() {
            self.notice = Some("Stop recording first.".into());
            return Task::none();
        }
        let saved = [
            ("audio_driver", chosen.driver.clone()),
            ("audio_output", chosen.output.clone()),
            ("audio_rate", chosen.rate.map(|rate| rate.to_string())),
            ("audio_buffer", chosen.buffer.map(|size| size.to_string())),
        ]
        .into_iter()
        .map(|(key, value)| match value {
            Some(value) => settings::save(key, &value),
            None => settings::forget(key),
        })
        .find_map(Result::err);
        self.audio = chosen;
        let task = self.restart_audio();
        self.refresh_audio_lists();
        if let Some(why) = saved {
            self.problem = Some(format!("Could not save settings: {why}"));
        }
        task
    }

    fn restart_audio(&mut self) -> Task<Message> {
        if self.playing {
            self.engine.stop();
            self.playing = false;
        }
        let old_rate = self.engine.rate();
        self.gather_fx_state();
        let racks = self.borrow_racks();
        let output = if self.silent { Output::SilentAt(self.audio.rate.unwrap_or(48_000)) } else { Output::Device(self.audio.clone()) };
        drop(std::mem::replace(&mut self.engine, Engine::start(Output::Silent)));
        self.engine = Engine::start(output);
        self.problem = self.engine.output_error().map(|e| format!("Sound output: {e}"));
        self.engine.set_metronome(self.metronome);
        self.engine.seek(self.playhead);
        if self.midi_keys.is_some() {
            self.midi_keys = loupe_engine::MidiKeys::open(self.engine.key_sender()).ok();
        }
        self.input = None;
        self.input_names = loupe_engine::input_devices();
        self.listen_if_armed();
        let new_rate = self.engine.rate();
        if new_rate == old_rate {
            self.engine.set_project(&self.project);
            self.racks = racks;
            self.hand_racks_over();
            return Task::none();
        }
        drop(racks);
        self.fx_was = 0;
        self.reopen_at(new_rate)
    }

    fn reopen_at(&mut self, rate: u32) -> Task<Message> {
        let text = SavedProject::capture(&self.project, |track| self.heights.get(&track).copied()).to_text();
        self.loading += 1;
        let (done, opened) = oneshot::channel();
        std::thread::spawn(move || {
            let _ = done.send(read_text(&text, rate));
        });
        Task::perform(
            async move { opened.await.unwrap_or_else(|_| Err("reopening stopped unexpectedly".into())) },
            Message::Reopened,
        )
    }

    pub(crate) fn reopened(&mut self, result: Result<Opened, String>) {
        self.loading = self.loading.saturating_sub(1);
        let opened = match result {
            Ok(opened) => opened,
            Err(why) => {
                self.problem = Some(format!("Could not move the song to the new sample rate: {why}"));
                return;
            }
        };
        let (path, dirty, playhead) = (self.path.clone(), self.dirty, self.playhead);
        let old_rate = self.project.rate.max(1) as u64;
        let (project, heights) = opened.build(self.engine.rate());
        self.replace_project(project, heights.into_iter().collect::<HashMap<_, _>>());
        self.path = path;
        self.dirty = dirty;
        self.seek((playhead as u128 * self.engine.rate() as u128 / old_rate as u128) as u64);
        self.notice = Some("The song moved to the new sample rate. Undo history starts again from here.".into());
    }

    pub(crate) fn audio_settings(&self) -> Element<'_, Message> {
        let palette = self.palette;
        let lists = &self.audio_lists;
        let driver = Driver(self.audio.driver.clone().unwrap_or_else(loupe_engine::default_driver));
        let output = self.audio.output.clone().unwrap_or_else(|| SYSTEM_OUTPUT.to_string());
        let rate = self.engine.rate();
        let buffers: Vec<Buffer> =
            std::iter::once(None).chain(lists.buffers.iter().copied().map(Some)).map(|size| Buffer { size, rate }).collect();
        let label = |words: &'static str| text(words).size(13).font(palette.medium);
        let dim = |words: String| text(words).size(12).color(palette.text_dim);
        column![
            label("Driver"),
            pick_list(lists.drivers.clone(), Some(driver), Message::AudioDriverChosen).text_size(13).padding([5, 10]).width(Length::Fill),
            label("Output"),
            pick_list(lists.outputs.clone(), Some(output), Message::AudioOutputChosen).text_size(13).padding([5, 10]).width(Length::Fill),
            row![
                column![
                    label("Sample rate"),
                    pick_list(lists.rates.clone(), Some(Rate(self.audio.rate)), Message::AudioRateChosen).text_size(13).padding([5, 10]).width(Length::Fill),
                ]
                .spacing(8),
                column![
                    label("Buffer size"),
                    pick_list(buffers, Some(Buffer { size: self.audio.buffer, rate }), Message::AudioBufferChosen).text_size(13).padding([5, 10]).width(Length::Fill),
                ]
                .spacing(8),
            ]
            .spacing(12),
            dim(running_text(&self.engine)),
        ]
        .spacing(8)
        .into()
    }
}
