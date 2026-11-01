//! Values that animations move between: clock times, numbers inside any text (path data,
//! lists, lengths with units) and colours.

use std::str::FromStr;

/// A clock value in seconds: `5`, `2.5s`, `300ms`, `1.5min`, `1h`, `01:30`, `00:01:30.5`.
pub(super) fn clock(text: &str) -> Option<f64> {
    let text = text.trim();
    if text.contains(':') {
        let parts: Vec<f64> = text
            .split(':')
            .map(|p| p.trim().parse::<f64>().ok())
            .collect::<Option<_>>()?;
        return match parts.as_slice() {
            [m, s] => Some(m * 60.0 + s),
            [h, m, s] => Some(h * 3600.0 + m * 60.0 + s),
            _ => None,
        };
    }
    let (number, scale) = if let Some(n) = text.strip_suffix("ms") {
        (n, 0.001)
    } else if let Some(n) = text.strip_suffix("min") {
        (n, 60.0)
    } else if let Some(n) = text.strip_suffix('h') {
        (n, 3600.0)
    } else if let Some(n) = text.strip_suffix('s') {
        (n, 1.0)
    } else {
        (text, 1.0)
    };
    let value = number.trim().parse::<f64>().ok()? * scale;
    value.is_finite().then_some(value)
}

/// A value split into the numbers in it and the text around them: `M10,20 L30,40` is
/// `["M", ",", " L", ",", ""]` and `[10, 20, 30, 40]`.
pub(super) fn split_numbers(value: &str) -> (Vec<String>, Vec<f64>) {
    let bytes = value.as_bytes();
    let mut texts = vec![String::new()];
    let mut numbers = Vec::new();
    let mut i = 0;
    let digit = |i: usize| bytes.get(i).is_some_and(u8::is_ascii_digit);
    while i < bytes.len() {
        let c = bytes[i];
        let starts = digit(i)
            || (c == b'.' && digit(i + 1))
            || ((c == b'-' || c == b'+')
                && (digit(i + 1) || (bytes.get(i + 1) == Some(&b'.') && digit(i + 2))));
        // A digit inside a name (`a01`, `grad1`, `#ff0000`) is not a number of its own; after
        // a lone letter it is (`M10`, `L20` in path data).
        let word_char = |b: u8| b.is_ascii_alphanumeric() || b == b'#' || b == b'_' || b == b'-';
        let in_word = i > 0
            && (bytes[i - 1] == b'#'
                || bytes[i - 1] == b'_'
                || (bytes[i - 1].is_ascii_alphabetic() && i > 1 && word_char(bytes[i - 2])));
        if starts && !in_word {
            let start = i;
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                // A second dot starts the next number (`.5.5` is two numbers in path data).
                if bytes[i] == b'.' && value[start..i].contains('.') {
                    break;
                }
                i += 1;
            }
            if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
                let mut j = i + 1;
                if j < bytes.len() && (bytes[j] == b'-' || bytes[j] == b'+') {
                    j += 1;
                }
                if digit(j) {
                    i = j;
                    while digit(i) {
                        i += 1;
                    }
                }
            }
            match value[start..i].parse::<f64>() {
                Ok(n) => {
                    numbers.push(n);
                    texts.push(String::new());
                }
                Err(_) => texts.last_mut().unwrap().push_str(&value[start..i]),
            }
        } else {
            let ch = value[i..].chars().next().unwrap();
            texts.last_mut().unwrap().push(ch);
            i += ch.len_utf8();
        }
    }
    (texts, numbers)
}

/// A number as SVG wants it: no exponent, at most four decimals, no trailing zeros.
pub(super) fn number(n: f64) -> String {
    let text = format!("{n:.4}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".into()
    } else {
        text.into()
    }
}

fn join(texts: &[String], numbers: &[f64]) -> String {
    let mut out = texts[0].clone();
    for (n, text) in numbers.iter().zip(&texts[1..]) {
        out.push_str(&number(*n));
        out.push_str(text);
    }
    out
}

