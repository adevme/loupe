use iced::Element;

use loupe_engine::TrackId;
use loupe_stock_ui::{Change, ChorusEditor, CompressorEditor, DeesserEditor, SaturationEditor, DelayEditor, EqEditor, EqMessage, LimiterEditor, Look, ReverbEditor};

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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spot {
    Track(TrackId),
    Clip(loupe_engine::ClipId),
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
        }
    }
}

impl App {
    pub(crate) fn stock_sheet(&self) -> Element<'_, Message> {
        let Some(window) = &self.stock else {
            return iced::widget::text("No plugin is open.").size(13).into();
        };
        let body = iced::widget::container(window.view()).height(iced::Length::Fixed(470.0));
        self.window(window.name.clone(), body.into(), 1060.0)
    }
}
