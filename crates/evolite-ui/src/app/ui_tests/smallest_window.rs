//! BUG-249: at the smallest window the app allows (800×440), in English, Russian, Japanese,
//! German and Arabic (whose lines wrap differently), every preset fits the window's width, and
//! an open dialog either keeps a scrollable part of at least [`MIN_MIDDLE`] or has its summary
//! and footer in the window.

use super::*;
use crate::cli::PRESETS;
use crate::components::{MIN_MIDDLE, dialog_seen};

const SMALLEST: [f32; 2] = [800.0, 440.0];

/// Every dialog made with `dialog_with_summary`, by the id written in its code, and whether it
/// spends (a spend's row of buttons or password field). Each must be seen by the presets with its
/// summary, spending where it spends; and the code holds no other
/// (`the_summary_dialogs_are_all_listed`). BUG-249 r3, the architect.
const SUMMARY_DIALOGS: [(&str, bool); 32] = [
    ("accept-contact", true),
    ("add-contact", true),
    ("add-name", true),
    ("app-connect-v2", true),
    ("app-disconnect", true),
    ("contest", true),
    ("create-identity-confirm", true),
    // FEAT-0048: «Create privately», from the pool.
    ("create-identity-pool", true),
    // Its password is the wallet's own, not a spend's: it deletes, it pays nothing.
    ("delete-wallet", false),
    ("edit-profile", true),
    ("fund-address", true),
    ("fund-address-resume", true),
    ("gift-dialog", true),
    ("key-change", true),
    ("resend", true),
    ("send-confirm", true),
    // FEAT-0056: a payment to several.
    ("send-many-confirm", true),
    ("shield-dialog", true),
    // FEAT-0048 UC-4 «Shield again»: a refused identity's money back into the pool.
    ("shield-refund", true),
    ("token-buy", true),
    ("token-claim", true),
    ("token-issuer", true),
    ("token-send", true),
    ("top-up", true),
    ("unshield-dialog", true),
    // FEAT-0038 W3 «Check the collection».
    ("workshop-check-collection", true),
    // FEAT-0038 W5 «Check the model».
    ("workshop-check-model", true),
    // FEAT-0038 W8 «Give» (and «Continue» of a gift) and «Issue more» (and of an issue).
    ("workshop-give", true),
    ("workshop-issue", true),
    // FEAT-0038 W9 «Put up for sale», «Change the price», «Take off sale».
    ("workshop-sell", true),
    ("workshop-reprice", true),
    ("workshop-unprice", true),
];

/// What a dialog shown is, for [`quiet`]: its id, whether it spends, its summary, its scroll.
type Shown = Option<(String, bool, bool, bool)>;

fn shown(kit: &Kit) -> Shown {
    dialog_seen(&kit.ctx).map(|s| (s.name, s.spends, s.summary.is_some(), s.one_scroll))
}

/// Frames, with `edit` before each, until the dialog shown (its id, whether it spends, its
/// summary) and the demo core's snapshot have not changed for a while: what comes after the
/// first frames — a dialog opened, or a spend's row drawn, on the core's answer (≈0.6 s in the
/// demo) — is in by then (the architect, BUG-249 r3: the top-up got past 6 frames).
fn quiet(kit: &mut Kit, edit: impl Fn(&mut App)) {
    const QUIET: std::time::Duration = std::time::Duration::from_millis(300);
    const MOST: std::time::Duration = std::time::Duration::from_secs(5);
    let started = std::time::Instant::now();
    let mut since = std::time::Instant::now();
    let mut last = (shown(kit), kit.state().snapshot.clone());
    while since.elapsed() < QUIET && started.elapsed() < MOST {
        std::thread::sleep(std::time::Duration::from_millis(5));
        let ctx = kit.ctx.clone();
        kit.state_mut().take_events(&ctx);
        edit(kit.state_mut());
        kit.step();
        let now = (shown(kit), kit.state().snapshot.clone());
        if now != last {
            since = std::time::Instant::now();
            last = now;
        }
    }
}

const LANGUAGES: [&str; 5] = ["en", "ru", "ja", "de", "ar"];

