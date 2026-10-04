use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::{button, canvas, column, container, mouse_area, row as hrow, scrollable, text, text_input, vertical_slider, Space};
use iced::{mouse, Alignment, Color, Element, Length};

use loupe_engine::{ClipId, Command, TrackId};

use crate::{upright_rule, App, Message};

pub const MIXER_HEIGHT: f32 = 236.0;
pub const SHORTEST_MIXER: f32 = 150.0;
const MIXER_WINDOW: iced::Size = iced::Size::new(1000.0, 360.0);
const SHORTEST_MIXER_WINDOW: f32 = 420.0;
const GRAB_BAR: f32 = 6.0;
const STRIP_MARGIN: f32 = 32.0;
const STRIP_WIDTH: f32 = 112.0;
const NAME_LENGTH: usize = 9;
const PAN_PER_PX: f32 = 1.0;
pub const SILENT_DB: f32 = -60.0;
pub const LOUDEST_DB: f32 = 6.0;
pub const LEVEL_ENTRY_ID: &str = "level-entry";
pub const LOUDEST_MASTER_PERCENT: f32 = 125.0;

pub fn percent_text(gain: f32) -> String {
    format!("{:.0}%", gain * 100.0)
}

pub fn percent_from_typed(typed: &str) -> Option<f32> {
    let number = typed.trim().trim_end_matches('%').trim();
    number.parse::<f32>().ok().filter(|percent| percent.is_finite()).map(|percent| percent.clamp(0.0, LOUDEST_MASTER_PERCENT))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Level {
    Track(TrackId),
    Clip(ClipId),
    Master,
}

pub fn db_from_typed(typed: &str) -> Option<f32> {
    let typed = typed.trim().to_lowercase();
    let number = typed.trim_end_matches("db").trim();
    if number == "-inf" {
        return Some(SILENT_DB);
    }
    number.parse::<f32>().ok().filter(|db| db.is_finite())
}

pub fn gain_from_db(db: f32) -> f32 {
    if db <= SILENT_DB {
        0.0
    } else {
        10f32.powf(db / 20.0)
    }
}

pub fn db_from_gain(gain: f32) -> f32 {
    if gain <= 0.0 {
        SILENT_DB
    } else {
        (20.0 * gain.log10()).clamp(SILENT_DB, LOUDEST_DB)
    }
}

pub fn level_text(gain: f32) -> String {
    if gain <= 0.0 {
        "-inf".to_string()
    } else {
        format!("{:+.1}", (20.0 * gain.log10()).max(SILENT_DB))
    }
}

pub fn pan_text(pan: f32) -> String {
    let percent = (pan * 100.0).round() as i32;
    match percent {
        0 => "C".to_string(),
        p if p < 0 => format!("L{}", -p),
        p => format!("R{p}"),
    }
}

impl App {
    pub(crate) fn toggle_solo(&mut self, track: TrackId) {
        let Some(was) = self.project.track(track).map(|t| t.solo) else {
            return;
        };
        let adding = self.modifiers.command();
        let others: Vec<TrackId> = match adding {
            true => Vec::new(),
            false => self.project.tracks.iter().filter(|t| t.solo && t.id != track).map(|t| t.id).collect(),
        };
        let solo = !was || (!adding && !others.is_empty());
        self.transact(None, |project| {
            for other in others {
                project.apply(Command::SetTrackSolo { track: other, solo: false })?;
            }
            project.apply(Command::SetTrackSolo { track, solo })
        });
    }

    pub(crate) fn pan_knob(&self, track: TrackId, pan: f32) -> Element<'_, Message> {
        let palette = self.palette;
        hrow![
            mouse_area(canvas(crate::knob::Knob {
                palette: &self.palette,
                value: pan * 100.0,
                lowest: -100.0,
                highest: 100.0,
                resting: 0.0,
                per_px: PAN_PER_PX,
                centred: true,
                on_turn: Box::new(move |percent| Message::TrackPan(track, percent / 100.0)),
            })
            .width(20)
            .height(20))
            .on_right_press(Message::OpenAutomation(loupe_engine::Target::TrackPan(track)))
            .on_enter(Message::Hint(Some("Track pan. Drag up and down, right click to automate it.")))
            .on_exit(Message::Hint(None)),
        ]
        .push_maybe((self.editing_level.is_none()).then(|| text(pan_text(pan)).size(10.5).font(palette.mono).color(palette.text_dim).width(26)))
        .spacing(3)
        .align_y(Alignment::Center)
        .into()
    }

    pub(crate) fn level_readout(&self, level: Level, gain: f32) -> Element<'_, Message> {
        let palette = self.palette;
        if self.editing_level == Some(level) {
            return text_input("", &self.entry)
                .id(LEVEL_ENTRY_ID)
                .on_input(Message::EntryTyped)
                .on_submit(Message::LevelEntered)
                .font(palette.mono)
                .size(11.5)
                .padding([2, 4])
                .width(52)
                .style(move |_, status| palette.field(status))
                .into();
        }
        let shown = if level == Level::Master { percent_text(gain) } else { level_text(gain) };
        mouse_area(text(shown).size(11.5).font(palette.mono).color(palette.text_dim))
            .on_press(Message::LevelPressed(level))
            .into()
    }

    fn mixer_window_button(&self) -> Element<'_, Message> {
        let palette = self.palette;
        let alone = self.mixer_window.is_some();
        button(text(if alone { "Dock" } else { "Pop out" }).size(10.5).font(palette.medium))
            .padding([2, 6])
            .style(move |_, status| palette.outlined(status))
            .on_press(if alone { Message::MixerBackUnderTheSong } else { Message::MixerToItsOwnWindow })
            .into()
    }

    fn master_strip(&self) -> Element<'_, Message> {
        let palette = self.palette;
        let muted = self.project.master_muted;
        let heard = self
            .project
            .envelope(loupe_engine::Target::MasterGain)
            .and_then(|shape| shape.value_at(self.playhead))
            .unwrap_or(self.project.master);
        let percent = heard * 100.0;
        container(
            column![
                container(Space::new(Length::Fill, 3)).style(move |_| container::Style {
                    background: Some(palette.accent.into()),
                    ..Default::default()
                }),
                hrow![text("Master").size(12).font(palette.semibold).width(Length::Fill), self.mixer_window_button()]
                    .align_y(Alignment::Center),
                self.master_fx_block(),
                hrow![
                    mouse_area(
                        vertical_slider(0.0..=LOUDEST_MASTER_PERCENT, percent, Message::MasterPercent)
                            .step(1.0)
                            .default(100.0)
                            .on_release(Message::DragEnd)
                            .height(Length::Fill)
                            .style(move |_, status| palette.slider(status))
                    )
                    .on_scroll(|delta| Message::WheelOverFader(Level::Master, delta)),
                    meter(palette, self.master_level),
                ]
                .spacing(10)
                .height(Length::Fill)
                .align_y(Alignment::Center),
                self.level_readout(Level::Master, heard),
                button(text("M").size(11.5).font(palette.semibold))
                    .padding([3, 9])
                    .style(move |_, status| palette.mute(muted, status))
                    .on_press(Message::ToggleMasterMute),
            ]
            .spacing(8)
            .align_x(Alignment::Center),
        )
        .padding(8)
        .width(STRIP_WIDTH)
        .height(self.strip_tall())
        .style(move |_| palette.strip())
        .into()
    }

    pub(crate) fn mixer_to_its_own_window(&mut self) -> iced::Task<Message> {
        if self.mixer_window.is_some() {
            return iced::Task::none();
        }
        let (_, opening) = iced::window::open(iced::window::Settings {
            size: MIXER_WINDOW,
            min_size: Some(iced::Size::new(SHORTEST_MIXER_WINDOW, SHORTEST_MIXER)),
            icon: iced::window::icon::from_file_data(include_bytes!("../assets/icon.png"), None).ok(),
            ..iced::window::Settings::default()
        });
        opening.map(Message::MixerWindowOpened)
    }

    fn strip_tall(&self) -> Length {
        match self.mixer_window {
            Some(_) => Length::Fill,
            None => Length::Fixed(self.mixer_height - STRIP_MARGIN - GRAB_BAR),
        }
    }

    fn mixer_tall(&self) -> Length {
        match self.mixer_window {
            Some(_) => Length::Fill,
            None => Length::Fixed(self.mixer_height - GRAB_BAR),
        }
    }

    pub(crate) fn mixer(&self) -> Element<'_, Message> {
        let palette = self.palette;
        let grab_bar = mouse_area(container(Space::new(Length::Fill, GRAB_BAR)).style(move |_| palette.bar()))
            .on_press(Message::MixerGrabbed)
            .interaction(mouse::Interaction::ResizingVertically);
        column![grab_bar, self.mixer_strips()].into()
    }

    pub(crate) fn mixer_alone(&self) -> Element<'_, Message> {
        container(self.mixer_strips()).width(Length::Fill).height(Length::Fill).into()
    }

    fn mixer_strips(&self) -> Element<'_, Message> {
        let palette = self.palette;
        let strips = self.project.tracks.iter().enumerate().map(|(index, track)| {
            let id = track.id;
            let level = self.track_levels.get(index).copied().unwrap_or(0.0);
            let shown = self
                .project
                .envelope(loupe_engine::Target::TrackGain(id))
                .and_then(|shape| shape.value_at(self.playhead))
                .unwrap_or(track.gain);
            let db = db_from_gain(shown);
            let colour = match track.colour {
                Some([r, g, b]) => Color::from_rgb8(r, g, b),
                None => palette.track(track.id.0.saturating_sub(1) as usize),
            };
            let name: String = track.name.chars().take(NAME_LENGTH).collect();
            let muted = track.muted;
            let soloed = track.solo;
            let count = track.sends.len();
            let wired = count > 0 || track.parent.is_some();
            let routes = if count > 0 { format!("→{count}") } else { "→".to_string() };
            container(
                column![
                    container(Space::new(Length::Fill, 3)).style(move |_| container::Style {
                        background: Some(colour.into()),
                        ..Default::default()
                    }),
                    text(name).size(12).font(palette.medium),
                    self.fx_block(id),
                    hrow![
                        mouse_area(
                            vertical_slider(SILENT_DB..=LOUDEST_DB, db, move |db| Message::TrackGain(id, db))
                                .step(0.1)
                                .default(0.0)
                                .on_release(Message::DragEnd)
                                .height(Length::Fill)
                                .style(move |_, status| palette.slider(status))
                        )
                        .on_scroll(move |delta| Message::WheelOverFader(Level::Track(id), delta))
                        .on_right_press(Message::OpenAutomation(loupe_engine::Target::TrackGain(id)))
                        .on_enter(Message::Hint(Some("Track volume. Scroll to nudge it, right click to automate it.")))
                        .on_exit(Message::Hint(None)),
                        meter(palette, level),
                    ]
                    .spacing(10)
                    .height(Length::Fill)
                    .align_y(Alignment::Center),
                    hrow![self.pan_knob(id, track.pan), self.level_readout(Level::Track(id), shown)].spacing(4).align_y(Alignment::Center),
                    hrow![
                        button(text("M").size(11.5).font(palette.semibold))
                            .padding([3, 7])
                            .style(move |_, status| palette.mute(muted, status))
                            .on_press(Message::ToggleMute(id)),
                        button(text("S").size(11.5).font(palette.semibold))
                            .padding([3, 7])
                            .style(move |_, status| palette.solo(soloed, status))
                            .on_press(Message::ToggleSolo(id)),
                        button(text(routes).size(11.5).font(palette.semibold))
                            .padding([3, 7])
                            .style(move |_, status| palette.toggled(wired, status))
                            .on_press(Message::OpenRouting(id)),
                    ]
                    .spacing(6),
                ]
                .spacing(8)
                .align_x(Alignment::Center),
            )
            .padding(8)
            .width(STRIP_WIDTH)
            .height(self.strip_tall())
            .style(move |_| palette.strip())
            .into()
        });
        let row = iced::widget::row(strips).spacing(8);
        let faders: Element<'_, Message> = if self.project.tracks.is_empty() {
            container(text("Each track gets a fader here.").size(12.5).color(palette.text_dim))
                .center_x(Length::Fill)
                .center_y(self.mixer_tall())
                .into()
        } else {
            container(scrollable(row).direction(Direction::Horizontal(Scrollbar::new())))
                .padding([4, 10])
                .width(Length::Fill)
                .height(self.mixer_tall())
                .into()
        };
        let master = container(self.master_strip())
            .padding([4, 10])
            .height(self.mixer_tall());
        container(iced::widget::row![master, upright_rule(palette), faders].align_y(Alignment::Center))
            .width(Length::Fill)
            .height(self.mixer_tall())
            .style(move |_| palette.bar())
            .into()
    }
}
pub const METER_W: f32 = 14.0;
const FLOOR_DB: f32 = -60.0;
const MID_DB: f32 = -12.0;
const HIGH_DB: f32 = -6.0;

