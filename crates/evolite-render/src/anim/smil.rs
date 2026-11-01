//! SMIL: `<animate>`, `<animateTransform>`, `<set>`.
//!
//! Understood: `<animate>` (numbers, lists of numbers such as path data or `viewBox`, colours;
//! `from`/`to`/`by`/`values`, `keyTimes`, `calcMode` linear or discrete), `<animateTransform>`
//! (translate, scale, rotate, skewX, skewY; `additive="sum"`), `<set>`; `begin` as a time,
//! `dur`, `repeatCount`, `repeatDur`, `fill="freeze"`; a target by `href="#id"` or the parent.
//!
//! Not understood, and drawn as at rest: `<animateMotion>`, `begin` on events ("on click"),
//! spline timing (drawn linear).

use roxmltree::{Document, Node, NodeId};

use super::values::{add, clock, interpolate};
use super::{Edit, Slot, Timing};

/// One attribute of one element that animations change.
struct Target {
    element: NodeId,
    name: String,
    slot: Slot,
    /// The element's own value, if it has one.
    base: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Shape {
    /// The value is written as it is.
    Plain,
    /// The value is the numbers of an `animateTransform` of this type.
    Transform(&'static str),
}

/// One animation element, resolved.
struct Track {
    target: usize,
    keyframes: Vec<String>,
    key_times: Option<Vec<f64>>,
    discrete: bool,
    begin: f64,
    /// Seconds; infinite for a `<set>` without `dur`.
    dur: f64,
    /// Iterations; infinite for `indefinite`.
    repeat: f64,
    freeze: bool,
    shape: Shape,
    sum: bool,
}

/// A transform of this type that does nothing, for `to`/`by` without `from`.
fn identity(kind: &str) -> &'static str {
    match kind {
        "scale" => "1",
        "translate" => "0 0",
        _ => "0",
    }
}

impl Track {
    /// The value at `t` seconds, or `None` before it starts and after it ends (unless frozen).
    fn sample(&self, t: f64) -> Option<String> {
        let local = t - self.begin;
        if local < 0.0 {
            return None;
        }
        let active = self.dur * self.repeat;
        let progress = if local >= active {
            if !self.freeze {
                return None;
            }
            let fraction = self.repeat.fract();
            if self.repeat.is_finite() && fraction > 0.0 {
                fraction
            } else {
                1.0
            }
        } else if self.dur.is_finite() {
            (local % self.dur) / self.dur
        } else {
            0.0
        };
        Some(self.at(progress))
    }

    /// The value at `p` (0…1) of one iteration.
    fn at(&self, p: f64) -> String {
        let k = &self.keyframes;
        let n = k.len();
        if n == 1 {
            return k[0].clone();
        }
        if self.discrete {
            let i = match &self.key_times {
                Some(times) => times.iter().rposition(|t| *t <= p).unwrap_or(0),
                None => ((p * n as f64) as usize).min(n - 1),
            };
            return k[i].clone();
        }
        let (i, f) = match &self.key_times {
            Some(times) => {
                let i = times.windows(2).position(|w| p <= w[1]).unwrap_or(n - 2);
                let span = times[i + 1] - times[i];
                let f = if span > 0.0 {
                    (p - times[i]) / span
                } else {
                    1.0
                };
                (i, f.clamp(0.0, 1.0))
            }
            None => {
                let segment = p * (n - 1) as f64;
                let i = (segment as usize).min(n - 2);
                (i, segment - i as f64)
            }
        };
        interpolate(&k[i], &k[i + 1], f)
    }

    /// Repeats for ever: the picture loops.
    fn loops(&self) -> bool {
        self.dur.is_finite() && self.repeat.is_infinite()
    }

    /// The last moment this animation changes anything: its end, or its start for one that
    /// holds a value for ever (a `<set>` without `dur`). Infinite for a loop.
    fn last_change(&self) -> f64 {
        if self.loops() {
            f64::INFINITY
        } else if self.dur.is_infinite() {
            self.begin
        } else {
            self.begin + self.dur * self.repeat
        }
    }
}

/// The attributes that animations may not touch: identity, links and inline style.
fn writable(name: &str) -> bool {
    !name.is_empty()
        && !name.contains(':')
        && !matches!(name, "id" | "href" | "style" | "class" | "begin" | "end")
}

fn target_of<'a, 'i>(xml: &'a Document<'i>, animation: Node<'a, 'i>) -> Option<Node<'a, 'i>> {
    let href = animation
        .attributes()
        .find(|a| a.name() == "href")
        .map(|a| a.value());
    match href {
        Some(href) => {
            let id = href.strip_prefix('#')?;
            xml.descendants()
                .find(|n| n.is_element() && n.attribute("id") == Some(id))
        }
        None => animation.parent_element(),
    }
}

/// The SMIL animations of a document, with the attributes they change.
pub(super) struct Smil {
    targets: Vec<Target>,
    tracks: Vec<Track>,
}

impl Smil {
    pub(super) fn len(&self) -> usize {
        self.tracks.len()
    }

    pub(super) fn timings(&self) -> Vec<Timing> {
        self.tracks
            .iter()
            .map(|t| Timing {
                begin: t.begin,
                period: t.dur,
                loops: t.loops(),
                last_change: t.last_change(),
            })
            .collect()
    }