/// The accessible nodes that stand out of the window to the left or the right — a page wider
/// than the window. A node out at the top or the bottom is only scrolled away.
fn too_wide(kit: &Kit) -> Vec<String> {
    let window = kit.ctx.content_rect();
    let ppp = kit.ctx.pixels_per_point();
    kit.query_all_by(|n| n.bounding_box().is_some())
        .filter_map(|n| {
            let b = n.accesskit_node().bounding_box()?;
            let r = egui::Rect::from_min_max(
                egui::pos2(b.x0 as f32 / ppp, b.y0 as f32 / ppp),
                egui::pos2(b.x1 as f32 / ppp, b.y1 as f32 / ppp),
            );
            let out = r.width() > 0.5
                && r.height() > 0.5
                && (r.left() < window.left() - 1.0 || r.right() > window.right() + 1.0);
            let node = n.accesskit_node();
            out.then(|| {
                format!(
                    "{:?} {:?} at {r:?}",
                    node.role(),
                    node.label().or(node.value()).unwrap_or_default()
                )
            })
        })
        .take(3)
        .collect()
}

/// What a dialog that is up must keep at the smallest window.
fn dialog_kept(kit: &Kit) -> Option<String> {
    let seen = dialog_seen(&kit.ctx)?;
    let window = kit.ctx.content_rect();
    if let Some(view) = seen.scrolled
        && view.height() < MIN_MIDDLE - 1.0
    {
        let pinned = seen.footer.is_some_and(|f| window.contains_rect(f))
            && seen.summary.is_none_or(|(s, _)| window.contains_rect(s));
        if !pinned {
            return Some(format!(
                "{:?}: {:.0} px scroll and its summary or footer out of the window",
                seen.id,
                view.height()
            ));
        }
    }
    None
}

fn sweep(lang: &str) {
    let mut wrong = Vec::new();
    for preset in PRESETS {
        // The staged sync never goes quiet; it has no page of its own to check.
        if matches!(*preset, "sync" | "sync-nodes") {
            continue;
        }
        let (_turn, mut kit) = demo_app(preset, lang);
        if preset.starts_with("connect") {
            wait_for(&mut kit, "the app's request", |kit| {
                dialog_seen(&kit.ctx).is_some()
            });
        }
        kit.set_size(egui::vec2(SMALLEST[0], SMALLEST[1]));
        quiet(&mut kit, |_| {});
        for w in too_wide(&kit) {
            wrong.push(format!("{preset}: wider than the window: {w}"));
        }
        if let Some(w) = dialog_kept(&kit) {
            wrong.push(format!("{preset}: {w}"));
        }
    }
    assert!(wrong.is_empty(), "{lang}:\n{}", wrong.join("\n"));
}

#[test]
fn every_preset_fits_the_smallest_window_en() {
    sweep(LANGUAGES[0]);
}

#[test]
fn every_preset_fits_the_smallest_window_ru() {
    sweep(LANGUAGES[1]);
}

#[test]
fn every_preset_fits_the_smallest_window_ja() {
    sweep(LANGUAGES[2]);
}

#[test]
fn every_preset_fits_the_smallest_window_de() {
    sweep(LANGUAGES[3]);
}

#[test]
fn every_preset_fits_the_smallest_window_ar() {
    sweep(LANGUAGES[4]);
}

