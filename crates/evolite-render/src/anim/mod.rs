//! Animated SVG, drawn as frames.
//!
//! resvg draws one moment of a picture. For each frame the animations of the file ([`smil`]
//! elements and CSS `@keyframes` ([`css`])) say what their attributes or styles are at that
//! moment; those values are written into the SVG's own text, and resvg draws the result
//! through the same closed-off path as a still ([`crate::svg`]).

mod css;
mod smil;
mod values;

use std::ops::Range;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use roxmltree::{Document, Node};

use crate::frames::{Art, Frame, Limits, RenderError, fit};
use crate::svg;

/// How long the last frame of an animation that plays once stays before it starts again.
const HOLD_LAST_MS: u32 = 2_000;

/// Threads that draw the frames of one animation ([`Limits::threads`]), never more than the
/// machine has cores.
fn threads(limits: &Limits) -> usize {
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    limits.threads.clamp(1, cores)
}

/// Where a value goes in the SVG's text.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Slot {
    /// The value of an attribute the element has.
    Replace(Range<usize>),
    /// Right after the element's name: the element does not have the attribute.
    Insert(usize),
}

impl Slot {
    fn position(&self) -> usize {
        match self {
            Slot::Replace(range) => range.start,
            Slot::Insert(at) => *at,
        }
    }

    /// The slot of attribute `name` of `element`: its value, or a place to add it.
    pub(super) fn of(text: &str, element: Node, name: &str) -> (Slot, Option<String>) {
        match element
            .attributes()
            .find(|a| a.name() == name && a.namespace().is_none())
        {
            Some(a) => (Slot::Replace(a.range_value()), Some(a.value().to_owned())),
            None => (Slot::Insert(after_name(text, element)), None),
        }
    }
}

/// One attribute set to one value in one frame.
pub(super) struct Edit<'a> {
    pub slot: &'a Slot,
    pub name: &'a str,
    pub value: String,
}

/// When one animation runs, for the timeline of the whole picture.
pub(super) struct Timing {
    pub begin: f64,
    /// One cycle, seconds (twice the duration when it alternates).
    pub period: f64,
    /// Repeats for ever: the picture loops.
    pub loops: bool,
    /// The last moment it changes anything (infinite for a loop).
    pub last_change: f64,
}

/// Where `element` starts its attributes: right after `<name`.
fn after_name(text: &str, element: Node) -> usize {
    let start = element.range().start + 1;
    start
        + text[start..]
            .find(|c: char| c.is_whitespace() || c == '/' || c == '>')
            .unwrap_or(0)
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// `text` with `edits` written in, from the end back so earlier positions stay right. Several
/// attributes may be added at one place (one element); each carries its own name.
fn apply(text: &str, mut edits: Vec<Edit>) -> String {
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.slot.position()));
    let mut out = text.to_owned();
    for Edit { slot, name, value } in edits {
        match slot {
            Slot::Replace(range) => out.replace_range(range.clone(), &escape(&value)),
            Slot::Insert(at) => out.insert_str(*at, &format!(" {name}=\"{}\"", escape(&value))),
        }
    }
    out
}

/// The least common multiple of loop lengths in milliseconds, or `cap` once it passes it.
fn common_loop(durations_ms: impl Iterator<Item = u64>, cap: u64) -> u64 {
    fn gcd(a: u64, b: u64) -> u64 {
        if b == 0 { a } else { gcd(b, a % b) }
    }
    let mut lcm = 1u64;
    for d in durations_ms.filter(|d| *d > 0) {
        lcm = lcm / gcd(lcm, d) * d;
        if lcm >= cap {
            return cap;
        }
    }
    lcm
}