    /// Every animated attribute set to its value at `t` seconds.
    pub(super) fn edits(&self, t: f64) -> Vec<Edit<'_>> {
        let mut edits = Vec::new();
        for (i, target) in self.targets.iter().enumerate() {
            let mut value: Option<String> = None;
            for track in self.tracks.iter().filter(|track| track.target == i) {
                let Some(sample) = track.sample(t) else {
                    continue;
                };
                let sample = match track.shape {
                    Shape::Transform(kind) => format!("{kind}({sample})"),
                    Shape::Plain => sample,
                };
                value = Some(if track.sum {
                    let under = value.or_else(|| target.base.clone()).unwrap_or_default();
                    format!("{under} {sample}").trim().to_owned()
                } else {
                    sample
                });
            }
            if let Some(value) = value {
                edits.push(Edit {
                    slot: &target.slot,
                    name: &target.name,
                    value,
                });
            }
        }
        edits
    }
}

/// All animations of the document that this module can play, with their targets.
pub(super) fn collect(text: &str, xml: &Document) -> Smil {
    let mut targets: Vec<Target> = Vec::new();
    let mut tracks = Vec::new();
    for node in xml.descendants().filter(Node::is_element) {
        let kind = node.tag_name().name();
        if !matches!(kind, "animate" | "animateTransform" | "set") {
            continue;
        }
        let attr = |name: &str| node.attribute(name).map(str::trim);
        let Some(name) = attr("attributeName").filter(|n| writable(n)) else {
            continue;
        };
        let Some(element) = target_of(xml, node) else {
            continue;
        };
        // Only a time: a begin on an event ("a.click") never comes in a still picture.
        let Some(begin) = clock(
            attr("begin")
                .unwrap_or("0")
                .split(';')
                .next()
                .unwrap_or("0"),
        ) else {
            continue;
        };
        let dur = match attr("dur") {
            Some(d) if d != "indefinite" => match clock(d) {
                Some(d) if d > 0.0 => d,
                _ => continue,
            },
            _ if kind == "set" => f64::INFINITY,
            _ => continue,
        };
        let count = match attr("repeatCount") {
            Some("indefinite") => f64::INFINITY,
            Some(n) => n.parse::<f64>().ok().filter(|n| *n > 0.0).unwrap_or(1.0),
            None => 1.0,
        };
        let repeat = match attr("repeatDur") {
            Some("indefinite") => f64::INFINITY,
            Some(d) => clock(d).map_or(count, |d| (d / dur).min(count.max(d / dur))),
            None => count,
        };

        let shape = if kind == "animateTransform" {
            match attr("type").unwrap_or("translate") {
                "translate" => Shape::Transform("translate"),
                "scale" => Shape::Transform("scale"),
                "rotate" => Shape::Transform("rotate"),
                "skewX" => Shape::Transform("skewX"),
                "skewY" => Shape::Transform("skewY"),
                _ => continue,
            }
        } else {
            Shape::Plain
        };

        // The target attribute: one entry per element and name, shared by its animations.
        let target = match targets
            .iter()
            .position(|t| t.element == element.id() && t.name == name)
        {
            Some(i) => i,
            None => {
                let (slot, base) = Slot::of(text, element, name);
                targets.push(Target {
                    element: element.id(),
                    name: name.into(),
                    slot,
                    base,
                });
                targets.len() - 1
            }
        };
        let base = match shape {
            Shape::Transform(kind) => identity(kind).to_owned(),
            Shape::Plain => targets[target].base.clone().unwrap_or_else(|| "0".into()),
        };

        let keyframes: Vec<String> = if kind == "set" {
            match attr("to") {
                Some(to) => vec![to.into()],
                None => continue,
            }
        } else if let Some(values) = attr("values") {
            values
                .split(';')
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(Into::into)
                .collect()
        } else {
            match (attr("from"), attr("to"), attr("by")) {
                (Some(from), Some(to), _) => vec![from.into(), to.into()],
                (None, Some(to), _) => vec![base, to.into()],
                (from, None, Some(by)) => {
                    let from = from.map_or(base, Into::into);
                    let to = add(&from, by);
                    vec![from, to]
                }
                _ => continue,
            }
        };
        if keyframes.is_empty() {
            continue;
        }
        let key_times = attr("keyTimes").and_then(|times| {
            let times: Vec<f64> = times
                .split(';')
                .map(|t| t.trim().parse::<f64>().ok())
                .collect::<Option<_>>()?;
            let ordered = times.windows(2).all(|w| w[0] <= w[1]);
            (times.len() == keyframes.len() && ordered && times.first() == Some(&0.0))
                .then_some(times)
        });

        tracks.push(Track {
            target,
            keyframes,
            key_times,
            discrete: kind == "set" || attr("calcMode") == Some("discrete"),
            begin,
            dur,
            repeat,
            freeze: attr("fill") == Some("freeze"),
            shape,
            sum: kind == "animateTransform" && attr("additive") == Some("sum"),
        });
    }
    Smil { targets, tracks }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discrete_steps_follow_key_times() {
        let xml = r#"<svg xmlns="http://www.w3.org/2000/svg"><rect width="1" height="1">
            <animate attributeName="width" values="1;5;9" keyTimes="0;0.1;0.9" calcMode="discrete" dur="1s"/>
          </rect></svg>"#;
        let doc = Document::parse(xml).unwrap();
        let smil = collect(xml, &doc);
        assert_eq!(smil.tracks[0].at(0.05), "1");
        assert_eq!(smil.tracks[0].at(0.5), "5");
        assert_eq!(smil.tracks[0].at(0.95), "9");
    }

    #[test]
    fn values_are_escaped_into_the_text() {
        let xml = r#"<svg xmlns="http://www.w3.org/2000/svg"><text>a<set attributeName="font-family" to="x&quot;/&gt;&lt;script"/></text></svg>"#;
        let doc = Document::parse(xml).unwrap();
        let smil = collect(xml, &doc);
        let out = super::super::apply(xml, smil.edits(0.0));
        assert!(out.contains(r#"font-family="x&quot;/>&lt;script""#));
        assert!(Document::parse(&out).is_ok());
    }
}