/// BUG-249 r3 (QA: the identity top-up signed on a first press with its total out of sight):
/// every dialog that spends — a spend's row of buttons or its password field drawn in it — has
/// its summary, so its primary never acts on a total not seen. «Password for every send» is on,
/// so that every spend's password field is drawn.
#[test]
fn every_dialog_that_spends_has_its_summary() {
    let mut wrong = Vec::new();
    let mut spending = 0;
    let mut met = std::collections::BTreeMap::<String, bool>::new();
    for preset in PRESETS {
        if matches!(*preset, "sync" | "sync-nodes") {
            continue;
        }
        let (_turn, mut kit) = demo_app(preset, "en");
        if preset.starts_with("connect") {
            wait_for(&mut kit, "the app's request", |kit| {
                dialog_seen(&kit.ctx).is_some()
            });
        }
        kit.set_size(egui::vec2(SMALLEST[0], SMALLEST[1]));
        quiet(&mut kit, |app| {
            if let Some(w) = app.snapshot.as_mut().and_then(|s| s.wallet.as_mut()) {
                w.password_each_send = true;
            }
        });
        let Some(seen) = dialog_seen(&kit.ctx) else {
            continue;
        };
        if seen.spends {
            spending += 1;
            if seen.summary.is_none() {
                wrong.push(format!("{preset} ({}): spends with no summary", seen.name));
            }
        }
        if seen.summary.is_some() {
            // Spending in any preset: a form's first step may not spend yet.
            *met.entry(seen.name.clone()).or_default() |= seen.spends;
        }
    }
    // Every dialog with a summary is seen by a preset, spending where it spends.
    for (name, spends) in SUMMARY_DIALOGS {
        match met.get(name) {
            None => wrong.push(format!("{name}: no preset shows it with its summary")),
            Some(seen) if *seen != spends => wrong.push(format!(
                "{name}: seen {} a spend, listed {}",
                if *seen { "with" } else { "without" },
                if spends { "spending" } else { "not spending" }
            )),
            Some(_) => {}
        }
    }
    assert!(
        spending > 10,
        "only {spending} presets open a dialog that spends"
    );
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// The list above is the code's: every `dialog_with_summary` written in the screens has its id
/// there, and the list holds no id the code has not (BUG-249 r3).
#[test]
fn the_summary_dialogs_are_all_listed() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut found = std::collections::BTreeSet::new();
    let mut stack = vec![src];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            // The summary dialog and its form with a line under the title (`_under`); this
            // comment names neither whole, or the search would read it.
            for (at, needle) in ["summary(", "summary_under("]
                .iter()
                .flat_map(|end| {
                    let needle = ["dialog_with_", end].concat();
                    text.match_indices(&needle)
                        .map(|(at, _)| (at, needle.clone()))
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
            {
                if text[..at].ends_with("fn ") {
                    continue;
                }
                // `(ui, a, "id", …`: the third argument.
                let args: Vec<&str> = text[at + needle.len()..].splitn(4, ',').collect();
                let id = args.get(2).map(|a| a.trim()).unwrap_or_default();
                let literal = id.strip_prefix('"').and_then(|i| i.strip_suffix('"'));
                assert!(
                    literal.is_some(),
                    "{}: an id that is no literal: {id}",
                    path.display()
                );
                found.insert(literal.unwrap().to_owned());
            }
        }
    }
    let listed: std::collections::BTreeSet<String> = SUMMARY_DIALOGS
        .iter()
        .map(|(n, _)| (*n).to_owned())
        .collect();
    assert_eq!(
        found, listed,
        "the code's dialogs with a summary, and the list"
    );
}

/// BUG-255 (1): «Send» at 1024 pt with the «Fee» row (FEAT-0040, a custom rate): the form and
/// «Summary» beside it stay inside the window in every language swept — the rate's field
/// leaves room for its unit.
#[test]
fn send_with_a_custom_fee_fits_at_1024() {
    for lang in LANGUAGES {
        let (_turn, mut kit) = demo_app("send", lang);
        kit.set_size(egui::vec2(1024.0, 768.0));
        kit.state_mut().settings.set("advanced", true);
        kit.state_mut().settings.fee = evolite_api::fee_choice::FeeChoice::Custom(1_000);
        kit.state_mut().send.fee_custom = "1000".into();
        kit.state_mut().send.recipient = "yXdxCB2vAX6wPHeaQqyB6XJT9eSZGJx3qB".into();
        kit.state_mut().send.amount = "0.1".into();
        kit.run_steps(6);
        let unit = crate::i18n::t_in(crate::i18n::Lang::from_code(lang).unwrap(), "duffs per kB");
        // Arabic is painted reshaped (`i18n::visual`): its unit is not found as typed.
        if lang != "ar" {
            assert!(
                painted_has(&kit, &unit),
                "{lang}: the «Fee» row with its unit"
            );
        }
        let wide = too_wide(&kit);
        assert!(wide.is_empty(), "{lang}: wider than the window: {wide:?}");
    }
}

/// BUG-258: the Workshop's presets, at the medium window (the rail, under 1280 wide).
const WORKSHOP: [&str; 15] = [
    "workshop",
    "workshop-new",
    "workshop-review",
    "workshop-steps",
    "workshop-collection",
    "workshop-model",
    "workshop-model-copy",
    "workshop-model-review",
    "workshop-model-page",
    "workshop-give",
    "workshop-issue",
    "workshop-sell",
    "workshop-reprice",
    "workshop-unprice",
    "workshop-sale-done",
];

/// Painted words cut by their clip (a single line wider than its card or column) or drawn over
/// other words (BUG-258: the identity chips over «Credits @…», a summary's label over its sum).
/// Horizontal clipping only: a scrolled page clips its words at the top and bottom by design.
/// Words in a text field are left out: a long entry scrolls inside its field.
/// A text, its row's ink, its clip, its layer (how many fills over the whole window were
/// painted before it: the page's ground, then a dialog's dimming over the page) and its place
/// in the paint order.
type Seen = (String, egui::Rect, egui::Rect, usize, usize);