/// How long the frames cover, ms, and whether they loop.
fn span(timings: &[Timing], cap_ms: u64) -> (u64, bool) {
    let ms = |s: f64| (s * 1000.0).round().max(0.0) as u64;
    let looping = timings.iter().any(|t| t.loops);
    if !looping {
        let end = timings.iter().map(|t| ms(t.last_change)).max().unwrap_or(0);
        return (end.min(cap_ms), false);
    }
    // Whole periods of every repeating animation, so their motion closes without a jump, and
    // enough of them to see the ones that start late. A one-off change (a `<set>` at 1 s)
    // plays again on every loop: frames have no "intro, then loop".
    let lcm = common_loop(
        timings
            .iter()
            .filter(|t| t.loops)
            .map(|t| ms(t.period) / 10 * 10),
        cap_ms,
    )
    .max(1);
    let latest_begin = timings.iter().map(|t| ms(t.begin)).max().unwrap_or(0);
    let periods = (latest_begin / lcm + 1).max(1);
    let whole = lcm.saturating_mul(periods);
    // Past the cap: as many whole periods as fit, or the cap if not even one does.
    let span = if whole <= cap_ms {
        whole
    } else if lcm <= cap_ms {
        cap_ms / lcm * lcm
    } else {
        cap_ms
    };
    (span, true)
}

