use std::sync::Arc;

use iced::futures::channel::oneshot;
use iced::Task;
use loupe_engine::{ClipId, Command, Frames, Source};

use crate::{App, Message, Run};

const KEPT_STRETCHES: usize = 24;

#[derive(Default)]
pub struct Stretches {
    made: Vec<(Arc<Source>, f64, Arc<Source>)>,
    making: Vec<Arc<Source>>,
}

impl Stretches {
    fn find(&self, source: &Arc<Source>, stretch: f64) -> Option<Arc<Source>> {
        self.made.iter().find(|(from, s, _)| *s == stretch && Arc::ptr_eq(from, source)).map(|(_, _, made)| made.clone())
    }

    fn keep(&mut self, source: Arc<Source>, stretch: f64, made: Arc<Source>) {
        self.making.retain(|busy| !Arc::ptr_eq(busy, &source));
        self.made.retain(|(from, s, _)| !(*s == stretch && Arc::ptr_eq(from, &source)));
        self.made.push((source, stretch, made));
        if self.made.len() > KEPT_STRETCHES {
            self.made.remove(0);
        }
    }
}

impl App {
    pub(crate) fn stretch_clip(&mut self, clip: ClipId, start: Frames, len: Frames) {
        let Some(found) = self.project.clip(clip) else {
            return;
        };
        let stretch = found.stretch * len as f64 / found.len as f64;
        let Some(track) = self.project.track_of(clip).map(|track| track.id) else {
            return;
        };
        self.transact(Some(Run::Stretch(clip)), |project| {
            project.apply(Command::SetStretch { clip, stretch })?;
            project.apply(Command::MoveClip { clip, track, start })
        });
    }

    pub(crate) fn stretch_waiting(&mut self) -> Task<Message> {
        let waiting: Vec<(Arc<Source>, f64)> = self
            .project
            .clips()
            .filter(|clip| clip.waiting_for_stretch() && !clip.source.frames.is_empty())
            .map(|clip| (clip.source.clone(), clip.stretch))
            .collect();
        if waiting.is_empty() {
            return Task::none();
        }
        let mut filled = false;
        let mut tasks = Vec::new();
        for (source, stretch) in waiting {
            if let Some(stretched) = self.stretches.find(&source, stretch) {
                let _ = self.project.apply(Command::FillStretch { source, stretch, stretched });
                filled = true;
                continue;
            }
            if self.stretches.making.iter().any(|busy| Arc::ptr_eq(busy, &source)) {
                continue;
            }
            self.stretches.making.push(source.clone());
            let rate = self.project.rate;
            let (done, made) = oneshot::channel();
            let from = source.clone();
            std::thread::spawn(move || {
                let _ = done.send(Arc::new(from.stretched(rate, stretch)));
            });
            tasks.push(Task::perform(async move { made.await.ok() }, move |made| Message::Stretched {
                source: source.clone(),
                stretch,
                made,
            }));
        }
        if filled {
            self.engine.set_project(&self.project);
            self.cache.clear();
        }
        Task::batch(tasks)
    }

    pub(crate) fn stretched(&mut self, source: Arc<Source>, stretch: f64, made: Option<Arc<Source>>) {
        match made {
            Some(made) => self.stretches.keep(source, stretch, made),
            None => self.stretches.making.retain(|busy| !Arc::ptr_eq(busy, &source)),
        }
    }
}