/// What [`painted`] finds, each with its place in the paint order: the rows of words, the
/// pictures ([`crate::art::picture_mark`]: rect, clip, layer, place) and the filled grounds
/// (rect, place) — a badge's or a tooltip's own ground over a picture.
#[derive(Default)]
struct Painted {
    texts: Vec<Seen>,
    pictures: Vec<(egui::Rect, egui::Rect, usize, usize)>,
    grounds: Vec<(egui::Rect, usize)>,
    order: usize,
    layer: usize,
}

fn painted(kit: &Kit) -> Painted {
    fn walk(shape: &egui::Shape, clip: egui::Rect, screen: egui::Rect, out: &mut Painted) {
        out.order += 1;
        let (layer, order) = (out.layer, out.order);
        match shape {
            // One entry a row of words, its glyphs' own extent: a label in a wrapping row is
            // laid out from the row's left edge, with leading space that is not ink.
            egui::Shape::Text(t) if !t.galley.text().trim().is_empty() => {
                let text = t.galley.text().to_owned();
                for row in t.galley.rows.iter().filter(|r| !r.row.glyphs.is_empty()) {
                    let rect = row.rect_without_leading_space().translate(t.pos.to_vec2());
                    out.texts.push((text.clone(), rect, clip, layer, order));
                }
            }
            egui::Shape::Rect(r) if r.blur_width == crate::art::PICTURE_MARK => {
                out.pictures.push((r.rect, clip, layer, order));
            }
            egui::Shape::Rect(r)
                if r.fill.a() > 0 && r.rect.expand(1.0).contains_rect(screen.shrink(1.0)) =>
            {
                out.layer += 1;
            }
            egui::Shape::Rect(r) if r.fill.a() > 0 => {
                out.grounds.push((r.rect.intersect(clip), order));
            }
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, clip, screen, out)),
            _ => {}
        }
    }
    let screen = kit.ctx.content_rect();
    let mut out = Painted::default();
    for c in &kit.output().shapes {
        walk(&c.shape, c.clip_rect, screen, &mut out);
    }
    out
}

/// Painted words cut by their clip, drawn over other words, or drawn over a picture.
fn text_trouble(kit: &Kit) -> Vec<String> {
    let Painted {
        texts,
        pictures,
        grounds,
        ..
    } = painted(kit);
    let fields: Vec<egui::Rect> = kit
        .query_all_by(|n| n.role() == Role::TextInput || n.role() == Role::MultilineTextInput)
        .filter_map(|n| {
            let b = n.accesskit_node().bounding_box()?;
            let ppp = kit.ctx.pixels_per_point();
            Some(egui::Rect::from_min_max(
                egui::pos2(b.x0 as f32 / ppp, b.y0 as f32 / ppp),
                egui::pos2(b.x1 as f32 / ppp, b.y1 as f32 / ppp),
            ))
        })
        .collect();
    let in_field = |r: &egui::Rect| {
        fields.iter().any(|f| {
            f.expand(2.0).contains_rect(*r)
                || f.intersects(*r) && r.height() < f.height() + 2.0 && f.contains(r.center())
        })
    };
    let texts: Vec<_> = texts
        .into_iter()
        .filter(|(_, r, clip, _, _)| !in_field(r) && clip.intersects(*r))
        .collect();
    let mut wrong = Vec::new();
    for (text, r, clip, _, _) in &texts {
        if r.right() > clip.right() + 1.0 || r.left() < clip.left() - 1.0 {
            wrong.push(format!("cut: «{text}» at {r:?} in {clip:?}"));
        }
    }
    // BUG-277: words of the page over a picture of the same layer — a picture's own words (its
    // stand-in, «loading», «could not show») are clipped to it, and words on a ground of their
    // own painted over the picture (the «×N» badge, a tooltip) are read on that ground.
    for (picture, picture_clip, layer, p_at) in &pictures {
        let shown = picture.intersect(*picture_clip);
        for (text, r, clip, l, t_at) in &texts {
            let both = r.intersect(shown).intersect(*clip);
            let on_own_ground = grounds
                .iter()
                .any(|(g, g_at)| p_at < g_at && g_at < t_at && g.expand(1.0).contains_rect(both));
            if l == layer
                && !on_own_ground
                && !picture.expand(1.0).contains_rect(*clip)
                && both.width() > 2.0
                && both.height() > 3.0
            {
                wrong.push(format!("over a picture: «{text}» at {r:?} on {picture:?}"));
            }
        }
    }
    // Words of one region and one layer only (the same clip, painted over the same fill): a
    // dialog's words over the dimmed page under it are not a collision.
    for (i, (a, ra, ca, la, _)) in texts.iter().enumerate() {
        for (b, rb, cb, lb, _) in &texts[i + 1..] {
            let both = ra.intersect(*rb).intersect(*ca);
            if ca == cb && la == lb && both.width() > 2.0 && both.height() > 3.0 && a != b {
                wrong.push(format!("over: «{a}» and «{b}» at {both:?}"));
            }
        }
    }
    wrong.truncate(6);
    wrong
}