/// A colour, unless the value is a number (`0` would read as a colour name otherwise).
pub(super) fn colour(value: &str) -> Option<svgtypes::Color> {
    let v = value.trim();
    if v.is_empty() || v.starts_with(|c: char| c.is_ascii_digit() || c == '-' || c == '.') {
        return None;
    }
    svgtypes::Color::from_str(v).ok()
}

/// The value `f` of the way from `a` to `b`. Numbers move, colours blend; values of different
/// shape jump half way.
pub(super) fn interpolate(a: &str, b: &str, f: f64) -> String {
    if let (Some(x), Some(y)) = (colour(a), colour(b)) {
        let mix = |p: u8, q: u8| {
            (p as f64 + (q as f64 - p as f64) * f)
                .round()
                .clamp(0.0, 255.0) as u8
        };
        let (r, g, bl, al) = (
            mix(x.red, y.red),
            mix(x.green, y.green),
            mix(x.blue, y.blue),
            mix(x.alpha, y.alpha),
        );
        return if al == 255 {
            format!("rgb({r},{g},{bl})")
        } else {
            format!("rgba({r},{g},{bl},{})", number(al as f64 / 255.0))
        };
    }
    let (texts_a, numbers_a) = split_numbers(a);
    let (texts_b, numbers_b) = split_numbers(b);
    if texts_a == texts_b && numbers_a.len() == numbers_b.len() {
        let numbers: Vec<f64> = numbers_a
            .iter()
            .zip(&numbers_b)
            .map(|(p, q)| p + (q - p) * f)
            .collect();
        return join(&texts_a, &numbers);
    }
    if f < 0.5 { a.into() } else { b.into() }
}

/// `a + b` for `by` animations, when both have the same shape; otherwise `b`.
pub(super) fn add(a: &str, b: &str) -> String {
    let (texts_a, numbers_a) = split_numbers(a);
    let (texts_b, numbers_b) = split_numbers(b);
    if numbers_a.len() == numbers_b.len() && (texts_a == texts_b || numbers_a.len() == 1) {
        let numbers: Vec<f64> = numbers_a
            .iter()
            .zip(&numbers_b)
            .map(|(p, q)| p + q)
            .collect();
        return join(&texts_b, &numbers);
    }
    b.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_clock_values() {
        assert_eq!(clock("5"), Some(5.0));
        assert_eq!(clock("2.5s"), Some(2.5));
        assert_eq!(clock("300ms"), Some(0.3));
        assert_eq!(clock("1.5min"), Some(90.0));
        assert_eq!(clock("01:30"), Some(90.0));
        assert_eq!(clock("00:01:30.5"), Some(90.5));
        assert_eq!(clock("-1s"), Some(-1.0));
        assert_eq!(clock("a.click"), None);
        assert_eq!(clock("indefinite"), None);
    }

    #[test]
    fn numbers_move_colours_blend_the_rest_jumps() {
        assert_eq!(interpolate("0", "100", 0.25), "25");
        assert_eq!(interpolate("10px", "20px", 0.5), "15px");
        assert_eq!(
            interpolate("M0,0 L10,10", "M10,0 L20,30", 0.5),
            "M5,0 L15,20"
        );
        assert_eq!(interpolate("0 0 100 100", "0 0 50 50", 0.5), "0 0 75 75");
        assert_eq!(interpolate("red", "blue", 0.5), "rgb(128,0,128)");
        assert_eq!(
            interpolate("#ff0000", "#ff000000", 0.5),
            "rgba(255,0,0,0.502)"
        );
        assert_eq!(interpolate("visible", "hidden", 0.4), "visible");
        assert_eq!(interpolate("visible", "hidden", 0.6), "hidden");
        // Numbers inside words are not numbers.
        assert_eq!(split_numbers("url(#a1)").1, Vec::<f64>::new());
        assert_eq!(split_numbers("1e2-3").1, vec![100.0, -3.0]);
    }
}
