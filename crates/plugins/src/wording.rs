pub const MOST_KNOBS: u32 = 4096;
const MOST_STEPS: u32 = 1000;
const HALVINGS: usize = 40;

pub fn number_in(text: &str) -> Option<f64> {
    let text = text.trim();
    let end = text
        .char_indices()
        .find(|(at, c)| !(c.is_ascii_digit() || *c == '.' || (*at == 0 && matches!(c, '-' | '+'))))
        .map_or(text.len(), |(at, _)| at);
    let number: f64 = text[..end].parse().ok()?;
    let thousands = text[end..].trim_start().starts_with(['k', 'K']);
    Some(if thousands { number * 1000.0 } else { number })
}

pub fn find_value(steps: u32, wanted: &str, mut say: impl FnMut(f64) -> Option<String>) -> Option<f64> {
    let wanted = wanted.trim();
    if steps > 0 && steps <= MOST_STEPS {
        let named = (0..=steps).map(|step| step as f64 / steps as f64).find(|value| say(*value).is_some_and(|text| text.trim().eq_ignore_ascii_case(wanted)));
        if named.is_some() {
            return named;
        }
    }
    let target = number_in(wanted)?;
    let mut read = |value: f64| say(value).as_deref().and_then(number_in);
    let (mut low, mut high) = (0.0, 1.0);
    let (bottom, top) = (read(low), read(high));
    let rising = match (bottom, top) {
        (Some(bottom), Some(top)) if bottom == top => return Some(0.0),
        (Some(bottom), Some(top)) => top > bottom,
        (None, Some(_)) => true,
        (Some(_), None) => false,
        (None, None) => return None,
    };
    for _ in 0..HALVINGS {
        let middle = (low + high) / 2.0;
        let below = match read(middle) {
            Some(found) => (found < target) == rising,
            None => rising,
        };
        if below {
            low = middle;
        } else {
            high = middle;
        }
    }
    let mut miss = |value: f64| read(value).map_or(f64::INFINITY, |found| (found - target).abs());
    let value = if miss(low) < miss(high) { low } else { high };
    Some(if steps > 0 { (value * steps as f64).round() / steps as f64 } else { value })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hertz(value: f64) -> Option<String> {
        let hz = 10.0 * 3000f64.powf(value);
        Some(if hz >= 1000.0 { format!("{:.2} kHz", hz / 1000.0) } else { format!("{hz:.1} Hz") })
    }

    #[test]
    fn a_frequency_is_found_in_either_unit() {
        for wanted in ["3000", "3 kHz", "3k", "3000 Hz"] {
            let value = find_value(0, wanted, hertz).unwrap();
            assert_eq!(hertz(value).unwrap(), "3.00 kHz", "{wanted}");
        }
    }

    #[test]
    fn a_falling_knob_and_a_floor_of_silence_still_work() {
        let gain = |value: f64| if value >= 0.99 { Some("-inf dB".to_string()) } else { Some(format!("{:.1} dB", -60.0 * value)) };
        let value = find_value(0, "-6 dB", gain).unwrap();
        assert_eq!(gain(value).unwrap(), "-6.0 dB");
    }

    #[test]
    fn a_choice_is_found_by_its_name() {
        let shapes = ["Bell", "Low Shelf", "Low Cut", "High Shelf"];
        let say = |value: f64| Some(shapes[(value * 3.0).round() as usize].to_string());
        assert_eq!(find_value(3, "low cut", say), Some(2.0 / 3.0));
        assert_eq!(find_value(3, "Tilt", say), None);
    }

    #[test]
    fn numbers_read_with_their_thousands() {
        assert_eq!(number_in("-6.5 dB"), Some(-6.5));
        assert_eq!(number_in("+3"), Some(3.0));
        assert_eq!(number_in("1.5 kHz"), Some(1500.0));
        assert_eq!(number_in("Bell"), None);
    }
}