/// The named widgets wholly below `window`: their names and boxes. From AccessKit, not the
/// painted shapes — egui does not paint words outside what is shown.
fn nodes_below(kit: &Kit, window: egui::Rect) -> Vec<(String, egui::Rect)> {
    let ppp = kit.ctx.pixels_per_point();
    kit.query_all_by(|n| n.bounding_box().is_some())
        .filter_map(|n| {
            let node = n.accesskit_node();
            let name = node.label().or_else(|| node.value())?.to_owned();
            let b = node.bounding_box()?;
            let r = egui::Rect::from_min_max(
                egui::pos2(b.x0 as f32 / ppp, b.y0 as f32 / ppp),
                egui::pos2(b.x1 as f32 / ppp, b.y1 as f32 / ppp),
            );
            (!name.trim().is_empty() && r.top() > window.bottom() + 1.0).then_some((name, r))
        })
        .collect()
}

/// BUG-277 (TEAM.md §8d-2): widgets of the page below the window that the wheel does not bring
/// into it — a page that does not scroll. Turns the wheel down over the page's middle, so it
/// leaves the page scrolled to its end. Not with a dialog open: its page is dimmed under it.
fn below_the_edge(kit: &mut Kit) -> Vec<String> {
    let window = kit.ctx.content_rect();
    if dialog_seen(&kit.ctx).is_some() || nodes_below(kit, window).is_empty() {
        return Vec::new();
    }
    // The wheel over the page's middle and near its bottom (a list under a tall head starts
    // below the middle), then over each column that still has something below (a rail or a
    // side list scrolls on its own).
    let mut columns = vec![window.center().x + 120.0];
    for _ in 0..3 {
        for x in std::mem::take(&mut columns) {
            let mut places = Vec::new();
            for y in [window.center().y, window.bottom() - 40.0] {
                let at = egui::pos2(x, y);
                places.push(at);
                // A popup over the page (History's open filters) takes the wheel there: the page
                // is turned too beside it, where a user would turn it.
                if kit.ctx.layer_id_at(at).map(|l| l.order) != Some(egui::Order::Background) {
                    places.extend(uncovered_beside(kit, window, at));
                }
            }
            for at in places {
                for _ in 0..40 {
                    kit.input_mut().events.push(egui::Event::PointerMoved(at));
                    kit.input_mut().events.push(egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, -400.0),
                        modifiers: egui::Modifiers::NONE,
                        phase: egui::TouchPhase::Move,
                    });
                    kit.step();
                }
            }
            quiet(kit, |_| {});
        }
        columns = nodes_below(kit, window)
            .into_iter()
            .map(|(_, r)| {
                r.center()
                    .x
                    .clamp(window.left() + 1.0, window.right() - 1.0)
            })
            .collect();
        columns.dedup_by(|a, b| (*a - *b).abs() < 40.0);
        if columns.is_empty() {
            break;
        }
    }
    // The pointer leaves: no tooltip of where the wheel turned is left over the next look.
    kit.input_mut().events.push(egui::Event::PointerGone);
    quiet(kit, |_| {});
    let mut left: Vec<String> = nodes_below(kit, window)
        .into_iter()
        .map(|(name, r)| format!("below the window, not scrolled to: «{name}» at {r:?}"))
        .collect();
    left.truncate(3);
    left
}

