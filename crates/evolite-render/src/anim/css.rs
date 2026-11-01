//! CSS animations: `@keyframes` and the `animation` property.
//!
//! Rules come from the file's `<style>` (selectors matched by `simplecss`, as usvg does) and
//! from `style` attributes; `@keyframes`, which `simplecss` skips, are read here. At each moment
//! the animated properties are worked out and written into the element's `style` attribute,
//! which beats the style sheet. A `transform` is written in SVG syntax (usvg does not read
//! CSS functions such as `translateX(5px)` or units such as `deg`) and with its pivot already
//! applied, so `transform-origin` is reset to `0 0`: usvg knows neither `transform-box` nor
//! a pivot relative to the element itself.
//!
//! Understood: `animation` and its longhands (name, duration, delay — negative too — timing
//! function `ease…`/`linear`/`cubic-bezier`/`steps`, iteration count, direction, fill mode),
//! several animations on one element, keyframes `from`/`to`/`N%` (lists too, missing ends
//! taken from the element), per-keyframe timing functions; `transform` (translate, scale,
//! rotate, skew, matrix and their X/Y forms; `px`, `deg`, `rad`, `turn`, `grad`), `opacity`,
//! and any number or colour (`fill`, `stroke-dashoffset`…); `transform-origin` in lengths,
//! percentages and keywords; `transform-box: fill-box`.

use roxmltree::{Document, Node};
use simplecss::{AttributeOperator, DeclarationTokenizer, PseudoClass, StyleSheet};

use super::values::{clock, interpolate, number};
use super::{Edit, Slot, Timing};
use crate::frames::Limits;

/// A timing function.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Easing {
    Linear,
    Bezier(f64, f64, f64, f64),
    Steps(u32, StepPosition),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum StepPosition {
    Start,
    End,
    None,
    Both,
}

impl Easing {
    fn parse(text: &str) -> Option<Easing> {
        let text = text.trim();
        Some(match text {
            "linear" => Easing::Linear,
            "ease" => Easing::Bezier(0.25, 0.1, 0.25, 1.0),
            "ease-in" => Easing::Bezier(0.42, 0.0, 1.0, 1.0),
            "ease-out" => Easing::Bezier(0.0, 0.0, 0.58, 1.0),
            "ease-in-out" => Easing::Bezier(0.42, 0.0, 0.58, 1.0),
            "step-start" => Easing::Steps(1, StepPosition::Start),
            "step-end" => Easing::Steps(1, StepPosition::End),
            _ => {
                let (name, args) = function(text)?;
                match name {
                    "cubic-bezier" => {
                        let n: Vec<f64> =
                            args.iter().map(|a| a.parse().ok()).collect::<Option<_>>()?;
                        let [x1, y1, x2, y2] = n.as_slice() else {
                            return None;
                        };
                        Easing::Bezier(x1.clamp(0.0, 1.0), *y1, x2.clamp(0.0, 1.0), *y2)
                    }
                    "steps" => {
                        let count: u32 = args.first()?.parse().ok().filter(|n| *n > 0)?;
                        let position = match args.get(1).map(String::as_str) {
                            None | Some("end" | "jump-end") => StepPosition::End,
                            Some("start" | "jump-start") => StepPosition::Start,
                            Some("jump-none") if count > 1 => StepPosition::None,
                            Some("jump-both") => StepPosition::Both,
                            _ => return None,
                        };
                        Easing::Steps(count, position)
                    }
                    _ => return None,
                }
            }
        })
    }

    /// The output progress for input progress `x` (0…1).
    fn apply(&self, x: f64) -> f64 {
        match *self {
            Easing::Linear => x,
            Easing::Bezier(x1, y1, x2, y2) => {
                let at = |a: f64, b: f64, t: f64| {
                    3.0 * a * t * (1.0 - t) * (1.0 - t) + 3.0 * b * t * t * (1.0 - t) + t * t * t
                };
                // The curve's x is monotonic: halve the interval until it hits `x`.
                let (mut low, mut high) = (0.0, 1.0);
                let mut t = x;
                for _ in 0..40 {
                    let fx = at(x1, x2, t);
                    if (fx - x).abs() < 1e-7 {
                        break;
                    }
                    if fx < x {
                        low = t;
                    } else {
                        high = t;
                    }
                    t = (low + high) / 2.0;
                }
                at(y1, y2, t)
            }
            Easing::Steps(n, position) => {
                let n = n as f64;
                let jumps = match position {
                    StepPosition::Start | StepPosition::End => n,
                    StepPosition::None => n - 1.0,
                    StepPosition::Both => n + 1.0,
                };
                let mut step = (x * n).floor();
                if matches!(position, StepPosition::Start | StepPosition::Both) {
                    step += 1.0;
                }
                (step.clamp(0.0, jumps)) / jumps
            }
        }
    }
}

