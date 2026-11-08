//! Amounts: integers in duffs, formatted without going through floating point.
//!
//! What the screens show follows Settings → Appearance and "Hide amounts"
//!: the app puts the choice here once a frame ([`set_display`]) and every amount on
//! screen goes through [`amount`], [`signed_amount`] or [`number`], and amount fields are read
//! with [`parse_amount`]. Payment links and exports keep DASH on their own.

use std::cell::Cell;

pub const DUFFS_PER_DASH: i64 = 100_000_000;

/// The unit amounts are shown and typed in (Settings → "Unit").
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
    /// "Hide amounts": balances and the history show "••••" (the eye beside an amount switches it).
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
    let all = unit.decimals() as usize;
    write_digits(
        duffs,
        unit,
        style,
        (signed, group),
        (unit.min_decimals(), all),
    )
}

/// As [`write`], with at most `max` decimals (the rest cut off, not rounded: [`rounded_number`]
/// rounds first) and trailing zeros dropped down to `min`.
fn write_digits(
    duffs: i64,
    unit: Unit,
    style: NumberStyle,
    (signed, group): (bool, bool),
    (min, max): (usize, usize),
) -> String {
    let (thousands, decimal) = style.marks();
    let abs = duffs.unsigned_abs();
    let per_unit = 10u64.pow(unit.decimals());
    let whole = grouped(abs / per_unit, group.then_some(thousands));
    let mut fraction = format!(
        "{:0width$}",
        abs % per_unit,
        width = unit.decimals() as usize
    );
    fraction.truncate(max);
    while fraction.len() > min && fraction.ends_with('0') {
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

/// [`number`] with no more than `decimals` after the decimal mark, rounded half up (R4a: a tile
/// too narrow for the whole amount), and whether that is still the exact amount. Trailing zeros
/// go as in [`number`], but never below `decimals`: 1234.5 DASH with 2 is "1 234,50", exact.
/// With as many decimals as the unit has, or more, it is [`number`] itself.
pub fn rounded_number(duffs: i64, decimals: usize) -> (String, bool) {
    let d = display();
    let all = d.unit.decimals() as usize;
    if decimals >= all {
        return (number(duffs, false), true);
    }
    let step = 10u64.pow((all - decimals) as u32);
    let abs = duffs.unsigned_abs();
    let rounded = (abs / step + u64::from(abs % step >= step / 2)) * step;
    let signed = i64::try_from(rounded).map_or(duffs, |r| r * duffs.signum());
    let min = decimals.min(d.unit.min_decimals());
    let text = write_digits(signed, d.unit, d.style, (false, true), (min, decimals));
    (text, rounded == abs)
}

/// An amount with its unit, as screens show it: "12,8460 DASH", "12 846,00 mDASH".
pub fn amount(duffs: i64) -> String {
    format!("{} {}", number(duffs, false), unit_name())
}

/// A balance as [`amount`], or "•••• DASH" with "Hide amounts" (FEAT-0007): what a person has,
/// not what they typed or pay as a fee.
pub fn balance(duffs: i64) -> String {
    let shown = amount(duffs);
    if display().hide { mask(&shown) } else { shown }
}

/// "+0,2500 DASH", "−1,0000 DASH".
pub fn signed_amount(duffs: i64) -> String {
    format!("{} {}", number(duffs, true), unit_name())
}

/// Typed with more digits after the decimal mark than the chosen unit has ("0.000000018" in
/// DASH, "1.5" in duffs), and an amount without them: the line under the field says what is
/// wrong instead of "enter an amount".
pub fn too_precise(input: &str) -> bool {
    if parse_amount(input).is_some() {
        return false;
    }
    let d = display();
    let decimals = d.unit.decimals() as usize;
    let (group, decimal) = d.style.marks();
    let marks: &[char] = if group.is_whitespace() {
        &[',', '.']
    } else {
        &[decimal]
    };
    let text = input.trim();
    let Some(at) = text.rfind(marks) else {
        return false;
    };
    // The marks are ASCII: one byte each.
    let fraction = &text[at + 1..];
    if fraction.len() <= decimals || !fraction.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let kept = if decimals == 0 {
        &text[..at]
    } else {
        &text[..at + 1 + decimals]
    };
    parse_amount(kept).is_some()
}

/// What "Max" puts in an amount field: the chosen unit, no thousands marks.
pub fn input_text(duffs: i64) -> String {
    let d = display();
    write(duffs, d.unit, d.style, false, false)
}

/// A pill's label: the amount in the chosen unit and format, without trailing zeros — "0.1",
/// "0,5", "1", "100" (mDASH).
pub fn short_number(duffs: i64) -> String {
    let d = display();
    let (_, decimal) = d.style.marks();
    let text = write(duffs, d.unit, d.style, false, true);
    if text.contains(decimal) {
        text.trim_end_matches('0')
            .trim_end_matches(decimal)
            .to_string()
    } else {
        text
    }
}

/// A suggested amount (an example or a default) kept in the chosen unit and format, called each
/// frame the field is drawn. Forms are made at start-up, before the settings and the language
/// are known, and live on while they change: text written in another format is refused by
/// `parse_amount` ("0,1000" with "1,234.56"). While the field holds the suggestion, written in
/// any unit and format, it is rewritten in the current one; once the person changes it (or
/// clears it), the field is theirs and the suggestion is dropped.
///
/// A form starts with `text` already holding [`input_text`] of the suggestion, so that an empty
/// field means the person cleared it.
pub fn fill_suggested(text: &mut String, suggested: &mut Option<i64>) {
    let Some(duffs) = *suggested else {
        return;
    };
    let now = input_text(duffs);
    if *text == now {
        return;
    }
    let written_somehow = [Unit::Dash, Unit::MilliDash, Unit::Duffs]
        .into_iter()
        .flat_map(|unit| {
            [NumberStyle::SpaceComma, NumberStyle::CommaDot]
                .map(|style| write(duffs, unit, style, false, false))
        })
        .any(|t| t == *text);
    if written_somehow {
        *text = now;
    } else {
        *suggested = None;
    }
}

/// A sum of a payment as shown, or [`mask`]ed while "Hide amounts" is on (BUG-73: the payments
/// of a contact).
pub fn signed_balance(duffs: i64) -> String {
    let shown = signed_amount(duffs);
    if display().hide { mask(&shown) } else { shown }
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

/// A count of things («Shown 1 204 of 2 310»): whole, its thousands grouped as the number
/// format groups them.
pub fn count(n: u64) -> String {
    let (group, _) = display().style.marks();
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(group);
        }
        out.push(c);
    }
    out
}

/// An amount typed into History's search, in the chosen unit: a comma or a dot before the
/// decimals whatever the number format (FEAT-0009 R-4), spaces allowed; none for anything else.
pub fn parse_search_amount(input: &str) -> Option<i64> {
    parse_units(input, display().unit.decimals(), &[',', '.'])
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

    /// R4a: a tile's amount with fewer decimals is rounded half up, carries into the whole
    /// part, keeps the decimals asked for and says whether it is still exact.
    #[test]
    fn fewer_decimals_round_half_up_and_say_whether_exact() {
        with(Unit::Dash, NumberStyle::SpaceComma);
        assert_eq!(
            rounded_number(123_456_789_012, 5),
            ("1\u{a0}234,56789".into(), false)
        );
        assert_eq!(
            rounded_number(123_456_789_012, 2),
            ("1\u{a0}234,57".into(), false)
        );
        assert_eq!(
            rounded_number(123_454_999_999, 2),
            ("1\u{a0}234,55".into(), false)
        );
        assert_eq!(
            rounded_number(123_449_999_999, 2),
            ("1\u{a0}234,50".into(), false)
        );
        assert_eq!(
            rounded_number(123_444_499_999, 2),
            ("1\u{a0}234,44".into(), false)
        );
        assert_eq!(
            rounded_number(99_999_999_999, 2),
            ("1\u{a0}000,00".into(), false)
        );
        assert_eq!(
            rounded_number(123_450_000_000, 2),
            ("1\u{a0}234,50".into(), true)
        );
        assert_eq!(
            rounded_number(123_450_000_000, 6),
            ("1\u{a0}234,5000".into(), true)
        );
        assert_eq!(
            rounded_number(123_456_789_012, 8),
            (number(123_456_789_012, false), true)
        );
        assert_eq!(
            rounded_number(-123_456_789_012, 2),
            ("−1\u{a0}234,57".into(), false)
        );
        with(Unit::MilliDash, NumberStyle::CommaDot);
        assert_eq!(
            rounded_number(123_456_789_012, 2),
            ("1,234,567.89".into(), false)
        );
        with(Unit::Duffs, NumberStyle::CommaDot);
        assert_eq!(
            rounded_number(123_456_789_012, 2),
            ("123,456,789,012".into(), true)
        );
        with(Unit::Dash, NumberStyle::SpaceComma);
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
    fn suggested_amounts_are_written_as_the_settings_read_them() {
        for unit in [Unit::Dash, Unit::MilliDash, Unit::Duffs] {
            for style in [NumberStyle::SpaceComma, NumberStyle::CommaDot] {
                with(unit, style);
                for duffs in [10_000_000, 25_000_000, 30_000_000, 50_000_000, 100_000_000] {
                    // As written at start-up, before the settings were known: "0,1000".
                    let mut text = write(duffs, Unit::Dash, NumberStyle::SpaceComma, false, false);
                    let mut suggested = Some(duffs);
                    fill_suggested(&mut text, &mut suggested);
                    assert_eq!(
                        parse_amount(&text),
                        Some(duffs),
                        "{unit:?} {style:?} {text}"
                    );
                    assert_eq!(suggested, Some(duffs), "still the suggestion");
                    let label = short_number(duffs);
                    assert_eq!(
                        parse_amount(&label),
                        Some(duffs),
                        "{unit:?} {style:?} {label}"
                    );
                }
            }
        }
        // The settings change while the form lives: an untouched suggestion follows them.
        with(Unit::Dash, NumberStyle::SpaceComma);
        let (mut text, mut suggested) = (input_text(25_000_000), Some(25_000_000));
        fill_suggested(&mut text, &mut suggested);
        assert_eq!(text, "0,2500");
        with(Unit::Dash, NumberStyle::CommaDot);
        fill_suggested(&mut text, &mut suggested);
        assert_eq!(text, "0.2500");
        with(Unit::MilliDash, NumberStyle::CommaDot);
        fill_suggested(&mut text, &mut suggested);
        assert_eq!(parse_amount(&text), Some(25_000_000), "{text}");
        // What the person typed, or an empty field, is theirs: kept, and the suggestion dropped.
        for typed in ["0.3", ""] {
            let (mut text, mut suggested) = (typed.to_string(), Some(25_000_000));
            fill_suggested(&mut text, &mut suggested);
            assert_eq!((text.as_str(), suggested), (typed, None));
            with(Unit::Dash, NumberStyle::SpaceComma);
            fill_suggested(&mut text, &mut suggested);
            assert_eq!(text, typed);
            with(Unit::MilliDash, NumberStyle::CommaDot);
        }
        with(Unit::Dash, NumberStyle::CommaDot);
        assert_eq!(input_text(25_000_000), "0.2500");
        assert_eq!(short_number(10_000_000), "0.1");
        assert_eq!(short_number(100_000_000), "1");
        with(Unit::Dash, NumberStyle::SpaceComma);
        assert_eq!(short_number(50_000_000), "0,5");
        with(Unit::MilliDash, NumberStyle::CommaDot);
        assert_eq!(short_number(10_000_000), "100");
        set_display(Display::default());
    }

    #[test]
    fn too_many_decimals_are_told_apart_from_a_wrong_amount() {
        with(Unit::Dash, NumberStyle::CommaDot);
        assert!(too_precise("0.000000018"));
        assert!(too_precise("1,234.123456789"));
        assert!(!too_precise("0.00000001"), "a duff is fine");
        assert!(!too_precise("abc"));
        assert!(!too_precise("0,25"), "a wrong mark, not too precise");
        assert!(!too_precise("0.12a456789"));
        with(Unit::Dash, NumberStyle::SpaceComma);
        assert!(too_precise("0,000000018"));
        assert!(too_precise("0.000000018"));
        with(Unit::MilliDash, NumberStyle::CommaDot);
        assert!(too_precise("0.000001"));
        assert!(!too_precise("0.00001"));
        with(Unit::Duffs, NumberStyle::CommaDot);
        assert!(too_precise("1.5"));
        assert!(!too_precise("15"));
        set_display(Display::default());
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
