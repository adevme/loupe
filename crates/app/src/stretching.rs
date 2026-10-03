use std::sync::Arc;

use iced::futures::channel::oneshot;
use iced::Task;
use loupe_engine::{ClipId, Command, Frames, Source};

use crate::{App, Message, Run};

const KEPT_BYTES: usize = 256 << 20;

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
        while self.made.len() > 1 && self.held_bytes() > KEPT_BYTES {
            self.made.remove(0);
        }
    }

    fn held_bytes(&self) -> usize {
        let frame = std::mem::size_of::<[f32; 2]>();
        self.made.iter().map(|(_, _, made)| made.frames.len() * frame).sum()
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

#[cfg(test)]
mod tests {
    use super::*;
    use loupe_engine::Source;

    fn take(seconds: usize) -> Arc<Source> {
        Arc::new(Source::from_frames("take", vec![[0.1, 0.1]; seconds * 48_000]))
    }

    #[test]
    fn the_cache_lets_go_of_old_stretches_once_it_is_full() {
        let mut kept = Stretches::default();
        for n in 0..20 {
            let from = take(60);
            kept.keep(from, 1.0 + n as f64, take(60));
        }
        assert!(kept.held_bytes() <= KEPT_BYTES, "it held {} bytes", kept.held_bytes());
        assert!(!kept.made.is_empty(), "it should keep at least the newest");
    }

    #[test]
    fn a_single_huge_stretch_is_still_kept() {
        let mut kept = Stretches::default();
        let from = take(600);
        kept.keep(from.clone(), 2.0, take(600));
        assert_eq!(kept.made.len(), 1);
        assert!(kept.find(&from, 2.0).is_some());
    }
}
