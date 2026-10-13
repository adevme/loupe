pub const HELD_AT: f64 = 0.5;
const PAGE_LEAD: f64 = 0.1;
const PAGE_EDGE_PX: f64 = 24.0;
const ROOM_AFTER_ROUND: f64 = 0.05;
const CATCH_UP: f64 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Follow {
    #[default]
    Smooth,
    Page,
}

impl Follow {
    pub const ALL: [Follow; 2] = [Follow::Smooth, Follow::Page];

    pub fn saved_as(self) -> &'static str {
        match self {
            Follow::Smooth => "smooth",
            Follow::Page => "page",
        }
    }

    pub fn from_saved(value: Option<&str>) -> Self {
        match value {
            Some("page") => Follow::Page,
            _ => Follow::Smooth,
        }
    }

    pub fn scroll(self, sight: Sight, was: f64, at: f64, round: Option<(f64, f64)>) -> f64 {
        match self {
            Follow::Page => paged(sight, at),
            Follow::Smooth => on_whole_pixels(sight, smooth(sight, was, at, round)),
        }
    }
}

impl std::fmt::Display for Follow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Follow::Smooth => write!(f, "Smooth"),
            Follow::Page => write!(f, "Page"),
        }
    }
}

pub fn round_at(at: f64, loop_range: Option<(f64, f64)>, end: f64, endless: bool) -> Option<(f64, f64)> {
    if endless {
        return None;
    }
    if let Some((from, to)) = loop_range.filter(|(from, to)| (*from..*to).contains(&at)) {
        return Some((from, to));
    }
    (end > 0.0 && at < end).then_some((0.0, end))
}