/// The frames of an animated SVG, or `None` when there is nothing to animate (then it is a
/// still). Every frame is drawn through [`svg::parse`] and [`svg::tree`], like a still.
pub(crate) fn frames(
    text: &str,
    xml: &Document,
    limits: &Limits,
) -> Result<Option<Art>, RenderError> {
    let smil = smil::collect(text, xml);
    let css = css::collect(text, xml, limits);
    let count_all = smil.len() + css.len();
    if count_all == 0 || count_all > limits.max_animations {
        return Ok(None);
    }
    let timings: Vec<Timing> = smil.timings().into_iter().chain(css.timings()).collect();
    let (span_ms, looping) = span(&timings, limits.max_animation_ms.max(1) as u64);
    if span_ms == 0 {
        return Ok(None);
    }
    let text_at = |t: f64| {
        apply(
            text,
            smil.edits(t).into_iter().chain(css.edits(t)).collect(),
        )
    };

    // Frames lie on a grid of whole `frame_ms` steps from the start of the loop: the app shows
    // every animation on one common clock of that step, and none of them drops or doubles a
    // frame on it. The end state of a one-off animation is a frame of its own.
    let frame_ms = (1000 / limits.animation_fps.max(1)).max(1) as u64;
    let grid = |step: u64| -> Vec<u64> {
        let mut times: Vec<u64> = (0..span_ms.div_ceil(step)).map(|i| i * step).collect();
        if !looping {
            times.push(span_ms);
        }
        times
    };
    let max_frames = limits.max_frames.max(2) as u64;
    let points = span_ms.div_ceil(frame_ms) + u64::from(!looping);
    let mut step = frame_ms * points.div_ceil(max_frames).max(1);
    let count = grid(step).len();

    // The first frame fixes the canvas: its size and the drawing's own size. Its time sets how
    // many frames fit the budget, so the one-off finding of fonts is done before the clock.
    svg::warm_up();
    let started = Instant::now();
    let first_text = text_at(0.0);
    let first_xml = svg::parse(&first_text, limits)?;
    let first = svg::tree(&first_xml, limits)?;
    let base = first.size();
    let side = limits.max_animation_side.min(limits.max_side);
    let (mut width, mut height) = svg::canvas(base, side, limits.svg_size);
    while width as usize * height as usize * 4 * count > limits.max_total && width > 1 && height > 1
    {
        (width, height) = fit(width, height, width.max(height) * 3 / 4);
    }
    let first_rgba = svg::raster(&first, base, width, height)?;

    // The still pass: only the first frame, so the card shows the picture at once while the
    // animation is drawn. It keeps a delay: one frame with a delay is a picture to come.
    if limits.first_frame_only {
        return Ok(Some(Art {
            width,
            height,
            frames: vec![Frame {
                rgba: first_rgba,
                delay_ms: frame_ms as u32,
            }],
        }));
    }

    // Only the moments something changes are drawn: a frame whose values are those of the
    // frame before (CSS `steps()`, discrete SMIL, `<set>`, pauses) is that frame shown longer.
    let plan = |step: u64| {
        let times = grid(step);
        let mut changed = vec![true; times.len()];
        let mut before = text_at(0.0);
        for (i, changed) in changed.iter_mut().enumerate().skip(1) {
            let now = text_at(times[i] as f64 / 1000.0);
            *changed = now != before;
            before = now;
        }
        (times, changed)
    };
    let (mut times, mut changed) = plan(step);
    // Fewer frames when drawing is slow: what changes must fit the time budget, drawn on
    // several threads at once (frames do not depend on each other). The step stays a whole
    // number of grid steps.
    let threads = threads(limits);
    let per_frame = started.elapsed().as_millis().max(1) as usize;
    let budget = limits.max_render_ms.max(1) as usize;
    let to_draw = changed.iter().filter(|c| **c).count();
    if to_draw * per_frame > budget * threads {
        step *= (to_draw * per_frame).div_ceil(budget * threads) as u64;
        (times, changed) = plan(step);
    }
    let count = times.len();
    // The whole loop: a one-off animation holds its end state a while before it starts again.
    let end = if looping {
        span_ms
    } else {
        times[count - 1] + u64::from(HOLD_LAST_MS)
    };
    let delay = |i: usize| (times.get(i + 1).copied().unwrap_or(end) - times[i]) as u32;
    let todo: Vec<usize> = (1..count).filter(|i| changed[*i]).collect();

    // Frame `i`: `None` when not drawn in time, `Some(None)` when it repeats the frame before
    // (unchanged, or values usvg refused).
    let mut drawn_frames: Vec<Option<Option<Vec<u8>>>> =
        changed.iter().map(|c| (!c).then_some(None)).collect();
    drawn_frames[0] = Some(Some(first_rgba));
    let next = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut mine = Vec::new();
                    loop {
                        // The client waits a few seconds more than the budget, never longer.
                        if started.elapsed().as_millis() as usize > budget {
                            break;
                        }
                        let Some(&i) = todo.get(next.fetch_add(1, Ordering::Relaxed)) else {
                            break;
                        };
                        let frame_text = text_at(times[i] as f64 / 1000.0);
                        let rgba = svg::parse(&frame_text, limits)
                            .and_then(|xml| svg::tree(&xml, limits))
                            .and_then(|tree| svg::raster(&tree, base, width, height))
                            .ok();
                        mine.push((i, rgba));
                    }
                    mine
                })
            })
            .collect();
        for worker in workers {
            // A panic in a thread is the request's panic: the worker refuses the picture.
            for (i, rgba) in worker
                .join()
                .unwrap_or_else(|e| std::panic::resume_unwind(e))
            {
                drawn_frames[i] = Some(rgba);
            }
        }
    });

    let mut frames: Vec<Frame> = Vec::new();
    let mut drawn = 0;
    // Frames up to the first one not drawn in time; a later one drawn by a faster thread
    // would leave a hole.
    for (i, rgba) in drawn_frames.into_iter().map_while(|f| f).enumerate() {
        drawn += 1;
        // The same picture as before (a pause, a refused value) stays on screen longer instead
        // of taking the memory of one more frame, here and in the app.
        match (rgba, frames.last_mut()) {
            (Some(rgba), Some(last)) if rgba == last.rgba => last.delay_ms += delay(i),
            (Some(rgba), _) => frames.push(Frame {
                rgba,
                delay_ms: delay(i),
            }),
            (None, Some(last)) => last.delay_ms += delay(i),
            (None, None) => unreachable!("the first frame is drawn"),
        }
    }
    // Cut short by the budget: the last frame drawn stays to the end of the loop, so the
    // animation keeps its pace and only gets coarser at the end.
    if drawn < count
        && let Some(last) = frames.last_mut()
    {
        last.delay_ms += (end - times[drawn]) as u32;
    }
    Ok(Some(Art {
        width,
        height,
        frames,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn limits() -> Limits {
        Limits::default()
    }

    pub(super) fn render(svg: &str) -> Art {
        crate::render(svg.as_bytes(), &limits()).unwrap()
    }

    pub(super) fn pixel(art: &Art, frame: usize, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * art.width + x) * 4) as usize;
        art.frames[frame].rgba[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn a_moving_square_moves() {
        let art = render(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="10">
                 <rect x="0" width="10" height="10" fill="red">
                   <animate attributeName="x" from="0" to="90" dur="1s" repeatCount="indefinite"/>
                 </rect>
               </svg>"#,
        );
        assert_eq!(art.frames.len(), 20);
        assert!(art.frames.iter().all(|f| f.delay_ms == 50));
        assert_eq!(pixel(&art, 0, 5, 5), [255, 0, 0, 255]);
        // At 0.5 s the square is at x = 45.
        assert_eq!(pixel(&art, 10, 5, 5)[3], 0);
        assert_eq!(pixel(&art, 10, 50, 5), [255, 0, 0, 255]);
    }

    #[test]
    fn a_colour_changes() {
        let art = render(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
                 <rect width="10" height="10" fill="red">
                   <animate attributeName="fill" values="red;blue" dur="2s" repeatCount="indefinite"/>
                 </rect>
               </svg>"#,
        );
        assert_eq!(art.frames.len(), 40);
        assert_eq!(pixel(&art, 0, 5, 5), [255, 0, 0, 255]);
        let [r, g, b, a] = pixel(&art, 20, 5, 5);
        assert!((120..=135).contains(&r) && g == 0 && (120..=135).contains(&b) && a == 255);
    }

    #[test]
    fn a_transform_turns() {
        // A bar from the centre to the right edge, turned by 90°: then it points down.
        let art = render(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20">
                 <rect x="10" y="9" width="10" height="2" fill="black">
                   <animateTransform attributeName="transform" type="rotate"
                     from="0 10 10" to="360 10 10" dur="4s" repeatCount="indefinite"/>
                 </rect>
               </svg>"#,
        );
        assert_eq!(art.frames.len(), 80);
        assert_eq!(pixel(&art, 0, 17, 10)[3], 255);
        assert_eq!(pixel(&art, 0, 10, 17)[3], 0);
        // 1 s = a quarter turn = frame 20.
        assert_eq!(pixel(&art, 20, 17, 10)[3], 0);
        assert_eq!(pixel(&art, 20, 10, 17)[3], 255);
    }

    #[test]
    fn set_shows_at_its_time_and_the_end_is_held() {
        let art = render(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
                 <rect width="10" height="10" fill="green" visibility="hidden">
                   <set attributeName="visibility" to="visible" begin="0.5s"/>
                 </rect>
               </svg>"#,
        );
        assert_eq!(pixel(&art, 0, 5, 5)[3], 0);
        let last = art.frames.len() - 1;
        assert_eq!(pixel(&art, last, 5, 5), [0, 128, 0, 255]);
        assert_eq!(art.frames[last].delay_ms, HOLD_LAST_MS);
    }

    #[test]
    fn to_starts_from_the_elements_own_value() {
        let art = render(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="10">
                 <rect x="80" width="10" height="10" fill="red">
                   <animate attributeName="x" to="0" dur="1s" fill="freeze"/>
                 </rect>
               </svg>"#,
        );
        assert_eq!(pixel(&art, 0, 85, 5), [255, 0, 0, 255]);
        let last = art.frames.len() - 1;
        assert_eq!(pixel(&art, last, 5, 5), [255, 0, 0, 255]);
    }

    #[test]
    fn an_event_start_is_drawn_still() {
        let art = render(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
                 <rect width="10" height="10" fill="red">
                   <animate attributeName="fill" to="blue" begin="click" dur="1s"/>
                 </rect>
               </svg>"#,
        );
        assert_eq!(art.frames.len(), 1);
        assert_eq!(pixel(&art, 0, 5, 5), [255, 0, 0, 255]);
    }

    #[test]
    fn too_many_animations_are_drawn_still() {
        let many =
            r#"<animate attributeName="x" from="0" to="5" dur="1s" repeatCount="indefinite"/>"#
                .repeat(3);
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="5" height="5">{many}</rect></svg>"#
        );
        let few = Limits {
            max_animations: 2,
            ..limits()
        };
        assert_eq!(crate::render(svg.as_bytes(), &few).unwrap().frames.len(), 1);
    }

    #[test]
    fn a_long_loop_is_cut_and_a_slow_one_gets_fewer_frames() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="5" height="5">
            <animate attributeName="x" from="0" to="5" dur="7s" repeatCount="indefinite"/>
            <animate attributeName="y" from="0" to="5" dur="11s" repeatCount="indefinite"/>
          </rect></svg>"#;
        // 7 s and 11 s loop together every 77 s: cut at 10 s, 20 frames a second. On 10 px
        // many steps draw the same pixels; those are one frame shown longer.
        let art = crate::render(svg.as_bytes(), &limits()).unwrap();
        let total: u32 = art.frames.iter().map(|f| f.delay_ms).sum();
        assert_eq!(total, 10_000);
        assert!(art.frames.len() > 20 && art.frames.len() <= 200);
        assert!(art.frames.windows(2).all(|w| w[0].rgba != w[1].rgba));
        let hurried = Limits {
            max_render_ms: 1,
            ..limits()
        };
        // Past the budget at once: the first frame holds the whole loop.
        let art = crate::render(svg.as_bytes(), &hurried).unwrap();
        assert!(!art.frames.is_empty() && art.frames.len() < 200);
        assert_eq!(art.frames.iter().map(|f| f.delay_ms).sum::<u32>(), 10_000);
    }

    #[test]
    fn the_still_pass_gives_the_first_frame_with_a_delay() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="5" height="5"><animate attributeName="x" from="0" to="5" dur="1s" repeatCount="indefinite"/></rect></svg>"#;
        let first = Limits {
            first_frame_only: true,
            ..limits()
        };
        let art = crate::render(svg.as_bytes(), &first).unwrap();
        assert_eq!(art.frames.len(), 1);
        assert!(art.frames[0].delay_ms > 0);
        assert_eq!(art.frames[0].rgba, render(svg).frames[0].rgba);
        // A still stays a still: delay 0, nothing to come.
        let still = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="5" height="5"/></svg>"#;
        assert_eq!(
            crate::render(still.as_bytes(), &first).unwrap().frames[0].delay_ms,
            0
        );
    }

    #[test]
    fn frames_lie_on_the_grid_of_the_app_clock() {
        // A loop of 1.23 s: frames every 50 ms from its start, the last one shorter.
        let art = render(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="10"><rect width="5" height="5"><animate attributeName="x" from="0" to="35" dur="1.23s" repeatCount="indefinite"/></rect></svg>"#,
        );
        let delays: Vec<u32> = art.frames.iter().map(|f| f.delay_ms).collect();
        assert_eq!(delays.iter().sum::<u32>(), 1230);
        let (last, rest) = delays.split_last().unwrap();
        assert!(rest.iter().all(|d| d % 50 == 0), "{delays:?}");
        assert_eq!(last % 50, 30);
    }

    #[test]
    fn only_the_moments_of_change_are_drawn() {
        // steps(2) over 1 s: two pictures, though 20 frames a second are planned.
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><style>rect{animation:m 1s steps(2) infinite}@keyframes m{from{opacity:1}to{opacity:0}}</style><rect width="10" height="10"/></svg>"#;
        let art = render(svg);
        assert_eq!(art.frames.len(), 2);
        assert_eq!(art.frames.iter().map(|f| f.delay_ms).sum::<u32>(), 1000);
    }

    #[test]
    fn a_value_usvg_refuses_keeps_the_picture() {
        let art = render(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
                 <rect width="10" height="10" fill="red">
                   <animate attributeName="width" values="10;nonsense" dur="1s" repeatCount="indefinite"/>
                 </rect>
               </svg>"#,
        );
        // The refused steps repeat the picture before them: fewer frames, the same second.
        assert!(art.frames.len() < 20);
        assert_eq!(art.frames.iter().map(|f| f.delay_ms).sum::<u32>(), 1000);
        assert!(art.frames.iter().all(|f| f.rgba.len() == 400));
    }
}
