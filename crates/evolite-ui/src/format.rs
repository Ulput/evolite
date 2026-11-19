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
    /// When the core got it: older than 5 minutes, no typing in the currency (FEAT-0008).
    pub at: evolite_api::Timestamp,
}

impl Rate {
    pub fn code(&self) -> &str {
        std::str::from_utf8(&self.currency).unwrap_or("")
    }
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

/// The thousands mark and the decimal mark of the chosen number format (FEAT-0019: token amounts
/// are written and typed with them too).
pub fn number_marks() -> (char, char) {
    display().style.marks()
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
    // One number ([`typed_parts`]) with more decimals than the unit has.
    typed_parts(input)
        .is_some_and(|(_, fraction)| fraction.len() > display().unit.decimals() as usize)
}

/// What "Max" puts in an amount field: the chosen unit, no thousands marks.
pub fn input_text(duffs: i64) -> String {
    let d = display();
    write(duffs, d.unit, d.style, false, false)
}

/// A pill's label: the amount in the chosen unit and format, without trailing zeros — "0.1",
/// "0,5", "1", "100" (mDASH). Thousands are set apart by a no-break space, never by a comma or a
/// point (BUG-161): «1 000» mDASH typed back reads as 1000, where «1,000» would read as 1. (The
/// designer asked for U+202F; our Inter and JetBrains Mono have no glyph for it, U+00A0 they
/// have, and the Russian format groups with it.)
pub fn short_number(duffs: i64) -> String {
    let d = display();
    let (_, decimal) = d.style.marks();
    let text = write(duffs, d.unit, d.style, false, false);
    let text = if text.contains(decimal) {
        text.trim_end_matches('0')
            .trim_end_matches(decimal)
            .to_string()
    } else {
        text
    };
    let (whole, fraction) = match text.split_once(decimal) {
        Some((w, f)) => (w, Some(f)),
        None => (text.as_str(), None),
    };
    let mut out = String::new();
    for (i, c) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i) % 3 == 0 {
            out.push('\u{a0}');
        }
        out.push(c);
    }
    if let Some(fraction) = fraction {
        out.push(decimal);
        out.push_str(fraction);
    }
    out
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

/// «1 234,56 USD»: hundredths of the currency in the chosen format. The code is never
/// translated (AC-T11).
pub fn fiat_text(cents: i128, currency: &str) -> String {
    let (thousands, decimal) = display().style.marks();
    let abs = cents.unsigned_abs();
    let sign = if cents < 0 { "−" } else { "" };
    let whole = grouped((abs / 100).min(u64::MAX as u128) as u64, Some(thousands));
    format!("{sign}{whole}{decimal}{:02} {currency}", abs % 100)
}

/// `duffs` in the currency at `price` (FEAT-0008, R-6): «≈ 25,00 USD», «< 0,01 USD» below half
/// a cent (AC-T2), «≈ •••• USD» with «Hide amounts» (AC-T9).
pub fn about(duffs: i64, price: i64, currency: &str) -> String {
    if display().hide {
        return format!("≈ •••• {currency}");
    }
    if evolite_api::fiat::below_a_cent(duffs, price) {
        return format!("< {}", fiat_text(if duffs < 0 { -1 } else { 1 }, currency));
    }
    match evolite_api::fiat::cents_of(duffs, price) {
        Some(cents) => format!("≈ {}", fiat_text(cents, currency)),
        None => "—".into(),
    }
}

/// An amount in the currency as its field shows it: «25,00», no thousands marks.
pub fn fiat_input_text(cents: i128) -> String {
    let (_, decimal) = display().style.marks();
    let abs = cents.unsigned_abs();
    format!("{}{decimal}{:02}", abs / 100, abs % 100)
}