fn on_whole_pixels(sight: Sight, scroll: f64) -> f64 {
    match scroll == sight.scroll {
        true => scroll,
        false => ((scroll * sight.zoom).round() / sight.zoom).max(0.0),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sight {
    pub scroll: f64,
    pub zoom: f64,
    pub width: f64,
}

impl Sight {
    fn seen(self) -> f64 {
        self.width / self.zoom
    }
}

pub fn paged(sight: Sight, at: f64) -> f64 {
    let x = (at - sight.scroll) * sight.zoom;
    match x > sight.width - PAGE_EDGE_PX || x < 0.0 {
        true => (at - sight.width * PAGE_LEAD / sight.zoom).max(0.0),
        false => sight.scroll,
    }
}

pub fn smooth(sight: Sight, was: f64, at: f64, round: Option<(f64, f64)>) -> f64 {
    let seen = sight.seen();
    let held = seen * HELD_AT;
    let on_screen = at - sight.scroll;
    if !(0.0..=seen).contains(&on_screen) {
        return jumped(at - held, seen, round);
    }
    if on_screen <= held {
        return sight.scroll;
    }
    let stood = (was - sight.scroll).clamp(held, on_screen);
    let caught_up = (stood - (at - was).max(0.0) * CATCH_UP).max(held);
    let place = match caught_up - held <= 0.5 / sight.zoom {
        true => held,
        false => caught_up,
    };
    let slid = at - place;
    let ceiling = last_scroll_in(seen, round).map_or(slid, |last| last.max(sight.scroll));
    slid.min(ceiling).max(sight.scroll)
}

fn last_scroll_in(seen: f64, round: Option<(f64, f64)>) -> Option<f64> {
    round.map(|(_, to)| to - seen * (1.0 - ROOM_AFTER_ROUND))
}

fn jumped(centred: f64, seen: f64, round: Option<(f64, f64)>) -> f64 {
    let placed = match (round, last_scroll_in(seen, round)) {
        (Some((from, _)), Some(last)) if last <= from => centred.clamp(last, from),
        (_, Some(last)) => centred.min(last),
        _ => centred,
    };
    placed.max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: f64 = 1.0 / 60.0;

    fn sight(scroll: f64, zoom: f64, width: f64) -> Sight {
        Sight { scroll, zoom, width }
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn page_holds_until_the_playhead_nears_the_right_edge() {
        let view = sight(0.0, 100.0, 1000.0);
        assert_eq!(paged(view, 5.0), 0.0);
        assert_eq!(paged(view, 9.75), 0.0, "just inside the 24 px edge");
    }

    #[test]
    fn page_jumps_a_page_and_leaves_the_playhead_a_tenth_in() {
        let view = sight(0.0, 100.0, 1000.0);
        assert!(close(paged(view, 9.8), 8.8));
        let zoomed = sight(0.0, 400.0, 1000.0);
        assert!(close(paged(zoomed, 2.5), 2.25));
    }

    #[test]
    fn page_brings_back_a_playhead_left_of_the_view_and_never_goes_below_zero() {
        assert!(close(paged(sight(10.0, 100.0, 1000.0), 3.0), 2.0));
        assert_eq!(paged(sight(10.0, 100.0, 1000.0), 0.5), 0.0);
    }

    #[test]
    fn smooth_holds_still_until_the_playhead_reaches_the_middle() {
        let view = sight(0.0, 100.0, 1000.0);
        assert_eq!(smooth(view, 2.0, 2.0 + FRAME, None), 0.0);
        assert_eq!(smooth(view, 5.0 - FRAME, 5.0, None), 0.0);
    }

    #[test]
    fn smooth_keeps_the_playhead_exactly_in_the_middle_once_it_gets_there() {
        let mut scroll = 0.0;
        let mut was = 4.9;
        for step in 1..600 {
            let at = 4.9 + step as f64 * FRAME;
            scroll = smooth(sight(scroll, 100.0, 1000.0), was, at, None);
            if at > 5.0 {
                assert!(close(at - scroll, 5.0), "frame {step} drifted to {}", at - scroll);
            }
            was = at;
        }
    }

    #[test]
    fn smooth_moves_by_exactly_what_the_playhead_moved_at_any_zoom() {
        for zoom in [10.0, 100.0, 2_000.0, 48_000.0] {
            let view = sight(30.0, zoom, 1200.0);
            let held = 30.0 + 600.0 / zoom;
            let step = 0.25 / zoom;
            let scroll = smooth(view, held, held + step, None);
            assert!(close(scroll, 30.0 + step), "zoom {zoom} moved {}", scroll - 30.0);
        }
    }

    #[test]
    fn smooth_never_scrolls_left_while_the_playhead_is_on_screen() {
        let view = sight(20.0, 100.0, 1000.0);
        assert_eq!(smooth(view, 28.0, 21.0, None), 20.0);
        assert!(smooth(view, 28.0, 27.0, None) >= 20.0);
    }

    #[test]
    fn a_seek_into_the_right_half_eases_back_to_the_middle_without_a_jump_or_overshoot() {
        let mut scroll = 0.0;
        let mut was = 9.0;
        let mut last_place = f64::MAX;
        for step in 1..240 {
            let at = 9.0 + step as f64 * FRAME;
            let before = scroll;
            scroll = smooth(sight(scroll, 100.0, 1000.0), was, at, None);
            let place = at - scroll;
            assert!(scroll - before < 0.1, "a jump of {} s", scroll - before);
            assert!(place >= 5.0 - 1e-9, "overshot the middle to {place}");
            assert!(place <= last_place + 1e-9, "the playhead moved right on screen");
            last_place = place;
            was = at;
        }
        assert!(last_place - 5.0 < 0.01, "still {} s right of the middle", last_place - 5.0);
    }

    #[test]
    fn smooth_centres_a_playhead_that_left_the_view() {
        assert!(close(smooth(sight(0.0, 100.0, 1000.0), 40.0, 40.0 + FRAME, None), 35.0 + FRAME));
        assert!(close(smooth(sight(50.0, 100.0, 1000.0), 12.0, 12.0 + FRAME, None), 7.0 + FRAME));
        assert_eq!(smooth(sight(50.0, 100.0, 1000.0), 2.0, 2.0 + FRAME, None), 0.0);
    }

    #[test]
    fn a_loop_longer_than_the_view_jumps_back_once_per_wrap_to_put_its_start_in_the_middle() {
        let round = Some((20.0, 60.0));
        let at_the_end = sight(50.5, 100.0, 1000.0);
        let wrapped = smooth(at_the_end, 59.99, 20.0, round);
        assert!(close(wrapped, 15.0));
        assert!(close(smooth(sight(wrapped, 100.0, 1000.0), 20.0, 20.0 + FRAME, round), wrapped + FRAME));
    }

    #[test]
    fn the_view_stops_before_the_end_of_the_round_so_its_end_stays_in_sight() {
        let round = Some((20.0, 60.0));
        let mut scroll = 40.0;
        let mut was = 45.0;
        for step in 1..900 {
            let at = 45.0 + step as f64 * FRAME;
            if at >= 60.0 {
                break;
            }
            scroll = smooth(sight(scroll, 100.0, 1000.0), was, at, round);
            assert!(scroll <= 50.5 + 1e-9, "scrolled past the loop end to {scroll}");
            was = at;
        }
        assert!(close(scroll, 50.5));
    }

    #[test]
    fn a_loop_that_fits_settles_and_then_never_moves_again() {
        let round = Some((12.0, 18.0));
        let mut scroll = 0.0;
        let mut was = 12.0;
        let mut settled = None;
        for step in 1..2_000 {
            let mut at = 12.0 + step as f64 * FRAME;
            while at >= 18.0 {
                at -= 6.0;
            }
            let before = scroll;
            scroll = smooth(sight(scroll, 100.0, 1000.0), was, at, round);
            assert!(scroll >= before, "the view went back at {at}");
            assert!((scroll..=scroll + 10.0).contains(&12.0) && (scroll..=scroll + 10.0).contains(&18.0), "the loop left the view");
            if step > 400 {
                assert_eq!(scroll, *settled.get_or_insert(scroll), "still moving at {at}");
            }
            was = at;
        }
    }

    #[test]
    fn a_wrap_into_a_fitting_loop_shows_all_of_it() {
        let round = Some((40.0, 47.0));
        let scroll = smooth(sight(0.0, 100.0, 1000.0), 3.0, 40.0, round);
        assert!(close(scroll, 37.5));
        assert!(scroll <= 40.0 && scroll + 10.0 >= 47.0);
    }

    #[test]
    fn endless_recording_slides_on_past_where_a_round_would_stop() {
        let free = smooth(sight(50.5, 100.0, 1000.0), 55.5, 55.5 + FRAME, None);
        assert!(close(free, 50.5 + FRAME));
        let capped = smooth(sight(50.5, 100.0, 1000.0), 55.5, 55.5 + FRAME, Some((0.0, 60.0)));
        assert_eq!(capped, 50.5);
    }

    #[test]
    fn a_short_song_near_the_start_never_scrolls_below_zero() {
        assert_eq!(smooth(sight(0.0, 100.0, 1000.0), 0.0, 3.0, Some((0.0, 4.0))), 0.0);
        assert_eq!(smooth(sight(30.0, 100.0, 1000.0), 0.0, 1.0, Some((0.0, 4.0))), 0.0);
    }

    #[test]
    fn follow_is_saved_and_read_back_and_starts_smooth() {
        for follow in Follow::ALL {
            assert_eq!(Follow::from_saved(Some(follow.saved_as())), follow);
        }
        assert_eq!(Follow::from_saved(None), Follow::Smooth);
        assert_eq!(Follow::from_saved(Some("nonsense")), Follow::Smooth);
        assert_eq!(Follow::default(), Follow::Smooth);
        assert_eq!(Follow::Page.saved_as(), "page");
        assert_eq!(Follow::Smooth.saved_as(), "smooth");
    }

    #[test]
    fn page_follow_is_exactly_the_old_page_turn() {
        for (scroll, zoom, width, at) in [(0.0, 100.0, 1000.0, 9.8), (3.0, 37.5, 811.0, 24.0), (10.0, 100.0, 1000.0, 3.0), (0.0, 100.0, 1000.0, 5.0)] {
            let view = sight(scroll, zoom, width);
            let x = (at - scroll) * zoom;
            let old = match x > width - 24.0 || x < 0.0 {
                true => (at - width * 0.1 / zoom).max(0.0),
                false => scroll,
            };
            assert_eq!(Follow::Page.scroll(view, at - FRAME, at, Some((0.0, 100.0))), old);
        }
    }

    #[test]
    fn the_round_is_the_loop_while_inside_it_then_the_song_and_none_when_recording_on() {
        assert_eq!(round_at(25.0, Some((20.0, 30.0)), 60.0, false), Some((20.0, 30.0)));
        assert_eq!(round_at(35.0, Some((20.0, 30.0)), 60.0, false), Some((0.0, 60.0)));
        assert_eq!(round_at(30.0, Some((20.0, 30.0)), 60.0, false), Some((0.0, 60.0)));
        assert_eq!(round_at(61.0, None, 60.0, false), None);
        assert_eq!(round_at(5.0, None, 0.0, false), None);
        assert_eq!(round_at(25.0, Some((20.0, 30.0)), 60.0, true), None);
    }

    #[test]
    fn smooth_moves_only_by_whole_pixels_and_holds_the_playhead_within_half_a_pixel_of_the_middle() {
        for zoom in [7.3, 100.0, 333.3, 4_800.0] {
            let width = 1001.0;
            let mut scroll = 0.0;
            let mut was = 0.0;
            for step in 1..2_000 {
                let at = step as f64 * FRAME * 100.0 / zoom * 1.37;
                let before = scroll;
                scroll = Follow::Smooth.scroll(sight(scroll, zoom, width), was, at, None);
                let moved = (scroll - before) * zoom;
                assert!((moved - moved.round()).abs() < 1e-6, "zoom {zoom} moved {moved} px");
                assert!(moved >= 0.0, "zoom {zoom} went back");
                if (at - scroll) * zoom > width * HELD_AT + 1.0 {
                    panic!("zoom {zoom} playhead at {} px", (at - scroll) * zoom);
                }
                if at * zoom > width {
                    assert!(((at - scroll) * zoom - width * HELD_AT).abs() <= 0.5 + 1e-6, "zoom {zoom} playhead at {} px", (at - scroll) * zoom);
                }
                was = at;
            }
        }
    }

    #[test]
    fn smooth_playback_through_a_long_loop_has_one_jump_per_wrap_and_otherwise_glides() {
        let (from, to) = (20.0, 60.0);
        let mut scroll = 15.0;
        let mut was = from;
        let mut jumps = 0;
        let mut wraps = 0;
        for step in 1..(60 * 130) {
            let mut at = from + step as f64 * FRAME;
            while at >= to {
                at -= to - from;
            }
            if at < was {
                wraps += 1;
            }
            let round = round_at(at, Some((from, to)), 120.0, false);
            let before = scroll;
            scroll = Follow::Smooth.scroll(sight(scroll, 100.0, 1000.0), was, at, round);
            if (scroll - before).abs() * 100.0 > 3.0 {
                jumps += 1;
                assert!(at < was, "a jump of {} px away from a wrap at {at}", (scroll - before) * 100.0);
            }
            assert!(scroll + 10.0 >= at && scroll <= at, "the playhead left the view at {at}");
            was = at;
        }
        assert_eq!(wraps, 3);
        assert_eq!(jumps, wraps);
    }

    #[test]
    fn stopping_leaves_the_view_where_it_is() {
        let view = sight(42.0, 100.0, 1000.0);
        assert_eq!(Follow::Smooth.scroll(view, 47.0, 47.0, None), 42.0);
        assert_eq!(Follow::Page.scroll(view, 47.0, 47.0, None), 42.0);
    }
}
