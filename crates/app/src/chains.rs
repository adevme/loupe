use std::fs;
use std::path::{Path, PathBuf};

use iced::widget::{button, column, row, scrollable, text, text_input};
use iced::{Alignment, Element, Length};
use loupe_engine::{chain_from, chain_text, Command, Fx, Outcome};

use crate::{settings, App, Message, Overlay};

const EXTENSION: &str = "chain";

pub fn saved_chains(folder: &Path) -> Vec<(String, PathBuf)> {
    let mut found: Vec<(String, PathBuf)> = fs::read_dir(folder)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == EXTENSION))
        .filter_map(|path| Some((path.file_stem()?.to_string_lossy().to_string(), path)))
        .collect();
    found.sort_by_key(|(name, _)| name.to_lowercase());
    found
}

impl App {
    fn chains_folder(&self) -> PathBuf {
        settings::chains_folder(self.folder.as_deref())
    }

    pub(crate) fn refresh_chains(&mut self) {
        self.chains = saved_chains(&self.chains_folder());
    }

    fn chain_here(&self) -> Option<Vec<Fx>> {
        match &self.overlay {
            Overlay::Plugins(track) => {
                let record = self.rec_shown.contains(track);
                Some(self.project.track(*track)?.fx.iter().filter(|fx| fx.record == record).cloned().collect())
            }
            Overlay::ClipPlugins(clip) => Some(self.project.clip(*clip)?.fx.clone()),
            Overlay::MasterPlugins => Some(self.project.master_fx.clone()),
            _ => None,
        }
    }

    pub(crate) fn save_chain(&mut self) {
        let name = crate::files::file_safe(&self.chain_name);
        if name.is_empty() {
            self.notice = Some("Give the chain a name first.".into());
            return;
        }
        self.gather_fx_state();
        let chain = self.chain_here().unwrap_or_default();
        if chain.is_empty() {
            self.notice = Some("There are no plugins here to save.".into());
            return;
        }
        let folder = self.chains_folder();
        let file = folder.join(format!("{name}.{EXTENSION}"));
        let written = fs::create_dir_all(&folder).and_then(|_| fs::write(&file, chain_text(&chain)));
        match written {
            Ok(()) => {
                self.notice = Some(format!("Saved the chain \"{name}\"."));
                self.chain_name.clear();
                self.refresh_chains();
            }
            Err(why) => self.problem = Some(format!("Could not save the chain: {}: {why}", file.display())),
        }
    }

    pub(crate) fn use_chain(&mut self, file: PathBuf) {
        let chain = match fs::read_to_string(&file).map_err(|why| why.to_string()).and_then(|text| chain_from(&text)) {
            Ok(chain) => chain,
            Err(why) => {
                self.problem = Some(format!("Could not open the chain: {}: {why}", file.display()));
                return;
            }
        };
        let overlay = std::mem::replace(&mut self.overlay, Overlay::None);
        let record = match &overlay {
            Overlay::Plugins(track) => self.rec_shown.contains(track),
            _ => false,
        };
        self.transact(None, |project| {
            for fx in chain {
                let fx = Fx { record, ..fx };
                match &overlay {
                    Overlay::Plugins(track) => project.apply(Command::AddFx { track: *track, fx })?,
                    Overlay::ClipPlugins(clip) => project.apply(Command::AddClipFx { clip: *clip, fx })?,
                    Overlay::MasterPlugins => project.apply(Command::AddMasterFx(fx))?,
                    _ => return Ok(Outcome::Done),
                };
            }
            Ok(Outcome::Done)
        });
        if let Overlay::ClipPlugins(clip) = overlay {
            self.overlay = Overlay::Clip(clip);
        }
    }

    pub(crate) fn chain_block(&self) -> Option<Element<'_, Message>> {
        self.chain_here()?;
        let palette = self.palette;
        let mut saved = row![].spacing(6);
        for (name, file) in &self.chains {
            saved = saved.push(
                button(text(name.clone()).size(12.5))
                    .padding([4, 10])
                    .style(move |_, status| palette.outlined(status))
                    .on_press(Message::UseChain(file.clone())),
            );
        }
        let list: Element<'_, Message> = if self.chains.is_empty() {
            text("No chains saved yet.").size(12.5).color(palette.text_dim).into()
        } else {
            scrollable(saved).direction(scrollable::Direction::Horizontal(scrollable::Scrollbar::new().width(4).scroller_width(4))).into()
        };
        let name = text_input("Name this chain", &self.chain_name)
            .on_input(Message::ChainName)
            .on_submit(Message::SaveChain)
            .size(13)
            .padding([5, 10])
            .style(move |_, status| palette.field(status));
        let save = button(text("Save these plugins as a chain").size(12.5))
            .padding([5, 10])
            .style(move |_, status| palette.outlined(status))
            .on_press(Message::SaveChain);
        Some(
            column![
                text("Chains").size(12).color(palette.text_dim),
                list,
                row![name, save].spacing(8).align_y(Alignment::Center).width(Length::Fill),
            ]
            .spacing(6)
            .into(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_chain_files_are_listed_in_name_order() {
        let folder = std::env::temp_dir().join(format!("loupe-chains-{}", std::process::id()));
        fs::create_dir_all(&folder).unwrap();
        for file in ["vocal.chain", "Drum bus.chain", "notes.txt"] {
            fs::write(folder.join(file), "loupe chain 1\n").unwrap();
        }
        let names: Vec<String> = saved_chains(&folder).into_iter().map(|(name, _)| name).collect();
        assert_eq!(names, vec!["Drum bus", "vocal"]);
        fs::remove_dir_all(&folder).unwrap();
    }
}