/// `name(a, b c)` as the name and its arguments.
fn function(text: &str) -> Option<(&str, Vec<String>)> {
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    let args = text[open + 1..close]
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|a| !a.is_empty())
        .map(str::to_owned)
        .collect();
    Some((text[..open].trim(), args))
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Direction {
    Normal,
    Reverse,
    Alternate,
    AlternateReverse,
}

/// One animation of an element.
#[derive(Clone, Debug)]
struct Layer {
    name: String,
    duration: f64,
    easing: Easing,
    delay: f64,
    iterations: f64,
    direction: Direction,
    forwards: bool,
    backwards: bool,
}

impl Layer {
    /// Progress through the keyframes (0…1) at `t` seconds, or `None` when not in effect.
    fn progress(&self, t: f64) -> Option<f64> {
        let local = t - self.delay;
        let active = self.duration * self.iterations;
        let (iteration, p) = if local < 0.0 {
            if !self.backwards {
                return None;
            }
            (0.0, 0.0)
        } else if local >= active {
            if !self.forwards {
                return None;
            }
            let fraction = self.iterations.fract();
            if fraction > 0.0 {
                (self.iterations.floor(), fraction)
            } else {
                (self.iterations - 1.0, 1.0)
            }
        } else {
            (
                (local / self.duration).floor(),
                (local % self.duration) / self.duration,
            )
        };
        let odd = iteration as u64 % 2 == 1;
        let reversed = match self.direction {
            Direction::Normal => false,
            Direction::Reverse => true,
            Direction::Alternate => odd,
            Direction::AlternateReverse => !odd,
        };
        Some(if reversed { 1.0 - p } else { p })
    }

    fn timing(&self) -> Timing {
        let alternates = matches!(
            self.direction,
            Direction::Alternate | Direction::AlternateReverse
        );
        let loops = self.iterations.is_infinite();
        Timing {
            begin: self.delay.max(0.0),
            period: if alternates {
                2.0 * self.duration
            } else {
                self.duration
            },
            loops,
            last_change: if loops {
                f64::INFINITY
            } else {
                (self.delay + self.duration * self.iterations).max(0.0)
            },
        }
    }
}

/// The values of the `animation-*` properties as declared: comma lists, one entry a layer.
#[derive(Default)]
struct Declared {
    names: Vec<String>,
    durations: Vec<String>,
    easings: Vec<String>,
    delays: Vec<String>,
    iterations: Vec<String>,
    directions: Vec<String>,
    fills: Vec<String>,
    origin: Option<String>,
    fill_box: bool,
    opacity: Option<String>,
}

fn list(value: &str) -> Vec<String> {
    split_top(value, ',')
        .into_iter()
        .map(|v| v.trim().to_owned())
        .collect()
}

