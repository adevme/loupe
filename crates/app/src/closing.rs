use std::path::PathBuf;

use iced::window;

use crate::Pending;

#[derive(Debug, Clone, PartialEq)]
pub enum Next {
    Ask(Pending),
    GoAhead(Pending),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Closing {
    Ignore,
    Redock,
    Quit,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Saving {
    WriteTo(PathBuf),
    AskForAName,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Saved {
    GoOn(Pending),
    ShowWhyNot,
}

pub fn save_before(song: Option<PathBuf>) -> Saving {
    match song {
        Some(song) => Saving::WriteTo(song),
        None => Saving::AskForAName,
    }
}

pub fn after_saving(dirty: bool, pending: Pending) -> Saved {
    match dirty {
        true => Saved::ShowWhyNot,
        false => Saved::GoOn(pending),
    }
}

pub fn ask_first(dirty: bool, tracks: usize, pending: Pending) -> Next {
    match dirty && tracks > 0 {
        true => Next::Ask(pending),
        false => Next::GoAhead(pending),
    }
}

pub fn tear_down(main: window::Id, mixer: Option<window::Id>) -> Vec<window::Id> {
    match mixer {
        Some(mixer) => vec![mixer, main],
        None => vec![main],
    }
}

pub fn window_closed(window: window::Id, main: window::Id, mixer: Option<window::Id>, going: bool) -> Closing {
    if going {
        return Closing::Ignore;
    }
    if mixer == Some(window) {
        return Closing::Redock;
    }
    match window == main {
        true => Closing::Quit,
        false => Closing::Ignore,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn every_way_out() -> Vec<Pending> {
        vec![
            Pending::Quit,
            Pending::Home,
            Pending::Open,
            Pending::New,
            Pending::Template(PathBuf::from("Trap.lp")),
            Pending::Recent(PathBuf::from("Get High.lp")),
            Pending::Dropped(PathBuf::from("Dropped.lp")),
        ]
    }

    #[test]
    fn a_dirty_song_asks_before_every_way_out() {
        for way in every_way_out() {
            assert_eq!(ask_first(true, 3, way.clone()), Next::Ask(way));
        }
    }

    #[test]
    fn a_clean_song_never_asks() {
        for way in every_way_out() {
            assert_eq!(ask_first(false, 3, way.clone()), Next::GoAhead(way));
        }
    }

    #[test]
    fn an_empty_song_never_asks_even_when_dirty() {
        for way in every_way_out() {
            assert_eq!(ask_first(true, 0, way.clone()), Next::GoAhead(way));
        }
    }

    #[test]
    fn saving_writes_the_song_it_already_has_a_file_for() {
        let song = PathBuf::from("/songs/Get High/Get High.lp");
        assert_eq!(save_before(Some(song.clone())), Saving::WriteTo(song));
    }

    #[test]
    fn saving_a_song_with_no_file_yet_asks_for_a_name() {
        assert_eq!(save_before(None), Saving::AskForAName);
    }

    #[test]
    fn a_song_that_saved_goes_on_and_one_that_could_not_says_why() {
        for way in every_way_out() {
            assert_eq!(after_saving(false, way.clone()), Saved::GoOn(way));
            assert_eq!(after_saving(true, Pending::Quit), Saved::ShowWhyNot);
        }
    }

    #[test]
    fn closing_the_main_window_quits() {
        let main = window::Id::unique();
        let mixer = window::Id::unique();
        assert_eq!(window_closed(main, main, Some(mixer), false), Closing::Quit);
        assert_eq!(window_closed(main, main, None, false), Closing::Quit);
    }

    #[test]
    fn going_down_takes_the_mixer_window_with_the_main_one() {
        let main = window::Id::unique();
        let mixer = window::Id::unique();
        assert_eq!(tear_down(main, Some(mixer)), vec![mixer, main]);
        assert_eq!(tear_down(main, None), vec![main]);
    }

    #[test]
    fn closing_the_mixer_window_only_redocks_it() {
        let main = window::Id::unique();
        let mixer = window::Id::unique();
        assert_eq!(window_closed(mixer, main, Some(mixer), false), Closing::Redock);
    }

    #[test]
    fn windows_dying_on_the_way_out_are_ignored() {
        let main = window::Id::unique();
        let mixer = window::Id::unique();
        assert_eq!(window_closed(main, main, Some(mixer), true), Closing::Ignore);
        assert_eq!(window_closed(mixer, main, Some(mixer), true), Closing::Ignore);
    }

    #[test]
    fn a_window_we_do_not_know_is_ignored() {
        let main = window::Id::unique();
        let stranger = window::Id::unique();
        assert_eq!(window_closed(stranger, main, None, false), Closing::Ignore);
    }
}
