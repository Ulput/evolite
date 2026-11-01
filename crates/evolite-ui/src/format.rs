//! Amounts: integers in duffs, formatted without going through floating point.
//!
//! What the screens show follows Settings → Appearance and "Hide amounts"
//!: the app puts the choice here once a frame ([`set_display`]) and every amount on
//! screen goes through [`amount`], [`signed_amount`] or [`number`], and amount fields are read
//! with [`parse_amount`]. Payment links and exports keep DASH on their own.

use std::cell::Cell;

pub const DUFFS_PER_DASH: i64 = 100_000_000;

/// The unit amounts are shown and typed in (Settings → "Amount unit").
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Unit {
    #[default]
    Dash,
    /// 0,001 DASH = 100 000 duffs.
    MilliDash,
    Duffs,
}

impl Unit {
    /// By the position of the segmented control.
    pub fn from_index(i: usize) -> Self {
        match i {
            1 => Unit::MilliDash,
            2 => Unit::Duffs,
            _ => Unit::Dash,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Unit::Dash => "DASH",
            Unit::MilliDash => "mDASH",
            Unit::Duffs => "duffs",
        }
    }

    /// Decimals one unit has: 8, 5, none.
    fn decimals(self) -> u32 {
        match self {
            Unit::Dash => 8,
            Unit::MilliDash => 5,
            Unit::Duffs => 0,
        }
    }

    /// Decimals always shown, as in Figma for DASH: "12,8460".
    fn min_decimals(self) -> usize {
        match self {
            Unit::Dash => 4,
            Unit::MilliDash => 2,
            Unit::Duffs => 0,
        }
    }
}

/// How numbers are written (Settings → "Number format").
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum NumberStyle {
    /// As the interface language writes them: "1 234,5678" in Russian, "1,234.5678" in English.
    #[default]
    Language,
    /// "1 234,5678": a space between thousands, a comma before decimals.
    SpaceComma,
    /// "1,234.5678".
    CommaDot,
}

impl NumberStyle {
    pub fn from_index(i: usize) -> Self {
        match i {
            1 => NumberStyle::SpaceComma,
            2 => NumberStyle::CommaDot,
            _ => NumberStyle::Language,
        }
    }

    /// The thousands mark and the decimal mark.
    fn marks(self) -> (char, char) {
        match self {
            NumberStyle::Language => crate::i18n::separators(),
            NumberStyle::SpaceComma => ('\u{a0}', ','),
            NumberStyle::CommaDot => (',', '.'),
        }
    }
}

/// The price of one DASH in a currency, as the core last fetched it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Rate {
    /// Hundred-millionths of the currency per DASH: no floating point here either.
    pub price: i64,
    /// "USD", "EUR", "RUB".
    pub currency: [u8; 3],
}

/// Everything about amounts the person chose in Settings.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Display {
    pub unit: Unit,
    pub style: NumberStyle,
    /// "Hide amounts": balances and the history show "••••" until the pointer is over them.
    pub hide: bool,
    /// "Fiat rate", when it is on and the core has a price.
    pub rate: Option<Rate>,
}

thread_local! {
    static DISPLAY: Cell<Display> = Cell::new(Display::default());
}

/// From the app, every frame: what Settings say now.
pub fn set_display(display: Display) {
    DISPLAY.with(|d| d.set(display));
}

pub fn display() -> Display {
    DISPLAY.with(Cell::get)
}

/// The unit's name for labels next to amounts: "DASH", "mDASH", "duffs".
pub fn unit_name() -> &'static str {
    display().unit.name()
}

/// Digits of `n` with `mark` between thousands.
fn grouped(n: u64, mark: Option<char>) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if let Some(mark) = mark
            && i > 0
            && (digits.len() - i).is_multiple_of(3)
        {
            out.push(mark);
        }
        out.push(c);
    }
    out
}

