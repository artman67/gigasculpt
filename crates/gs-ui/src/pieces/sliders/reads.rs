//! How a slider reads its value in words and digits ([`Reads`], [`number`]).

/// How a slider reads its value: as a percentage when `percent` (0.5 reads 50%), in whole
/// numbers when `whole`, else as [`number`] does; and as `word`'s word where it reads as
/// `word`'s value does (a form's unset number "auto" at its low end, the floor's Size "endless"
/// at 0; decision 0062). Every slider reads its value this way, the form's too.
#[derive(Clone, Copy, Default)]
pub(crate) struct Reads<'a> {
    pub(super) whole: bool,
    pub(super) word: Option<(f32, &'a str)>,
    pub(super) percent: bool,
}

impl Reads<'_> {
    /// `v` in digits.
    pub(super) fn digits(self, v: f32, ends: [f32; 2]) -> String {
        if self.percent {
            format!("{:.0}%", v * 100.0)
        } else if self.whole {
            format!("{v:.0}")
        } else {
            number(v, ends)
        }
    }

    /// `v` as the slider reads it.
    pub(crate) fn read(self, v: f32, ends: [f32; 2]) -> String {
        let digits = self.digits(v, ends);
        match self.word {
            Some((at, word)) if digits == self.digits(at, ends) => word.to_string(),
            _ => digits,
        }
    }
}

/// A count: whole numbers, with 1 reading "off" (a curve's Steps and Tile and the brush's Wrap
/// Mode are off at 1).
impl From<bool> for Reads<'_> {
    fn from(count: bool) -> Self {
        let word = count.then_some((1.0, "off"));
        (count, word).into()
    }
}

/// Whole numbers or not, and a word at a value.
impl<'a> From<(bool, Option<(f32, &'a str)>)> for Reads<'a> {
    fn from((whole, word): (bool, Option<(f32, &'a str)>)) -> Self {
        let percent = false;
        Reads {
            whole,
            word,
            percent,
        }
    }
}

/// `value` as a control's slider shows it: ranges spanning 100 or more in whole numbers; other
/// signed ranges (exposure) in tenths; the rest in hundredths, one place more for each tenth the
/// range's top falls below 1, so 0.005 in a range up to 0.02 reads 0.0050.
pub(crate) fn number(value: f32, [low, high]: [f32; 2]) -> String {
    if high - low >= 100.0 && low < 0.0 {
        format!("{value:+.0}")
    } else if high - low >= 100.0 {
        format!("{value:.0}")
    } else if low < 0.0 {
        format!("{value:+.1}")
    } else {
        let (mut places, mut top) = (2, high);
        while top < 1.0 && places < 6 {
            top *= 10.0;
            places += 1;
        }
        format!("{value:.places$}")
    }
}