/// The point nearest `at` on its row that no popup covers, if there is one.
fn uncovered_beside(kit: &Kit, window: egui::Rect, at: egui::Pos2) -> Option<egui::Pos2> {
    (1..40)
        .flat_map(|k| [at.x - 20.0 * k as f32, at.x + 20.0 * k as f32])
        .filter(|x| (window.left() + 1.0..window.right() - 1.0).contains(x))
        .map(|x| egui::pos2(x, at.y))
        .find(|p| kit.ctx.layer_id_at(*p).map(|l| l.order) == Some(egui::Order::Background))
}

/// The medium window: the rail layout, under 1280 wide — its ends and a middle.
const WORKSHOP_WIDTHS: [f32; 3] = [1024.0, 1150.0, 1279.0];

/// [`WORKSHOP_WIDTHS`] and (BUG-277)
/// the narrow window and the wide one beside them, 800 × 440 as QA saw it.
const WORKSHOP_SIZES: [(f32, f32); 5] = [
    (800.0, 440.0),
    (1024.0, 768.0),
    (1150.0, 768.0),
    (1279.0, 768.0),
    (1280.0, 800.0),
];

fn workshop_at_1024(lang: &str) {
    let mut wrong = Vec::new();
    for preset in WORKSHOP {
        for (width, height) in WORKSHOP_SIZES {
            let (_turn, mut kit) = demo_app(preset, lang);
            kit.set_size(egui::vec2(width, height));
            quiet(&mut kit, |_| {});
            let found = text_trouble(&kit)
                .into_iter()
                .chain(too_wide(&kit))
                .chain(below_the_edge(&mut kit));
            for w in found {
                wrong.push(format!("{preset} @{width}x{height}: {w}"));
            }
        }
    }
    assert!(wrong.is_empty(), "{lang}:\n{}", wrong.join("\n"));
}

/// BUG-258 and the customer's «on the medium screen the text is cut, not wrapped»: no Workshop
/// screen at 1024 × 768 cuts its words or draws them over each other.
#[test]
fn the_workshop_wraps_at_1024_en() {
    workshop_at_1024("en");
}

#[test]
fn the_workshop_wraps_at_1024_ru() {
    workshop_at_1024("ru");
}

#[test]
fn the_workshop_wraps_at_1024_de() {
    workshop_at_1024("de");
}

#[test]
fn the_workshop_wraps_at_1024_uk() {
    workshop_at_1024("uk");
}

#[test]
fn the_workshop_wraps_at_1024_ar() {
    workshop_at_1024("ar");
}

/// BUG-277: the model's page scrolls to its end — «Recent» under an 800 × 440 window comes into
/// it with the wheel, the picture and the words beside it apart.
#[test]
fn the_model_page_scrolls_to_recent() {
    let (_turn, mut kit) = demo_app("workshop-model-page", "en");
    kit.set_size(egui::vec2(800.0, 440.0));
    quiet(&mut kit, |_| {});
    let window = kit.ctx.content_rect();
    let recent = |kit: &Kit| {
        kit.query_all_by(|n| {
            n.label()
                .or_else(|| n.value())
                .is_some_and(|l| l.eq_ignore_ascii_case("recent"))
        })
        .find_map(|n| n.accesskit_node().bounding_box())
        .map(|b| b.y0 as f32 / kit.ctx.pixels_per_point())
    };
    let at_first = recent(&kit).expect("«Recent» laid out");
    assert!(
        at_first > window.bottom(),
        "under the window first: {at_first}"
    );
    assert!(below_the_edge(&mut kit).is_empty(), "everything reached");
    let scrolled = recent(&kit).expect("«Recent» laid out");
    assert!(scrolled < window.bottom(), "in the window: {scrolled}");
}

/// The window sizes of the class sweep (TEAM.md §8d-2): the narrowest supported, the medium
/// rail layout, the usual laptop and a full-HD monitor.
const SWEEP_SIZES: [(f32, f32); 4] = [
    (800.0, 600.0),
    (1024.0, 768.0),
    (1280.0, 800.0),
    (1920.0, 1080.0),
];

/// TEAM.md §8d-2 (the class of BUG-258): every preset at every sweep size, words cut by their
/// region or drawn over others — one line each.
fn every_preset_text(lang: &str) -> Vec<String> {
    let mut wrong = Vec::new();
    for preset in PRESETS {
        // The sync presets keep moving: their words change between two frames.
        if matches!(*preset, "sync" | "sync-nodes") {
            continue;
        }
        let (_turn, mut kit) = demo_app(preset, lang);
        if preset.starts_with("connect") {
            wait_for(&mut kit, "the app's request", |kit| {
                dialog_seen(&kit.ctx).is_some()
            });
        }
        for (w, h) in SWEEP_SIZES {
            kit.set_size(egui::vec2(w, h));
            quiet(&mut kit, |_| {});
            for t in text_trouble(&kit)
                .into_iter()
                .chain(below_the_edge(&mut kit))
            {
                wrong.push(format!("{preset} @{w}x{h}: {t}"));
            }
        }
    }
    wrong
}