/// `duffs` in `unit`, written in `style`; `group`: thousands marked (not in a field).
fn write(duffs: i64, unit: Unit, style: NumberStyle, signed: bool, group: bool) -> String {
    let (thousands, decimal) = style.marks();
    let abs = duffs.unsigned_abs();
    let per_unit = 10u64.pow(unit.decimals());
    let whole = grouped(abs / per_unit, group.then_some(thousands));
    let mut fraction = format!(
        "{:0width$}",
        abs % per_unit,
        width = unit.decimals() as usize
    );
    while fraction.len() > unit.min_decimals() && fraction.ends_with('0') {
        fraction.pop();
    }
    let sign = match (signed, duffs.signum()) {
        (_, -1) => "−",
        (true, 1) => "+",
        _ => "",
    };
    if fraction.is_empty() {
        format!("{sign}{whole}")
    } else {
        format!("{sign}{whole}{decimal}{fraction}")
    }
}

/// The number alone in the chosen unit and format: "12,8460", "12 846,00", "1 284 600 000".
pub fn number(duffs: i64, signed: bool) -> String {
    let d = display();
    write(duffs, d.unit, d.style, signed, true)
}

/// An amount with its unit, as screens show it: "12,8460 DASH", "12 846,00 mDASH".
pub fn amount(duffs: i64) -> String {
    format!("{} {}", number(duffs, false), unit_name())
}

/// "+0,2500 DASH", "−1,0000 DASH".
pub fn signed_amount(duffs: i64) -> String {
    format!("{} {}", number(duffs, true), unit_name())
}

/// What "Max" puts in an amount field: the chosen unit, no thousands marks.
pub fn input_text(duffs: i64) -> String {
    let d = display();
    write(duffs, d.unit, d.style, false, false)
}

/// "••••" in place of an amount, keeping its unit: "•••• DASH".
pub fn mask(shown: &str) -> String {
    match shown.rsplit_once(' ') {
        Some((_, unit)) if unit.chars().all(|c| c.is_ascii_alphabetic()) => {
            format!("•••• {unit}")
        }
        _ => "••••".into(),
    }
}

/// "≈ 615,40 USD": `duffs` at the rate the core fetched, when "Fiat rate" is on.
pub fn fiat(duffs: i64) -> Option<String> {
    let d = display();
    let rate = d.rate?;
    // Hundred-millionths of the currency, then cents rounded half up.
    let micro = (duffs as i128 * rate.price as i128) / DUFFS_PER_DASH as i128;
    let cents = (micro.unsigned_abs() + 500_000) / 1_000_000;
    let (thousands, decimal) = d.style.marks();
    let whole = grouped((cents / 100) as u64, Some(thousands));
    let sign = if micro < 0 { "−" } else { "" };
    let currency = std::str::from_utf8(&rate.currency).unwrap_or("");
    Some(format!(
        "≈ {sign}{whole}{decimal}{:02} {currency}",
        cents % 100
    ))
}

/// A price (hundred-millionths) to the cent in the chosen format: "47,96".
pub fn rate_number(price: i64) -> String {
    let cents = (price.unsigned_abs() + 500_000) / 1_000_000;
    let (thousands, decimal) = display().style.marks();
    format!(
        "{}{decimal}{:02}",
        grouped(cents / 100, Some(thousands)),
        cents % 100
    )
}

/// What stays of `balance` after paying `amount` and `fee`: below zero when it does not cover
/// them. Saturating: a typed amount can be as large as an `i64` holds, and an overflow would
/// wrap a "not enough" into a large balance left.
pub fn left_after(balance: i64, amount: i64, fee: i64) -> i64 {
    balance.saturating_sub(amount).saturating_sub(fee)
}

/// A decimal number of `decimals` places from text: digits, one decimal mark, spaces allowed.
fn parse_units(input: &str, decimals: u32, decimal_marks: &[char]) -> Option<i64> {
    let clean: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    let (whole, fraction) = match clean.split_once(decimal_marks) {
        Some((w, f)) => (w, f),
        None => (clean.as_str(), ""),
    };
    let digits = |s: &str| s.chars().all(|c| c.is_ascii_digit());
    if (whole.is_empty() && fraction.is_empty())
        || !digits(whole)
        || !digits(fraction)
        || fraction.len() > decimals as usize
    {
        return None;
    }
    let whole: i64 = if whole.is_empty() {
        0
    } else {
        whole.parse().ok()?
    };
    let fraction: i64 = if decimals == 0 {
        0
    } else {
        format!("{fraction:0<width$}", width = decimals as usize)
            .parse()
            .ok()?
    };
    whole
        .checked_mul(10i64.pow(decimals))?
        .checked_add(fraction)
}

