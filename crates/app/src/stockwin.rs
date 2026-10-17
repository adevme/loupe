use iced::widget::container;
use iced::{window, Element, Length, Size, Task};

use loupe_engine::TrackId;
use loupe_stock_ui::{
    Change, ChorusEditor, CompressorEditor, DeesserEditor, DelayEditor, EqEditor, EqMessage, GateEditor, LimiterEditor,
    Look, MeterEditor, MultibandEditor, ReverbEditor, SaturationEditor, TransientEditor, TuneEditor, KeyEditor, KeyMessage,
    RiderEditor, DoublerEditor, DenoiseEditor,
};

use crate::racks::Peek;
use crate::{App, Message};

pub enum Face {
    Eq(Box<EqEditor>),
    Compressor(Box<CompressorEditor>),
    Limiter(Box<LimiterEditor>),
    Delay(Box<DelayEditor>),
    Reverb(Box<ReverbEditor>),
    Deesser(Box<DeesserEditor>),
    Saturation(Box<SaturationEditor>),
    Chorus(Box<ChorusEditor>),
    Transient(Box<TransientEditor>),
    Gate(Box<GateEditor>),
    Meter(Box<MeterEditor>),
    Multiband(Box<MultibandEditor>),
    Tune(Box<TuneEditor>),
    Key(Box<KeyEditor>),
    Rider(Box<RiderEditor>),
    Doubler(Box<DoublerEditor>),
    Denoise(Box<DenoiseEditor>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Spot {
    Track(TrackId),
    Clip(loupe_engine::ClipId),
    Master,
    Monitor,
}

pub struct Window {
    pub spot: Spot,
    pub slot: usize,
    pub name: String,
    pub face: Face,
}

impl Window {
    pub fn open(spot: Spot, slot: usize, which: usize, name: &str, values: &[f32], peek: Peek, rate: f32, bpm: f32) -> Option<Self> {
        let look = Look::default();
        let mut face = match which {
            0 => Face::Eq(Box::new(EqEditor::new(rate, peek.scopes.clone(), look))),
            1 => Face::Compressor(Box::new(CompressorEditor::new(peek.history.clone(), look))),
            2 => Face::Limiter(Box::new(LimiterEditor::new(peek.history.clone(), look))),
            3 => Face::Delay(Box::new(DelayEditor::new(bpm, look))),
            4 => Face::Reverb(Box::new(ReverbEditor::new(look))),
            5 => Face::Deesser(Box::new(DeesserEditor::new(peek.history.clone(), look))),
            6 => Face::Saturation(Box::new(SaturationEditor::new(look))),
            7 => Face::Chorus(Box::new(ChorusEditor::new(look))),
            8 => Face::Transient(Box::new(TransientEditor::new(peek.history.clone(), look))),
            9 => Face::Gate(Box::new(GateEditor::new(peek.history.clone(), look))),
            10 => Face::Meter(Box::new(MeterEditor::new(peek.meter.clone(), look))),
            11 => Face::Multiband(Box::new(MultibandEditor::new(peek.history.clone(), look))),
            12 => Face::Tune(Box::new(TuneEditor::new(peek.history.clone(), look))),
            13 => Face::Key(Box::new(KeyEditor::new(peek.findings.clone(), look))),
            14 => Face::Rider(Box::new(RiderEditor::new(peek.history.clone(), look))),
            15 => Face::Doubler(Box::new(DoublerEditor::new(look))),
            16 => Face::Denoise(Box::new(DenoiseEditor::new(peek.spectra.clone(), rate, look))),
            _ => return None,
        };
        if !values.is_empty() {
            match &mut face {
                Face::Eq(editor) => editor.load(values),
                Face::Compressor(editor) => editor.load(values),
                Face::Limiter(editor) => editor.load(values),
                Face::Delay(editor) => editor.load(values),
                Face::Reverb(editor) => editor.load(values),
                Face::Deesser(editor) => editor.load(values),
                Face::Saturation(editor) => editor.load(values),
                Face::Chorus(editor) => editor.load(values),
                Face::Transient(editor) => editor.load(values),
                Face::Gate(editor) => editor.load(values),
                Face::Meter(editor) => editor.load(values),
                Face::Multiband(editor) => editor.load(values),
                Face::Tune(editor) => editor.load(values),
                Face::Rider(editor) => editor.load(values),
                Face::Doubler(editor) => editor.load(values),
                Face::Denoise(editor) => editor.load(values),
                Face::Key(_) => {}
            }
        }
        Some(Self { spot, slot, name: name.to_string(), face })
    }

    pub fn tick(&mut self) {
        match &mut self.face {
            Face::Eq(editor) => editor.tick(),
            Face::Compressor(editor) => editor.tick(),
            Face::Limiter(editor) => editor.tick(),
            Face::Deesser(editor) => editor.tick(),
            Face::Transient(editor) => editor.tick(),
            Face::Gate(editor) => editor.tick(),
            Face::Meter(editor) => editor.tick(),
            Face::Multiband(editor) => editor.tick(),
            Face::Tune(editor) => editor.tick(),
            Face::Rider(editor) => editor.tick(),
            Face::Denoise(editor) => editor.tick(),
            Face::Doubler(_) => {}
            Face::Key(editor) => editor.tick(),
            _ => {}
        }
    }

    pub fn turned(&mut self, change: Change) -> Vec<(usize, f32)> {
        match &mut self.face {
            Face::Eq(_) => Vec::new(),
            Face::Compressor(editor) => editor.update(change),
            Face::Limiter(editor) => editor.update(change),
            Face::Delay(editor) => editor.update(change),
            Face::Reverb(editor) => editor.update(change),
            Face::Deesser(editor) => editor.update(change),
            Face::Saturation(editor) => editor.update(change),
            Face::Chorus(editor) => editor.update(change),
            Face::Transient(editor) => editor.update(change),
            Face::Gate(editor) => editor.update(change),
            Face::Meter(editor) => editor.update(change),
            Face::Multiband(editor) => editor.update(change),
            Face::Tune(editor) => editor.update(change),
            Face::Rider(editor) => editor.update(change),
            Face::Doubler(editor) => editor.update(change),
            Face::Denoise(editor) => editor.update(change),
            Face::Key(_) => Vec::new(),
        }
    }

    pub fn told_key(&mut self, message: KeyMessage) -> Option<(Option<f32>, Option<u8>)> {
        let Face::Key(editor) = &mut self.face else { return None };
        let send = message == KeyMessage::Send;
        editor.update(message);
        send.then(|| editor.chosen())
    }

    pub fn key_editor(&mut self) -> Option<&mut KeyEditor> {
        match &mut self.face {
            Face::Key(editor) => Some(editor),
            _ => None,
        }
    }

    pub fn told(&mut self, message: EqMessage) -> Vec<(usize, f32)> {
        match &mut self.face {
            Face::Eq(editor) => editor.update(message),
            _ => Vec::new(),
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        match &self.face {
            Face::Eq(editor) => editor.view().map(Message::StockTold),
            Face::Compressor(editor) => editor.view().map(Message::StockTurned),
            Face::Limiter(editor) => editor.view().map(Message::StockTurned),
            Face::Delay(editor) => editor.view().map(Message::StockTurned),
            Face::Reverb(editor) => editor.view().map(Message::StockTurned),
            Face::Deesser(editor) => editor.view().map(Message::StockTurned),
            Face::Saturation(editor) => editor.view().map(Message::StockTurned),
            Face::Chorus(editor) => editor.view().map(Message::StockTurned),
            Face::Transient(editor) => editor.view().map(Message::StockTurned),
            Face::Gate(editor) => editor.view().map(Message::StockTurned),
            Face::Meter(editor) => editor.view().map(Message::StockTurned),
            Face::Multiband(editor) => editor.view().map(Message::StockTurned),
            Face::Tune(editor) => editor.view().map(Message::StockTurned),
            Face::Rider(editor) => editor.view().map(Message::StockTurned),
            Face::Doubler(editor) => editor.view().map(Message::StockTurned),
            Face::Denoise(editor) => editor.view().map(Message::StockTurned),
            Face::Key(editor) => editor.view().map(Message::KeyTold),
        }
    }
}

/// A stock plugin's editor in its own window, like a third-party plugin gets.
pub struct Open {
    pub id: window::Id,
    pub editor: Window,
}

const STOCK_WINDOW: Size = Size::new(1060.0, 560.0);
const SMALLEST_STOCK_WINDOW: Size = Size::new(720.0, 420.0);

impl App {
    pub(crate) fn stock_at(&self, window: window::Id) -> Option<&Open> {
        self.stocks.iter().find(|open| open.id == window)
    }

    /// The editor in the window that has focus, which is the one being used.
    pub(crate) fn stock_mut(&mut self) -> Option<&mut Window> {
        let focused = self.focused;
        self.stocks.iter_mut().find(|open| open.id == focused).map(|open| &mut open.editor)
    }

    pub(crate) fn stock_ref(&self) -> Option<&Window> {
        self.stock_at(self.focused).map(|open| &open.editor)
    }

    /// Opens a window for each editor asked for, brings an already open one to the front,
    /// and closes the windows whose plugin is gone.
    pub(crate) fn settle_stock_windows(&mut self) -> Task<Message> {
        let mut tasks = Vec::new();
        for editor in std::mem::take(&mut self.stocks_waiting) {
            if let Some(open) = self.stocks.iter_mut().find(|open| open.editor.spot == editor.spot && open.editor.slot == editor.slot) {
                open.editor = editor;
                tasks.push(window::gain_focus(open.id));
                continue;
            }
            let (id, opening) = window::open(window::Settings {
                size: STOCK_WINDOW,
                min_size: Some(SMALLEST_STOCK_WINDOW),
                icon: window::icon::from_file_data(include_bytes!("../assets/icon.png"), None).ok(),
                exit_on_close_request: false,
                ..window::Settings::default()
            });
            self.stocks.push(Open { id, editor });
            tasks.push(opening.map(Message::WindowFocused));
        }
        let gone: Vec<window::Id> = self.stocks.iter().filter(|open| !self.still_there(&open.editor)).map(|open| open.id).collect();
        if !gone.is_empty() {
            self.stocks.retain(|open| !gone.contains(&open.id));
            if gone.contains(&self.focused) {
                self.focused = self.main_window;
            }
            tasks.extend(gone.into_iter().map(window::close));
        }
        Task::batch(tasks)
    }

    fn still_there(&self, editor: &Window) -> bool {
        let fx = match editor.spot {
            Spot::Track(track) => self.project.track(track).and_then(|found| found.fx.get(editor.slot)),
            Spot::Clip(clip) => self.project.clip(clip).and_then(|found| found.fx.get(editor.slot)),
            Spot::Master => self.project.master_fx.get(editor.slot),
            Spot::Monitor => self.project.monitor_fx.get(editor.slot),
        };
        fx.is_some_and(|fx| loupe_plugins::rack::is_built_in(&fx.path) && fx.name == editor.name)
    }

    pub(crate) fn stock_window_view<'a>(&'a self, open: &'a Open) -> Element<'a, Message> {
        let palette = self.palette;
        let id = open.id;
        container(open.editor.view().map(move |message| Message::FromStock(id, Box::new(message))))
            .padding(20)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| container::Style { background: Some(palette.panel.into()), ..Default::default() })
            .into()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use loupe_engine::{Command, Fx, Outcome};

    use super::*;
    use crate::Overlay;

    fn eq_on_vox() -> (App, TrackId) {
        let mut app = App::bare();
        let Ok(Outcome::Track(track)) = app.project.apply(Command::AddTrack { name: "Vox".into() }) else { panic!("no track") };
        let fx = Fx { mix: 1.0, path: PathBuf::from(loupe_plugins::BUILT_IN), index: 0, name: "Loupe EQ".into(), bypassed: false, state: Vec::new(), record: false };
        app.project.apply(Command::AddFx { track, fx }).unwrap();
        (app, track)
    }

    #[test]
    fn a_stock_plugin_opens_in_its_own_window_not_over_the_song() {
        let (mut app, track) = eq_on_vox();
        let _ = app.update(Message::ShowPlugin(track, 0));
        assert_eq!(app.stocks.len(), 1, "the EQ did not get a window");
        assert_eq!(app.overlay, Overlay::None, "the EQ covered the song");
        let id = app.stocks[0].id;
        assert_eq!(app.title_of(id), "Loupe EQ");
        let _ = app.update(Message::ShowPlugin(track, 0));
        assert_eq!(app.stocks.len(), 1, "opening it again made a second window");
        let _ = app.update(Message::WindowClosed(id));
        assert!(app.stocks.is_empty(), "closing the window kept the editor");
        assert!(!app.going, "closing the EQ started to quit Loupe");
    }

    #[test]
    fn removing_the_plugin_closes_its_window() {
        let (mut app, track) = eq_on_vox();
        let _ = app.update(Message::ShowPlugin(track, 0));
        let _ = app.update(Message::RemovePlugin(track, 0));
        assert!(app.stocks.is_empty(), "the window outlived its plugin");
    }
}