fn no_text_trouble(lang: &str) {
    let wrong = every_preset_text(lang);
    assert!(wrong.is_empty(), "{lang}:\n{}", wrong.join("\n"));
}

/// BUG-270 (uk at 1024: «Налаштуванн / я»): painted words broken across two lines — a row that
/// ends with a letter and the next that starts with one, where the text has no space or hyphen
/// between them. Monospace text (addresses, ids) breaks anywhere by design and is left out, as
/// are words in a text field.
fn broken_words(kit: &Kit) -> Vec<String> {
    fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(t) if t.galley.rows.len() > 1 => {
                let mono = t.galley.job.sections.iter().any(|s| {
                    s.format.font_id.family == egui::FontFamily::Monospace
                        || matches!(&s.format.font_id.family, egui::FontFamily::Name(n) if n.contains("mono"))
                });
                if mono {
                    return;
                }
                let chars: Vec<char> = t.galley.text().chars().collect();
                let mut at = 0usize;
                for row in &t.galley.rows[..t.galley.rows.len() - 1] {
                    at += row.char_count_including_newline().0;
                    if row.ends_with_newline || at == 0 || at >= chars.len() {
                        continue;
                    }
                    let (before, after) = (chars[at - 1], chars[at]);
                    if before.is_alphabetic() && after.is_alphabetic() {
                        let start = chars[..at]
                            .iter()
                            .rposition(|c| !c.is_alphanumeric())
                            .map_or(0, |i| i + 1);
                        let end = chars[at..]
                            .iter()
                            .position(|c| !c.is_alphanumeric())
                            .map_or(chars.len(), |i| at + i);
                        let word: String = chars[start..end].iter().collect();
                        // A long run with digits is an id or a hash, not a word.
                        if !word.chars().any(|c| c.is_ascii_digit()) {
                            out.push(format!(
                                "«{}» broken as «{}» / «{}»",
                                word,
                                chars[start..at].iter().collect::<String>(),
                                chars[at..end].iter().collect::<String>()
                            ));
                        }
                    }
                }
            }
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for c in &kit.output().shapes {
        walk(&c.shape, &mut out);
    }
    out.sort();
    out.dedup();
    out
}

/// BUG-270, the whole class (TEAM.md §8d, 2): no screen a preset opens, at 1024 × 768, breaks a
/// word across two lines — a title too wide for its column is drawn smaller, a sentence wraps
/// between its words.
fn no_word_broken_at_1024(lang: &str) {
    let mut wrong = Vec::new();
    for preset in PRESETS {
        if matches!(*preset, "sync" | "sync-nodes") {
            continue;
        }
        let (_turn, mut kit) = demo_app(preset, lang);
        if preset.starts_with("connect") {
            wait_for(&mut kit, "the app's request", |kit| {
                dialog_seen(&kit.ctx).is_some()
            });
        }
        kit.set_size(egui::vec2(1024.0, 768.0));
        quiet(&mut kit, |_| {});
        for w in broken_words(&kit) {
            wrong.push(format!("{preset}: {w}"));
        }
    }
    assert!(wrong.is_empty(), "{lang}:\n{}", wrong.join("\n"));
}

#[test]
fn no_word_is_broken_across_lines_at_1024_uk() {
    no_word_broken_at_1024("uk");
}

#[test]
fn no_word_is_broken_across_lines_at_1024_ru() {
    no_word_broken_at_1024("ru");
}

#[test]
fn no_word_is_broken_across_lines_at_1024_de() {
    no_word_broken_at_1024("de");
}

/// BUG-265 (TEAM.md §8d, a hard preset): the 10 long-named identities on every screen that shows
/// them — Identity, Tokens, Collection, the Workshop — at every width: no words cut or drawn over
/// others (the chips over «Credits @…», QA 14), no page wider than the window.
const LONG_IDENTITIES: [&str; 4] = crate::app::dev::LONG_IDENTITIES;