pub struct Meter {
    level: f32,
    palette: crate::theme::Palette,
}

impl canvas::Program<Message> for Meter {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced::Renderer,
        _theme: &iced::Theme,
        bounds: iced::Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let p = self.palette;
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let h = bounds.height;
        let w = bounds.width;
        frame.fill_rectangle(iced::Point::ORIGIN, iced::Size::new(w, h), crate::theme::mix(p.panel, p.background, 0.6));
        let db = 20.0 * self.level.max(1e-6).log10();
        let up = |db: f32| ((db - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0) * h;
        let filled = up(db);
        let zones = [(FLOOR_DB, p.meter_low), (MID_DB, p.meter_mid), (HIGH_DB, p.meter_high)];
        for (i, (from_db, colour)) in zones.iter().enumerate() {
            let from = up(*from_db);
            let to = zones.get(i + 1).map_or(h, |(next, _)| up(*next)).min(filled);
            if to > from {
                let colour = if self.level >= 1.0 { p.danger } else { *colour };
                frame.fill_rectangle(iced::Point::new(0.0, h - to), iced::Size::new(w, to - from), colour);
            }
        }
        for mark in [-6.0, -12.0, -24.0, -48.0] {
            let y = h - up(mark);
            frame.fill_rectangle(iced::Point::new(0.0, y), iced::Size::new(w, 1.0), crate::theme::alpha(p.text_faint, 0.5));
        }
        vec![frame.into_geometry()]
    }
}

pub fn meter(palette: crate::theme::Palette, level: f32) -> Element<'static, Message> {
    let db = if level <= 0.0 { "-inf".to_string() } else { format!("{:.1}", 20.0 * level.log10()) };
    column![
        canvas(Meter { level, palette }).width(METER_W).height(Length::Fill),
        text(db).size(10).font(palette.mono).color(palette.text_faint),
    ]
    .spacing(4)
    .align_x(Alignment::Center)
    .into()
}