/// What the user typed in an amount field, in the chosen unit and format, in duffs. With
/// "1,234.56" (or "1.234,56") the thousands mark only groups three digits: "0,25" there is refused,
/// not read as 25 — a misread amount is money sent.
pub fn parse_amount(input: &str) -> Option<i64> {
    let d = display();
    let decimals = d.unit.decimals();
    let (group, decimal) = d.style.marks();
    if group.is_whitespace() {
        // Spaces between thousands: a comma or a dot before decimals, as typed.
        return if decimals == 0 {
            parse_units(input, 0, &[])
        } else {
            parse_units(input, decimals, &[',', '.'])
        };
    }
    // "1,234.56" or "1.234,56": the thousands mark only in groups of three.
    let clean: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    let (whole, fraction) = match clean.split_once(decimal) {
        Some((w, f)) => (w, Some(f)),
        None => (clean.as_str(), None),
    };
    let mut groups = whole.split(group);
    let first = groups.next().unwrap_or("");
    let rest: Vec<&str> = groups.collect();
    if !rest.is_empty() && (first.is_empty() || first.len() > 3) {
        return None;
    }
    if rest.iter().any(|g| g.len() != 3) {
        return None;
    }
    let whole: String = whole.chars().filter(|c| *c != group).collect();
    match fraction {
        Some(f) if f.contains(group) => None,
        Some(f) => parse_units(&format!("{whole}{decimal}{f}"), decimals, &[decimal]),
        None => parse_units(&whole, decimals, &[decimal]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// In DASH with the Russian marks, whatever the settings.
    fn format_dash(duffs: i64, signed: bool) -> String {
        write(duffs, Unit::Dash, NumberStyle::SpaceComma, signed, true)
    }

    fn parse_dash(input: &str) -> Option<i64> {
        parse_units(input, Unit::Dash.decimals(), &[',', '.'])
    }

    fn with(unit: Unit, style: NumberStyle) {
        set_display(Display {
            unit,
            style,
            ..Display::default()
        });
    }

    #[test]
    fn keeps_four_to_eight_decimals() {
        assert_eq!(format_dash(1_284_600_000, false), "12,8460");
        assert_eq!(format_dash(226, false), "0,00000226");
        assert_eq!(
            format_dash(123_456_700_000_001, false),
            "1\u{a0}234\u{a0}567,00000001"
        );
    }

    #[test]
    fn signs_when_asked() {
        assert_eq!(format_dash(25_000_000, true), "+0,2500");
        assert_eq!(format_dash(-100_000_000, true), "−1,0000");
        assert_eq!(format_dash(0, true), "0,0000");
    }

    #[test]
    fn parses_comma_dot_and_spaces() {
        assert_eq!(parse_dash("0,25"), Some(25_000_000));
        assert_eq!(parse_dash("0.25"), Some(25_000_000));
        assert_eq!(parse_dash("1 234,00000001"), Some(123_400_000_001));
        assert_eq!(parse_dash(",5"), Some(50_000_000));
        assert_eq!(parse_dash("3"), Some(300_000_000));
    }

    #[test]
    fn rejects_what_is_not_an_amount() {
        for bad in ["", ",", "-1", "1,2,3", "0,123456789", "abc", "1e5"] {
            assert_eq!(parse_dash(bad), None, "{bad}");
        }
    }

    #[test]
    fn amounts_follow_the_chosen_unit() {
        with(Unit::Dash, NumberStyle::SpaceComma);
        assert_eq!(amount(1_284_600_000), "12,8460 DASH");
        with(Unit::MilliDash, NumberStyle::SpaceComma);
        assert_eq!(amount(1_284_600_000), "12\u{a0}846,00 mDASH");
        assert_eq!(amount(226), "0,00226 mDASH");
        with(Unit::Duffs, NumberStyle::SpaceComma);
        assert_eq!(amount(1_284_600_000), "1\u{a0}284\u{a0}600\u{a0}000 duffs");
        assert_eq!(signed_amount(-226), "−226 duffs");
        with(Unit::Dash, NumberStyle::Language);
    }

    #[test]
    fn numbers_follow_the_chosen_format() {
        with(Unit::Dash, NumberStyle::CommaDot);
        assert_eq!(amount(123_456_700_000_001), "1,234,567.00000001 DASH");
        assert_eq!(input_text(123_456_700_000_001), "1234567.00000001");
        with(Unit::Dash, NumberStyle::SpaceComma);
        assert_eq!(number(-25_000_000, true), "−0,2500");
        assert_eq!(input_text(123_400_000_000), "1234,0000");
        with(Unit::Dash, NumberStyle::Language);
    }

    #[test]
    fn typed_amounts_are_read_in_the_chosen_unit() {
        with(Unit::MilliDash, NumberStyle::SpaceComma);
        assert_eq!(parse_amount("1,5"), Some(150_000));
        assert_eq!(parse_amount("0,00001"), Some(1));
        assert_eq!(parse_amount("0,000001"), None, "finer than a duff");
        with(Unit::Duffs, NumberStyle::SpaceComma);
        assert_eq!(parse_amount("226"), Some(226));
        assert_eq!(parse_amount("1 000"), Some(1000));
        assert_eq!(parse_amount("2,5"), None, "no part of a duff");
        with(Unit::Dash, NumberStyle::SpaceComma);
        assert_eq!(parse_amount("0,25"), Some(25_000_000));
        with(Unit::Dash, NumberStyle::Language);
    }

    #[test]
    fn a_comma_groups_thousands_only_where_the_format_says_so() {
        with(Unit::Dash, NumberStyle::CommaDot);
        assert_eq!(parse_amount("1,234.5"), Some(123_450_000_000));
        assert_eq!(parse_amount("0.25"), Some(25_000_000));
        assert_eq!(parse_amount("1234"), Some(123_400_000_000));
        // Not a thousands group: refused rather than read as 25 DASH.
        for bad in ["0,25", "1,23", "1234,567", ",123", "1.2,3", "1.234,5"] {
            assert_eq!(parse_amount(bad), None, "{bad}");
        }
        // What "Max" writes reads back.
        assert_eq!(parse_amount(&input_text(1_234_567)), Some(1_234_567));
        with(Unit::Dash, NumberStyle::SpaceComma);
    }

    #[test]
    fn the_largest_amount_typed_is_still_more_than_there_is() {
        with(Unit::Dash, NumberStyle::SpaceComma);
        let huge = parse_amount("92233720368,54775807").unwrap();
        assert_eq!(huge, i64::MAX);
        // From an empty balance: "more than there is", not a wrapped-around fortune.
        assert!(left_after(0, huge, 226) < 0);
        assert_eq!(left_after(1_000, 600, 226), 174);
    }

    #[test]
    fn hidden_amounts_keep_their_unit() {
        assert_eq!(mask("12,8460 DASH"), "•••• DASH");
        assert_eq!(mask("−226 duffs"), "•••• duffs");
        assert_eq!(mask("12,8460"), "••••");
    }

    #[test]
    fn the_rate_gives_cents_without_floats() {
        set_display(Display {
            rate: Some(Rate {
                price: 4_796_150_000,
                currency: *b"USD",
            }),
            ..Display::default()
        });
        // 12.846 DASH × 47.9615 = 616.113…
        assert_eq!(fiat(1_284_600_000).as_deref(), Some("≈ 616.11 USD"));
        assert_eq!(fiat(0).as_deref(), Some("≈ 0.00 USD"));
        set_display(Display::default());
        assert_eq!(fiat(1_284_600_000), None, "off: nothing");
    }
}