fn long_identities_at_every_width(lang: &str) {
    let mut wrong = Vec::new();
    for preset in LONG_IDENTITIES {
        for size in super::identity_picker::SIZES {
            let (_turn, mut kit) = demo_app(preset, lang);
            kit.set_size(size);
            if preset != "collection-identities-long" {
                wait_for(&mut kit, "10 identities", super::identity_picker::all_pills);
            }
            quiet(&mut kit, |_| {});
            for w in text_trouble(&kit).into_iter().chain(too_wide(&kit)) {
                wrong.push(format!("{preset} @{size:?}: {w}"));
            }
        }
    }
    assert!(wrong.is_empty(), "{lang}:\n{}", wrong.join("\n"));
}

/// TEAM.md §8d-2: no preset, at 800, 1024, 1280 or 1920 wide, cuts its words or draws them
/// over each other — the whole class of BUG-255/258, one test a language. ≈ 6.5 min each:
/// ignored, so the fast step and LIGHT skip them openly, and run in the full check's step t
/// (`-- --ignored no_preset_cuts_or_overlaps_words`; architect, 2026-11-22).
#[test]
#[ignore = "class sweep, ~32 min: full check step t"]
fn no_preset_cuts_or_overlaps_words_en() {
    no_text_trouble("en");
}

#[test]
#[ignore = "class sweep, ~32 min: full check step t"]
fn no_preset_cuts_or_overlaps_words_ru() {
    no_text_trouble("ru");
}

#[test]
#[ignore = "class sweep, ~32 min: full check step t"]
fn no_preset_cuts_or_overlaps_words_de() {
    no_text_trouble("de");
}

#[test]
#[ignore = "class sweep, ~32 min: full check step t"]
fn no_preset_cuts_or_overlaps_words_uk() {
    no_text_trouble("uk");
}

#[test]
#[ignore = "class sweep, ~32 min: full check step t"]
fn no_preset_cuts_or_overlaps_words_ar() {
    no_text_trouble("ar");
}

#[test]
fn long_identities_fit_at_every_width_en() {
    long_identities_at_every_width("en");
}

#[test]
fn long_identities_fit_at_every_width_ru() {
    long_identities_at_every_width("ru");
}

#[test]
fn long_identities_fit_at_every_width_de() {
    long_identities_at_every_width("de");
}

#[test]
fn long_identities_fit_at_every_width_uk() {
    long_identities_at_every_width("uk");
}

#[test]
fn long_identities_fit_at_every_width_ar() {
    long_identities_at_every_width("ar");
}

/// FEAT-0054 r1 Must 3: a group tile whose picture waits for «Show» (an issuer not trusted, a
/// source not chosen) keeps «Picture from …» and «Show» under its «×N», never drawn over it — at
/// the medium widths, in every language of the check.
fn untrusted_groups_at_medium_widths(lang: &str) {
    let mut wrong = Vec::new();
    for width in WORKSHOP_WIDTHS.into_iter().chain([1280.0]) {
        let (_turn, mut kit) = demo_app("collection", lang);
        kit.set_size(egui::vec2(width, 800.0));
        quiet(&mut kit, |app| {
            if let Some(w) = app.snapshot.as_mut().and_then(|s| s.wallet.as_mut()) {
                for nft in &mut w.nfts {
                    nft.issuer = "stranger".into();
                    nft.chosen = false;
                }
            }
        });
        assert!(
            kit.query_all_by_label(&crate::i18n::spoken("Show"))
                .next()
                .is_some(),
            "{lang} @{width}: the pictures wait for «Show»"
        );
        for w in text_trouble(&kit).into_iter().chain(too_wide(&kit)) {
            wrong.push(format!("@{width}: {w}"));
        }
    }
    assert!(wrong.is_empty(), "{lang}:\n{}", wrong.join("\n"));
}

#[test]
fn untrusted_group_tiles_keep_their_words_clear_en() {
    untrusted_groups_at_medium_widths("en");
}

#[test]
fn untrusted_group_tiles_keep_their_words_clear_ru() {
    untrusted_groups_at_medium_widths("ru");
}

#[test]
fn untrusted_group_tiles_keep_their_words_clear_de() {
    untrusted_groups_at_medium_widths("de");
}

#[test]
fn untrusted_group_tiles_keep_their_words_clear_uk() {
    untrusted_groups_at_medium_widths("uk");
}

#[test]
fn untrusted_group_tiles_keep_their_words_clear_ar() {
    untrusted_groups_at_medium_widths("ar");
}