/// An amount typed in the currency, in hundredths, read as [`parse_amount`] reads one.
pub fn parse_fiat(input: &str) -> Option<i128> {
    parse_typed(input, 2).map(i128::from)
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

/// An amount typed into History's search, in the chosen unit, by the one rule of every amount
/// ([`typed_parts`], BUG-161): a comma or a dot before the decimals whatever the number format
/// (FEAT-0009 R-4), spaces allowed; none for anything else.
pub fn parse_search_amount(input: &str) -> Option<i64> {
    parse_typed(input, display().unit.decimals())
}

/// An amount in DASH whatever the unit chosen, in duffs: a `dash:` link's `amount=` (from a QR
/// code, the clipboard or a click), by the one rule of every amount ([`typed_parts`], BUG-161).
pub fn parse_dash(input: &str) -> Option<i64> {
    // A machine's format, not the person's: no [`unclear`] here — «1.000» in a German
    // interface is 1 DASH in a link, as BIP21 writes it.
    let (whole, fraction) = typed_parts(input)?;
    parse_units(
        &format!("{whole}.{fraction}"),
        Unit::Dash.decimals(),
        &['.'],
    )
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

/// What the user typed in an amount field, in the chosen unit, in duffs: [`typed_parts`] read
/// with the unit's decimals. "0,002" and "0.002" are 0.002 DASH in every language; "1,234.56"
/// and "0,0,2" are refused, never read as another number — a misread amount is money sent
/// (BUG-161: "0,002" in English was read as 2).
pub fn parse_amount(input: &str) -> Option<i64> {
    parse_typed(input, display().unit.decimals())
}

/// BUG-161: a typed number split at its decimal mark, by one rule in every language and number
/// format: digits, spaces between them, and at most one comma or point, which is the decimal
/// mark. A second mark (thousands marks, "1,234.56", "1.234,56", "0,0,2") makes it ambiguous: it
/// is refused, the line under the field says so, and nothing is guessed. The fields are filled
/// without thousands marks ([`input_text`]), so what the app puts there reads back the same.
pub fn typed_parts(input: &str) -> Option<(String, String)> {
    let clean: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    let mut parts = clean.split([',', '.']);
    let whole = parts.next().unwrap_or("").to_owned();
    let fraction = parts.next().unwrap_or("").to_owned();
    let digits = |s: &str| s.chars().all(|c| c.is_ascii_digit());
    if parts.next().is_some()
        || (whole.is_empty() && fraction.is_empty())
        || !digits(&whole)
        || !digits(&fraction)
    {
        return None;
    }
    Some((whole, fraction))
}

/// BUG-161 r1 Should 1, the customer's decision: a number a person in this format reads as
/// thousands — «1,000» or «2,500» where the comma groups thousands («1,234.56», English; the
/// point in German) — is neither 1 nor 2.5 nor guessed: it is refused, and [`amount_hint`] asks
/// which. Exactly: one mark, the format's thousands mark, a whole part of 1–3 digits not starting
/// with 0, and exactly 3 digits after it. «0,002», «1,25», «12345,678» and every format with a
/// space between thousands (where a comma or a point is only decimal) read as before. The parts,
/// when it is so.
pub fn unclear(input: &str) -> Option<(String, String)> {
    let (group, _) = display().style.marks();
    if group.is_whitespace() || input.matches([',', '.']).count() != 1 || !input.contains(group) {
        return None;
    }
    let (whole, fraction) = typed_parts(input)?;
    (!whole.is_empty() && whole.len() <= 3 && !whole.starts_with('0') && fraction.len() == 3)
        .then_some((whole, fraction))
}

/// [`typed_parts`] of a number a person typed in the chosen format: an [`unclear`] one is no
/// number. Every field reads through this; a `dash:` link, written by a machine, does not.
pub fn typed_amount_parts(input: &str) -> Option<(String, String)> {
    if unclear(input).is_some() {
        return None;
    }
    typed_parts(input)
}

/// A number typed with `decimals` places: see [`typed_amount_parts`]. More digits after the
/// mark than `decimals` is no amount ([`too_precise`] says why).
fn parse_typed(input: &str, decimals: u32) -> Option<i64> {
    let (whole, fraction) = typed_amount_parts(input)?;
    parse_units(&format!("{whole}.{fraction}"), decimals, &['.'])
}

/// The line under an amount field for text it cannot read as an amount (BUG-161): which number
/// an [`unclear`] one is meant to be, too many decimals, or not one number — a second mark, a
/// letter. None when it reads, or is empty.
pub fn amount_hint(input: &str) -> Option<String> {
    if input.trim().is_empty() || parse_amount(input).is_some() {
        return None;
    }
    if let Some((whole, fraction)) = unclear(input) {
        let (_, decimal) = display().style.marks();
        let kept = fraction.trim_end_matches('0');
        let small = if kept.is_empty() {
            whole.clone()
        } else {
            format!("{whole}{decimal}{kept}")
        };
        let decimals = if kept.is_empty() { "0" } else { kept };
        return Some(format!(
            tr!("Unclear: is {typed} {small} or {large}? Type {large} or {decimal}."),
            typed = input.trim(),
            small = small,
            large = format!("{whole}{fraction}"),
            decimal = format!("{whole}{decimal}{decimals}"),
        ));
    }
    Some(if too_precise(input) {
        format!(
            tr!("Too many digits after the decimal mark: the smallest amount is {}"),
            amount(1)
        )
    } else {
        tr!("Enter an amount, e.g. 0.25").to_owned()
    })
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
        // Exactly half goes up.
        assert_eq!(
            rounded_number(123_456_500_000, 2),
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

    /// BUG-161: in English «0,002» was read as 2 DASH («0,002» passed as a thousands group). In
    /// every interface language and number format, and in every unit: one comma or point is the
    /// decimal mark; two marks are refused with a line under the field, never read as another
    /// number; what the app writes into a field reads back the same; a fiat amount reads alike.
    #[test]
    fn one_mark_is_decimal_and_two_are_refused_in_every_language_and_format() {
        let styles = [
            NumberStyle::Language,
            NumberStyle::SpaceComma,
            NumberStyle::CommaDot,
        ];
        for lang in crate::i18n::Lang::ALL {
            crate::i18n::set(lang);
            for style in styles {
                with(Unit::Dash, style);
                let at = format!("{lang:?} {style:?}");
                for (typed, duffs) in [
                    ("0,002", 200_000),
                    ("0.002", 200_000),
                    ("1,25", 125_000_000),
                    ("1.25", 125_000_000),
                    ("12345,678", 1_234_567_800_000),
                    ("0,25", 25_000_000),
                    (".5", 50_000_000),
                    ("5.", 500_000_000),
                    ("1234", 123_400_000_000),
                    ("1 234,5", 123_450_000_000),
                    ("0,00000001", 1),
                ] {
                    assert_eq!(parse_amount(typed), Some(duffs), "{at}: {typed}");
                    assert_eq!(amount_hint(typed), None, "{at}: {typed}");
                }
                // r1 Should 1: refused exactly where the mark is this format's thousands mark,
                // a decimal everywhere else.
                let (group, _) = style.marks();
                for (typed, mark, duffs) in [
                    ("1,000", ',', 100_000_000),
                    ("2,500", ',', 250_000_000),
                    ("999,999", ',', 99_999_900_000),
                    ("1.000", '.', 100_000_000),
                ] {
                    if group == mark {
                        assert_eq!(parse_amount(typed), None, "{at}: {typed} is unclear");
                        assert!(unclear(typed).is_some(), "{at}: {typed}");
                        assert!(amount_hint(typed).is_some(), "{at}: a line for {typed}");
                    } else {
                        assert_eq!(parse_amount(typed), Some(duffs), "{at}: {typed}");
                    }
                }
                for bad in [
                    "1,234.56",
                    "1.234,56",
                    "0,0,2",
                    "1.234.567",
                    "1,234,567",
                    ",",
                    ".",
                    "1,2a",
                    "--1",
                    "1e3",
                ] {
                    assert_eq!(parse_amount(bad), None, "{at}: {bad}");
                    assert!(
                        amount_hint(bad).is_some(),
                        "{at}: a line under the field for {bad}"
                    );
                }
                assert_eq!(parse_fiat("0,02"), Some(2), "{at}: cents");
                // History's search and a `dash:` link read by the same rule.
                assert_eq!(parse_search_amount("0,002"), Some(200_000), "{at}: search");
                assert_eq!(parse_search_amount("0.002"), Some(200_000), "{at}: search");
                assert_eq!(parse_search_amount("1,000,000"), None, "{at}: search");
                assert_eq!(parse_dash("0,002"), Some(200_000), "{at}: a link");
                assert_eq!(parse_dash("1,000,000"), None, "{at}: a link");
                assert_eq!(parse_fiat("1,234.56"), None, "{at}: a fiat amount alike");
                // Too many decimals is said as such.
                assert!(too_precise("0,000000001"), "{at}");
                assert_eq!(amount_hint(""), None, "{at}: an empty field says nothing");
                // What "Max" and the defaults write reads back, in every unit.
                for unit in [Unit::Dash, Unit::MilliDash, Unit::Duffs] {
                    with(unit, style);
                    for duffs in [1, 1_234_567, 123_456_700_000_001] {
                        assert_eq!(
                            parse_amount(&input_text(duffs)),
                            Some(duffs),
                            "{at} {unit:?}"
                        );
                    }
                    // A pill's label, its thousands apart by a no-break space, reads back.
                    for duffs in [10_000_000, 100_000_000, 123_456_700_000] {
                        let label = short_number(duffs);
                        assert_eq!(parse_amount(&label), Some(duffs), "{at} {unit:?} «{label}»");
                    }
                }
            }
        }
        crate::i18n::set(crate::i18n::Lang::En);
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
        // BUG-161, the designer: thousands apart by a no-break space, never a comma.
        assert_eq!(short_number(100_000_000), "1\u{a0}000");
        with(Unit::Duffs, NumberStyle::CommaDot);
        assert_eq!(short_number(100_000_000), "100\u{a0}000\u{a0}000");
        set_display(Display::default());
    }

    #[test]
    fn too_many_decimals_are_told_apart_from_a_wrong_amount() {
        with(Unit::Dash, NumberStyle::CommaDot);
        assert!(too_precise("0.000000018"));
        assert!(
            too_precise("0,000000018"),
            "a comma is a decimal mark too (BUG-161)"
        );
        assert!(
            !too_precise("1,234.123456789"),
            "two marks: not one number at all"
        );
        assert!(!too_precise("0.00000001"), "a duff is fine");
        assert!(!too_precise("abc"));
        assert!(!too_precise("0,25"), "an amount");
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

    /// BUG-161 r1 Should 1, the customer's decision: in English «1,000» and «2,500» are a
    /// thousand and two and a half thousand to the person, so they are neither 1 nor 2.5: refused
    /// with a line that asks which. In Russian the comma is only decimal: «1,000» is 1. In German
    /// the point groups thousands: «1.000» asks, «1,000» is 1. «0,002» and «0.002» read alike
    /// everywhere, and a pill's «1 000» (no-break space) reads back as 1000.
    #[test]
    fn a_number_this_format_reads_as_thousands_is_refused_not_guessed() {
        use crate::i18n::Lang;
        crate::i18n::set(Lang::En);
        with(Unit::Dash, NumberStyle::Language);
        for unclear_text in ["1,000", "2,500", " 1,000 "] {
            assert_eq!(parse_amount(unclear_text), None, "en: {unclear_text}");
        }
        assert_eq!(
            amount_hint("1,000").as_deref(),
            Some("Unclear: is 1,000 1 or 1000? Type 1000 or 1.0.")
        );
        assert_eq!(
            amount_hint("2,500").as_deref(),
            Some("Unclear: is 2,500 2.5 or 2500? Type 2500 or 2.5.")
        );
        for (typed, duffs) in [
            ("0,002", 200_000),
            ("0.002", 200_000),
            ("1,25", 125_000_000),
            ("1000", 100_000_000_000),
            ("1.000", 100_000_000),
            ("1\u{a0}000", 100_000_000_000),
        ] {
            assert_eq!(parse_amount(typed), Some(duffs), "en: {typed}");
        }
        crate::i18n::set(Lang::Ru);
        for (typed, duffs) in [
            ("1,000", 100_000_000),
            ("1.000", 100_000_000),
            ("0,002", 200_000),
            ("0.002", 200_000),
            ("1\u{a0}000", 100_000_000_000),
        ] {
            assert_eq!(parse_amount(typed), Some(duffs), "ru: {typed}");
        }
        // The line in another language, as it is drawn (`i18n::t`), its places in order:
        // Russian with «1,234.56» chosen.
        with(Unit::Dash, NumberStyle::CommaDot);
        assert_eq!(
            crate::i18n::t(&amount_hint("1,000").unwrap()),
            "Непонятно: 1,000 — это 1 или 1000? Напишите 1000 или 1.0."
        );
        with(Unit::Dash, NumberStyle::Language);
        crate::i18n::set(Lang::De);
        assert_eq!(
            parse_amount("1.000"),
            None,
            "de: the point groups thousands"
        );
        assert!(amount_hint("1.000").is_some());
        assert_eq!(
            parse_amount("1,000"),
            Some(100_000_000),
            "de: the comma is decimal"
        );
        assert_eq!(parse_amount("0,002"), Some(200_000));
        assert_eq!(parse_amount("0.002"), Some(200_000));
        // A `dash:` link is a machine's: «1.000» there is 1 DASH in any interface.
        assert_eq!(parse_dash("1.000"), Some(100_000_000), "de: a link");
        crate::i18n::set(Lang::En);
        assert_eq!(parse_dash("1,000"), Some(100_000_000), "en: a link");
        // Pills in every unit read back, «1 000» mDASH among them.
        for unit in [Unit::Dash, Unit::MilliDash, Unit::Duffs] {
            with(unit, NumberStyle::Language);
            for duffs in [10_000_000, 100_000_000, 123_456_700_000] {
                assert_eq!(parse_amount(&short_number(duffs)), Some(duffs), "{unit:?}");
            }
        }
        set_display(Display::default());
    }

    /// BUG-161, the customer: no field that takes money, tokens or anything of value may read
    /// typed text by a rule of its own. Every place in `evolite-ui` and `evolite-api` that turns
    /// text into a number is listed here with why it is not an amount; a new one fails this test
    /// until it goes through [`typed_parts`] in this file, or is listed with its reason.
    #[test]
    fn no_amount_is_read_outside_format() {
        use std::path::{Path, PathBuf};
        const READS: [&str; 6] = [
            ".parse::<",
            ".parse()",
            "from_str_radix(",
            "split_once('.')",
            "split_once(',')",
            "f64::from_str",
        ];
        // (file under crates/, how many, why it is not an amount typed by the person)
        const NOT_AMOUNTS: &[(&str, usize, &str)] = &[
            ("evolite-ui/src/format.rs", 2, "the one rule itself"),
            (
                "evolite-ui/src/diagnostics.rs",
                1,
                "EVOLITE_SPV_LOG's tracing filter (dev-tools, BUG-221 trace), not a number",
            ),
            (
                "evolite-api/src/tokens.rs",
                1,
                "a token's units from «1250.5», fed only by format::typed_parts",
            ),
            ("evolite-ui/src/app/dev.rs", 1, "a dev-tools flag"),
            ("evolite-ui/src/cli.rs", 2, "--window WxH"),
            (
                "evolite-ui/src/clipboard.rs",
                1,
                "the hex SHA-256 of the clipboard-secret mark (SEC-2 B5)",
            ),
            (
                "evolite-ui/src/components/para.rs",
                1,
                "an e-mail's domain, to set it in mono (BUG-159)",
            ),
            ("evolite-ui/src/i18n/catalog.rs", 2, "a plural form's index"),
            ("evolite-ui/src/network_choice.rs", 1, "a node's port"),
            ("evolite-ui/src/payment_uri.rs", 1, "a link's %XX escapes"),
            (
                "evolite-ui/src/screens/tokens/state.rs",
                1,
                "writes an amount, does not read one",
            ),
            ("evolite-ui/src/setup/plan.rs", 1, "the installed version"),
            ("evolite-ui/src/setup/system.rs", 1, "a process id"),
            (
                "evolite-ui/src/app/ui_tests/bugs.rs",
                1,
                "a test reading a count",
            ),
            (
                "evolite-ui/src/app/ui_tests/identity_pool.rs",
                1,
                "a test counting decimals",
            ),
        ];
        fn files(dir: &Path, out: &mut Vec<PathBuf>) {
            for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    files(&path, out);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    out.push(path);
                }
            }
        }
        let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut all = Vec::new();
        files(&crates.join("evolite-ui/src"), &mut all);
        files(&crates.join("evolite-api/src"), &mut all);
        let mut found = Vec::new();
        for path in all {
            let text = std::fs::read_to_string(&path).unwrap();
            let n: usize = READS.iter().map(|r| text.matches(r).count()).sum();
            // This test names the reads above: not one of them.
            let n = if path.ends_with("format.rs") {
                n - READS.len()
            } else {
                n
            };
            if n > 0 {
                let name = path
                    .strip_prefix(&crates)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                found.push((name, n));
            }
        }
        for (name, n) in &found {
            let listed = NOT_AMOUNTS.iter().find(|(f, _, _)| f == name);
            assert!(
                listed.is_some_and(|(_, count, _)| n <= count),
                "{name}: {n} read(s) of a number, listed {:?}. An amount goes through \
                 format::typed_parts; anything else is listed in NOT_AMOUNTS with why",
                listed.map(|(_, c, _)| c)
            );
        }
        assert!(found.len() >= 5, "the scan found the files: {found:?}");
    }

    #[test]
    fn the_rate_gives_cents_without_floats() {
        set_display(Display {
            rate: Some(Rate {
                price: 4_796_150_000,
                currency: *b"USD",
                at: 0,
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