/// `value` split at `separator` outside parentheses.
fn split_top(value: &str, separator: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut depth, mut start) = (0i32, 0);
    for (i, c) in value.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            c if c == separator && depth == 0 => {
                parts.push(&value[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&value[start..]);
    parts
}

/// A time with its unit (`1.5s`, `300ms`): bare numbers are not times in CSS.
fn time(token: &str) -> Option<f64> {
    (token.ends_with('s') && !token.ends_with("ss"))
        .then(|| clock(token))
        .flatten()
}

impl Declared {
    fn declare(&mut self, name: &str, value: &str) {
        let value = value.trim();
        match name {
            "animation" => self.shorthand(value),
            "animation-name" => self.names = list(value),
            "animation-duration" => self.durations = list(value),
            "animation-timing-function" => self.easings = list(value),
            "animation-delay" => self.delays = list(value),
            "animation-iteration-count" => self.iterations = list(value),
            "animation-direction" => self.directions = list(value),
            "animation-fill-mode" => self.fills = list(value),
            "transform-origin" => self.origin = Some(value.to_owned()),
            "transform-box" => self.fill_box = value == "fill-box",
            "opacity" => self.opacity = Some(value.to_owned()),
            _ => {}
        }
    }

    /// `animation: name 2s ease-in-out -1s infinite alternate both, …` sets every longhand.
    fn shorthand(&mut self, value: &str) {
        *self = Declared {
            origin: self.origin.take(),
            fill_box: self.fill_box,
            opacity: self.opacity.take(),
            ..Declared::default()
        };
        for layer in split_top(value, ',') {
            let (mut name, mut duration, mut delay) = (String::from("none"), None, None);
            let (mut easing, mut count, mut direction, mut fill) = (
                "ease".to_owned(),
                "1".to_owned(),
                "normal".to_owned(),
                "none".to_owned(),
            );
            for token in split_top(layer.trim(), ' ')
                .into_iter()
                .filter(|t| !t.is_empty())
            {
                if let Some(t) = time(token) {
                    if duration.is_none() {
                        duration = Some(t);
                    } else {
                        delay = Some(t);
                    }
                } else if Easing::parse(token).is_some() {
                    easing = token.to_owned();
                } else if token == "infinite" || token.parse::<f64>().is_ok() {
                    count = token.to_owned();
                } else if matches!(
                    token,
                    "normal" | "reverse" | "alternate" | "alternate-reverse"
                ) {
                    direction = token.to_owned();
                } else if matches!(token, "forwards" | "backwards" | "both") {
                    fill = token.to_owned();
                } else if !matches!(token, "running" | "paused" | "none") {
                    name = token.to_owned();
                }
            }
            self.names.push(name);
            self.durations.push(format!("{}s", duration.unwrap_or(0.0)));
            self.delays.push(format!("{}s", delay.unwrap_or(0.0)));
            self.easings.push(easing);
            self.iterations.push(count);
            self.directions.push(direction);
            self.fills.push(fill);
        }
    }

    /// The layers, each longhand list repeated to the number of names.
    fn layers(&self) -> Vec<Layer> {
        let pick = |list: &[String], i: usize| -> Option<String> {
            (!list.is_empty()).then(|| list[i % list.len()].clone())
        };
        (0..self.names.len())
            .filter_map(|i| {
                let name = self.names[i].clone();
                let duration = pick(&self.durations, i)
                    .and_then(|d| time(&d))
                    .unwrap_or(0.0);
                if name == "none" || duration <= 0.0 {
                    return None;
                }
                let fill = pick(&self.fills, i).unwrap_or_default();
                Some(Layer {
                    name,
                    duration,
                    easing: pick(&self.easings, i)
                        .and_then(|e| Easing::parse(&e))
                        .unwrap_or(Easing::Bezier(0.25, 0.1, 0.25, 1.0)),
                    delay: pick(&self.delays, i).and_then(|d| time(&d)).unwrap_or(0.0),
                    iterations: match pick(&self.iterations, i).as_deref() {
                        Some("infinite") => f64::INFINITY,
                        Some(n) => n.parse::<f64>().ok().filter(|n| *n > 0.0).unwrap_or(1.0),
                        None => 1.0,
                    },
                    direction: match pick(&self.directions, i).as_deref() {
                        Some("reverse") => Direction::Reverse,
                        Some("alternate") => Direction::Alternate,
                        Some("alternate-reverse") => Direction::AlternateReverse,
                        _ => Direction::Normal,
                    },
                    forwards: matches!(fill.as_str(), "forwards" | "both"),
                    backwards: matches!(fill.as_str(), "backwards" | "both"),
                })
            })
            .collect()
    }
}

/// One keyframe: where (0…1), what it declares, and the timing function to the next one.
#[derive(Debug)]
struct Keyframe {
    offset: f64,
    properties: Vec<(String, String)>,
    easing: Option<Easing>,
}

/// The `@keyframes` blocks of a style sheet, by name; a later block of a name wins.
fn keyframes(css: &str) -> Vec<(String, Vec<Keyframe>)> {
    let mut blocks: Vec<(String, Vec<Keyframe>)> = Vec::new();
    let mut rest = css;
    while let Some(at) = rest.find('@') {
        rest = &rest[at + 1..];
        let keyword_end = rest
            .find(|c: char| c.is_whitespace() || c == '{')
            .unwrap_or(rest.len());
        let keyword = &rest[..keyword_end];
        let Some(open) = rest.find('{') else { break };
        if !keyword.ends_with("keyframes") {
            continue;
        }
        let name = rest[keyword_end..open]
            .trim()
            .trim_matches(|c| c == '"' || c == '\'')
            .to_owned();
        // The block ends where its braces balance.
        let mut depth = 0;
        let mut close = None;
        for (i, c) in rest[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(open + i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(close) = close else { break };
        let body = &rest[open + 1..close];
        rest = &rest[close..];

        let mut frames: Vec<Keyframe> = Vec::new();
        let mut inner = body;
        while let Some(brace) = inner.find('{') {
            let selectors = &inner[..brace];
            let Some(end) = inner[brace..].find('}') else {
                break;
            };
            let declarations = &inner[brace + 1..brace + end];
            inner = &inner[brace + end + 1..];
            let mut properties = Vec::new();
            let mut easing = None;
            for d in DeclarationTokenizer::from(declarations) {
                if d.name == "animation-timing-function" {
                    easing = Easing::parse(d.value);
                } else {
                    properties.push((d.name.to_owned(), d.value.trim().to_owned()));
                }
            }
            for selector in selectors.split(',') {
                let offset = match selector.trim() {
                    "from" => 0.0,
                    "to" => 1.0,
                    s => match s
                        .strip_suffix('%')
                        .and_then(|n| n.trim().parse::<f64>().ok())
                    {
                        Some(p) if (0.0..=100.0).contains(&p) => p / 100.0,
                        _ => continue,
                    },
                };
                match frames.iter_mut().find(|f| f.offset == offset) {
                    Some(frame) => {
                        frame.properties.extend(properties.iter().cloned());
                        frame.easing = easing.or(frame.easing);
                    }
                    None => frames.push(Keyframe {
                        offset,
                        properties: properties.clone(),
                        easing,
                    }),
                }
            }
        }
        frames.sort_by(|a, b| a.offset.total_cmp(&b.offset));
        blocks.retain(|(n, _)| *n != name);
        blocks.push((name, frames));
    }
    blocks
}

/// A transform function, in the form SVG takes it.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Func {
    Translate(f64, f64),
    Scale(f64, f64),
    /// Degrees.
    Rotate(f64),
    /// Degrees, x and y.
    Skew(f64, f64),
    Matrix([f64; 6]),
}

impl Func {
    fn identity(&self) -> Func {
        match self {
            Func::Translate(..) => Func::Translate(0.0, 0.0),
            Func::Scale(..) => Func::Scale(1.0, 1.0),
            Func::Rotate(_) => Func::Rotate(0.0),
            Func::Skew(..) => Func::Skew(0.0, 0.0),
            Func::Matrix(_) => Func::Matrix([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
        }
    }

    fn lerp(&self, other: &Func, f: f64) -> Option<Func> {
        let l = |a: f64, b: f64| a + (b - a) * f;
        Some(match (self, other) {
            (Func::Translate(a, b), Func::Translate(c, d)) => Func::Translate(l(*a, *c), l(*b, *d)),
            (Func::Scale(a, b), Func::Scale(c, d)) => Func::Scale(l(*a, *c), l(*b, *d)),
            (Func::Rotate(a), Func::Rotate(b)) => Func::Rotate(l(*a, *b)),
            (Func::Skew(a, b), Func::Skew(c, d)) => Func::Skew(l(*a, *c), l(*b, *d)),
            (Func::Matrix(a), Func::Matrix(b)) => {
                Func::Matrix(std::array::from_fn(|i| l(a[i], b[i])))
            }
            _ => return None,
        })
    }

    fn svg(&self) -> String {
        let n = |v: f64| number(v);
        match self {
            Func::Translate(x, y) => format!("translate({} {})", n(*x), n(*y)),
            Func::Scale(x, y) => format!("scale({} {})", n(*x), n(*y)),
            Func::Rotate(a) => format!("rotate({})", n(*a)),
            Func::Skew(x, y) => format!("skewX({}) skewY({})", n(*x), n(*y)),
            Func::Matrix(m) => format!(
                "matrix({} {} {} {} {} {})",
                n(m[0]),
                n(m[1]),
                n(m[2]),
                n(m[3]),
                n(m[4]),
                n(m[5])
            ),
        }
    }
}

/// A length in px (bare numbers too); percentages are not lengths here.
fn px(token: &str) -> Option<f64> {
    token.strip_suffix("px").unwrap_or(token).parse().ok()
}

/// An angle in degrees.
fn degrees(token: &str) -> Option<f64> {
    let (number, scale) = if let Some(n) = token.strip_suffix("deg") {
        (n, 1.0)
    } else if let Some(n) = token.strip_suffix("grad") {
        (n, 0.9)
    } else if let Some(n) = token.strip_suffix("rad") {
        (n, 180.0 / std::f64::consts::PI)
    } else if let Some(n) = token.strip_suffix("turn") {
        (n, 360.0)
    } else {
        (token, 1.0)
    };
    Some(number.parse::<f64>().ok()? * scale)
}

/// A CSS `transform` value; `None` for what is not understood (then it is not animated).
fn transform(value: &str) -> Option<Vec<Func>> {
    let value = value.trim();
    if value == "none" {
        return Some(Vec::new());
    }
    let mut funcs = Vec::new();
    let mut rest = value;
    while !rest.trim().is_empty() {
        let close = rest.find(')')?;
        let (name, args) = function(&rest[..=close])?;
        rest = &rest[close + 1..];
        let a = |i: usize| args.get(i).map(String::as_str);
        let len = |i| a(i).and_then(px);
        let deg = |i| a(i).and_then(degrees);
        let num = |i: usize| a(i).and_then(|v| v.parse::<f64>().ok());
        funcs.push(match name {
            "translate" => Func::Translate(len(0)?, a(1).map_or(Some(0.0), px)?),
            "translateX" => Func::Translate(len(0)?, 0.0),
            "translateY" => Func::Translate(0.0, len(0)?),
            "scale" => {
                let x = num(0)?;
                Func::Scale(x, a(1).map_or(Some(x), |v| v.parse().ok())?)
            }
            "scaleX" => Func::Scale(num(0)?, 1.0),
            "scaleY" => Func::Scale(1.0, num(0)?),
            "rotate" => Func::Rotate(deg(0)?),
            "skew" => Func::Skew(deg(0)?, a(1).map_or(Some(0.0), degrees)?),
            "skewX" => Func::Skew(deg(0)?, 0.0),
            "skewY" => Func::Skew(0.0, deg(0)?),
            "matrix" => Func::Matrix(std::array::from_fn(|i| num(i).unwrap_or(f64::NAN))),
            _ => return None,
        });
    }
    funcs
        .iter()
        .all(|f| !matches!(f, Func::Matrix(m) if m.iter().any(|v| v.is_nan())))
        .then_some(funcs)
}

/// The transform `f` of the way from `a` to `b`: function by function when they match (an
/// empty list stands for the same functions doing nothing), otherwise a jump half way.
fn lerp_transform(a: &[Func], b: &[Func], f: f64) -> Vec<Func> {
    let ids = |list: &[Func]| list.iter().map(Func::identity).collect::<Vec<_>>();
    let (a, b) = match (a.is_empty(), b.is_empty()) {
        (true, false) => (ids(b), b.to_vec()),
        (false, true) => (a.to_vec(), ids(a)),
        _ => (a.to_vec(), b.to_vec()),
    };
    if a.len() == b.len()
        && let Some(funcs) = a
            .iter()
            .zip(&b)
            .map(|(x, y)| x.lerp(y, f))
            .collect::<Option<Vec<_>>>()
    {
        return funcs;
    }
    if f < 0.5 { a } else { b }
}

/// An element with CSS animations.
struct Animated {
    /// Its `style` attribute: where the moment's values are written.
    slot: Slot,
    style: String,
    /// Its own opacity, for keyframes that leave the ends out.
    opacity: String,
    /// The pivot of its transform, in its user space.
    pivot: (f64, f64),
    layers: Vec<Layer>,
}

/// The CSS animations of a document.
pub(super) struct Css {
    elements: Vec<Animated>,
    keyframes: Vec<(String, Vec<Keyframe>)>,
}

/// A `roxmltree` node as `simplecss` matches it.
#[derive(Clone, Copy)]
struct El<'a, 'i>(Node<'a, 'i>);

impl simplecss::Element for El<'_, '_> {
    fn parent_element(&self) -> Option<Self> {
        self.0.parent_element().map(El)
    }

    fn prev_sibling_element(&self) -> Option<Self> {
        self.0.prev_sibling_element().map(El)
    }

    fn has_local_name(&self, name: &str) -> bool {
        self.0.tag_name().name() == name
    }

    fn attribute_matches(&self, local_name: &str, operator: AttributeOperator<'_>) -> bool {
        self.0
            .attribute(local_name)
            .is_some_and(|value| operator.matches(value))
    }

    fn pseudo_class_matches(&self, class: PseudoClass<'_>) -> bool {
        matches!(class, PseudoClass::FirstChild) && self.0.prev_sibling_element().is_none()
    }
}

/// The view box of the root `<svg>`: `transform-origin` percentages and keywords are of it.
fn view_box(xml: &Document) -> (f64, f64) {
    let root = xml.root_element();
    if let Some(vb) = root.attribute("viewBox") {
        let n: Vec<f64> = vb
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter_map(|v| v.parse().ok())
            .collect();
        if let [_, _, w, h] = n.as_slice() {
            return (*w, *h);
        }
    }
    let size = |name| root.attribute(name).and_then(px).unwrap_or(100.0);
    (size("width"), size("height"))
}

/// The pivot for `transform-origin: value` within `bbox` (x, y, width, height).
fn origin(value: Option<&str>, bbox: (f64, f64, f64, f64)) -> (f64, f64) {
    let (x0, y0, w, h) = bbox;
    let Some(value) = value else {
        return (x0, y0);
    };
    let mut tokens: Vec<&str> = value.split_whitespace().take(2).collect();
    if tokens
        .first()
        .is_some_and(|t| matches!(*t, "top" | "bottom"))
        || tokens
            .get(1)
            .is_some_and(|t| matches!(*t, "left" | "right"))
    {
        tokens.reverse();
    }
    let along = |token: Option<&str>, size: f64, start: f64| -> f64 {
        match token {
            None | Some("center") => start + size / 2.0,
            Some("left" | "top") => start,
            Some("right" | "bottom") => start + size,
            Some(t) => match t.strip_suffix('%') {
                Some(p) => start + p.parse::<f64>().unwrap_or(50.0) / 100.0 * size,
                None => start + px(t).unwrap_or(0.0),
            },
        }
    };
    (
        along(tokens.first().copied(), w, x0),
        along(tokens.get(1).copied(), h, y0),
    )
}

/// Object bounding boxes of the given elements, as usvg sees them, from the SVG with an id
/// added where an element has none. An element usvg does not keep gets an empty box.
fn bounding_boxes(text: &str, elements: &[Node], limits: &Limits) -> Vec<(f64, f64, f64, f64)> {
    let mut ids = Vec::new();
    let mut inserts = Vec::new();
    for (i, element) in elements.iter().enumerate() {
        let (slot, own) = Slot::of(text, *element, "id");
        match own {
            Some(id) => ids.push(id),
            None => {
                let id = format!("evolite-box-{i}");
                inserts.push((slot, id.clone()));
                ids.push(id);
            }
        }
    }
    let marked = super::apply(
        text,
        inserts
            .iter()
            .map(|(slot, id)| Edit {
                slot,
                name: "id",
                value: id.clone(),
            })
            .collect(),
    );
    let tree = crate::svg::parse(&marked, limits).and_then(|xml| crate::svg::tree(&xml, limits));
    ids.iter()
        .map(|id| {
            tree.as_ref()
                .ok()
                .and_then(|tree| tree.node_by_id(id))
                .map(|node| {
                    let b = node.bounding_box();
                    (
                        b.x() as f64,
                        b.y() as f64,
                        b.width() as f64,
                        b.height() as f64,
                    )
                })
                .unwrap_or_default()
        })
        .collect()
}

/// The CSS animations of the document, with the elements they move.
pub(super) fn collect(text: &str, xml: &Document, limits: &Limits) -> Css {
    let sheet_text: String = xml
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "style")
        .flat_map(|n| n.children().filter_map(|c| c.text()))
        .collect();
    let blocks = keyframes(&sheet_text);
    let mut rules: Vec<(usize, simplecss::Rule)> = StyleSheet::parse(&sheet_text)
        .rules
        .into_iter()
        .enumerate()
        .collect();
    // Later and more specific rules win: apply them in that order.
    rules.sort_by_key(|(order, rule)| (rule.selector.specificity(), *order));

    let mut nodes = Vec::new();
    for node in xml.descendants().filter(Node::is_element) {
        let mut d = Declared::default();
        for (_, rule) in &rules {
            if rule.selector.matches(&El(node)) {
                for declaration in &rule.declarations {
                    d.declare(declaration.name, declaration.value);
                }
            }
        }
        if let Some(style) = node.attribute("style") {
            for declaration in DeclarationTokenizer::from(style) {
                d.declare(declaration.name, declaration.value);
            }
        }
        let layers: Vec<Layer> = d
            .layers()
            .into_iter()
            .filter(|l| blocks.iter().any(|(name, _)| *name == l.name))
            .collect();
        if !layers.is_empty() {
            nodes.push((node, d, layers));
        }
        if nodes.len() > limits.max_animations {
            break;
        }
    }

    // Boxes only for the elements that turn around themselves.
    let boxed: Vec<Node> = nodes
        .iter()
        .filter(|(_, d, _)| d.fill_box)
        .map(|(n, _, _)| *n)
        .collect();
    let mut boxes = if boxed.is_empty() {
        Vec::new()
    } else {
        bounding_boxes(text, &boxed, limits)
    }
    .into_iter();
    let (vw, vh) = view_box(xml);

    let elements = nodes
        .into_iter()
        .map(|(node, d, layers)| {
            let reference = if d.fill_box {
                boxes.next().unwrap_or_default()
            } else {
                (0.0, 0.0, vw, vh)
            };
            let (slot, style) = Slot::of(text, node, "style");
            Animated {
                slot,
                style: style.unwrap_or_default(),
                opacity: d
                    .opacity
                    .clone()
                    .or_else(|| node.attribute("opacity").map(str::to_owned))
                    .unwrap_or_else(|| "1".into()),
                pivot: origin(d.origin.as_deref(), reference),
                layers,
            }
        })
        .collect();
    Css {
        elements,
        keyframes: blocks,
    }
}

impl Css {
    pub(super) fn len(&self) -> usize {
        self.elements.iter().map(|e| e.layers.len()).sum()
    }

    pub(super) fn timings(&self) -> Vec<Timing> {
        self.elements
            .iter()
            .flat_map(|e| e.layers.iter().map(Layer::timing))
            .collect()
    }

    /// The value of `property` at keyframe progress `p` of `layer`.
    fn value(&self, element: &Animated, layer: &Layer, property: &str, p: f64) -> Option<String> {
        let (_, frames) = self
            .keyframes
            .iter()
            .rev()
            .find(|(name, _)| *name == layer.name)?;
        let base = || match property {
            "opacity" => Some(element.opacity.clone()),
            "transform" => Some("none".to_owned()),
            _ => None,
        };
        // The keyframes that set this property, with the element's own value at a missing end.
        let mut stops: Vec<(f64, String, Option<Easing>)> = frames
            .iter()
            .filter_map(|f| {
                let value = f
                    .properties
                    .iter()
                    .rev()
                    .find(|(name, _)| name == property)?;
                Some((f.offset, value.1.clone(), f.easing))
            })
            .collect();
        if stops.is_empty() {
            return None;
        }
        if stops[0].0 > 0.0 {
            let first = base().unwrap_or_else(|| stops[0].1.clone());
            stops.insert(0, (0.0, first, frames.first().and_then(|f| f.easing)));
        }
        if stops.last().is_some_and(|s| s.0 < 1.0) {
            let last = base().unwrap_or_else(|| stops.last().unwrap().1.clone());
            stops.push((1.0, last, None));
        }
        let i = stops
            .windows(2)
            .position(|w| p <= w[1].0)
            .unwrap_or(stops.len() - 2);
        let (from, to) = (&stops[i], &stops[i + 1]);
        let span = to.0 - from.0;
        let local = if span > 0.0 {
            ((p - from.0) / span).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let eased = from.2.unwrap_or(layer.easing).apply(local);
        Some(if property == "transform" {
            let (a, b) = (transform(&from.1)?, transform(&to.1)?);
            let funcs = lerp_transform(&a, &b, eased);
            let (px_, py) = element.pivot;
            let inner: Vec<String> = funcs.iter().map(Func::svg).collect();
            if funcs.is_empty() {
                "none".to_owned()
            } else if px_ == 0.0 && py == 0.0 {
                inner.join(" ")
            } else {
                format!(
                    "translate({} {}) {} translate({} {})",
                    number(px_),
                    number(py),
                    inner.join(" "),
                    number(-px_),
                    number(-py)
                )
            }
        } else {
            interpolate(&from.1, &to.1, eased)
        })
    }

    /// Every animated element's style at `t` seconds.
    pub(super) fn edits(&self, t: f64) -> Vec<Edit<'_>> {
        let mut edits = Vec::new();
        for element in &self.elements {
            let mut set: Vec<(String, String)> = Vec::new();
            for layer in &element.layers {
                let Some(p) = layer.progress(t) else {
                    continue;
                };
                let Some((_, frames)) = self.keyframes.iter().rev().find(|(n, _)| *n == layer.name)
                else {
                    continue;
                };
                let mut properties: Vec<&str> = frames
                    .iter()
                    .flat_map(|f| f.properties.iter().map(|(name, _)| name.as_str()))
                    .collect();
                properties.sort_unstable();
                properties.dedup();
                for property in properties {
                    if let Some(value) = self.value(element, layer, property, p) {
                        // A later animation of the same property wins.
                        set.retain(|(name, _)| name != property);
                        set.push((property.to_owned(), value));
                    }
                }
            }
            if set.is_empty() {
                continue;
            }
            let mut style = element.style.trim().trim_end_matches(';').to_owned();
            for (name, value) in &set {
                if !style.is_empty() {
                    style.push(';');
                }
                style.push_str(&format!("{name}:{value}"));
                if name == "transform" {
                    // The pivot is in the transform already.
                    style.push_str(";transform-origin:0 0");
                }
            }
            edits.push(Edit {
                slot: &element.slot,
                name: "style",
                value: style,
            });
        }
        edits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timing_functions() {
        let ease_in_out = Easing::parse("ease-in-out").unwrap();
        assert!((ease_in_out.apply(0.5) - 0.5).abs() < 1e-6);
        assert!(ease_in_out.apply(0.25) < 0.25);
        assert_eq!(Easing::parse("linear").unwrap().apply(0.3), 0.3);
        let steps = Easing::parse("steps(4)").unwrap();
        assert_eq!(steps.apply(0.3), 0.25);
        assert_eq!(steps.apply(0.99), 0.75);
        assert_eq!(Easing::parse("steps(1)").unwrap().apply(0.6), 0.0);
        assert_eq!(Easing::parse("step-start").unwrap().apply(0.1), 1.0);
        assert!(Easing::parse("bounce").is_none());
    }

    #[test]
    fn reads_the_animation_shorthand() {
        let mut d = Declared::default();
        d.declare(
            "animation",
            "spin 1.92s ease-in-out -.24s infinite alternate both",
        );
        d.declare("animation-duration", "4s");
        let layers = d.layers();
        assert_eq!(layers.len(), 1);
        let l = &layers[0];
        assert_eq!(l.name, "spin");
        assert_eq!(l.duration, 4.0);
        assert_eq!(l.delay, -0.24);
        assert!(l.iterations.is_infinite());
        assert_eq!(l.direction, Direction::Alternate);
        assert!(l.forwards && l.backwards);
    }

    #[test]
    fn reads_keyframes() {
        let blocks = keyframes(
            ".a{color:red}@keyframes k{0%,100%{transform:none}50%{transform:translateY(-5px);opacity:.2}}@keyframes r{to{transform:rotate(1turn)}}",
        );
        assert_eq!(blocks.len(), 2);
        let (name, frames) = &blocks[0];
        assert_eq!(name, "k");
        assert_eq!(
            frames.iter().map(|f| f.offset).collect::<Vec<_>>(),
            vec![0.0, 0.5, 1.0]
        );
        assert_eq!(blocks[1].1[0].offset, 1.0);
    }

    #[test]
    fn css_transforms_become_svg_ones() {
        let t = transform("translateY(-4px) rotate(1turn) scaleX(.86)").unwrap();
        assert_eq!(
            t,
            vec![
                Func::Translate(0.0, -4.0),
                Func::Rotate(360.0),
                Func::Scale(0.86, 1.0)
            ]
        );
        assert_eq!(transform("none").unwrap(), vec![]);
        assert!(transform("perspective(10px)").is_none());
        let half = lerp_transform(&[], &[Func::Rotate(90.0)], 0.5);
        assert_eq!(half, vec![Func::Rotate(45.0)]);
    }

    #[test]
    fn origins() {
        let box_ = (10.0, 20.0, 100.0, 50.0);
        assert_eq!(origin(None, box_), (10.0, 20.0));
        assert_eq!(origin(Some("center"), box_), (60.0, 45.0));
        assert_eq!(origin(Some("100px 66px"), box_), (110.0, 86.0));
        assert_eq!(origin(Some("50% 100%"), box_), (60.0, 70.0));
        assert_eq!(origin(Some("top right"), box_), (110.0, 20.0));
    }
}
