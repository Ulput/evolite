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
const SUMMARY_DIALOGS: [(&str, bool); 26] = [
    ("accept-contact", true),
    ("add-contact", true),
    ("add-name", true),
    ("app-connect-v2", true),
    ("app-disconnect", true),
    ("contest", true),
    ("create-identity-confirm", true),
    // Its password is the wallet's own, not a spend's: it deletes, it pays nothing.
    ("delete-wallet", false),
    ("edit-profile", true),
    ("fund-address", true),
    ("fund-address-resume", true),
    ("gift-dialog", true),
    ("key-change", true),
    ("resend", true),
    ("send-confirm", true),
    ("shield-dialog", true),
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
            let needle = ["dialog_with_", "summary("].concat();
            for (at, _) in text.match_indices(&needle) {
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
const WORKSHOP: [&str; 11] = [
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
];

/// Painted words cut by their clip (a single line wider than its card or column) or drawn over
/// other words (BUG-258: the identity chips over «Credits @…», a summary's label over its sum).
/// Horizontal clipping only: a scrolled page clips its words at the top and bottom by design.
/// Words in a text field are left out: a long entry scrolls inside its field.
fn text_trouble(kit: &Kit) -> Vec<String> {
    fn walk(
        shape: &egui::Shape,
        clip: egui::Rect,
        out: &mut Vec<(String, egui::Rect, egui::Rect)>,
    ) {
        match shape {
            // One entry a row of words, its glyphs' own extent: a label in a wrapping row is
            // laid out from the row's left edge, with leading space that is not ink.
            egui::Shape::Text(t) if !t.galley.text().trim().is_empty() => {
                let text = t.galley.text().to_owned();
                for row in t.galley.rows.iter().filter(|r| !r.row.glyphs.is_empty()) {
                    let rect = row.rect_without_leading_space().translate(t.pos.to_vec2());
                    out.push((text.clone(), rect, clip));
                }
            }
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, clip, out)),
            _ => {}
        }
    }
    let mut texts = Vec::new();
    for c in &kit.output().shapes {
        walk(&c.shape, c.clip_rect, &mut texts);
    }
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
        .filter(|(_, r, clip)| !in_field(r) && clip.intersects(*r))
        .collect();
    let mut wrong = Vec::new();
    for (text, r, clip) in &texts {
        if r.right() > clip.right() + 1.0 || r.left() < clip.left() - 1.0 {
            wrong.push(format!("cut: «{text}» at {r:?} in {clip:?}"));
        }
    }
    // Words of one region only (the same clip): a dialog's words over the page under it are not
    // a collision.
    for (i, (a, ra, ca)) in texts.iter().enumerate() {
        for (b, rb, cb) in &texts[i + 1..] {
            let both = ra.intersect(*rb).intersect(*ca);
            if ca == cb && both.width() > 2.0 && both.height() > 3.0 && a != b {
                wrong.push(format!("over: «{a}» and «{b}» at {both:?}"));
            }
        }
    }
    wrong.truncate(6);
    wrong
}

/// The medium window: the rail layout, under 1280 wide — its ends and a middle.
const WORKSHOP_WIDTHS: [f32; 3] = [1024.0, 1150.0, 1279.0];

fn workshop_at_1024(lang: &str) {
    let mut wrong = Vec::new();
    for preset in WORKSHOP {
        for width in WORKSHOP_WIDTHS {
            let (_turn, mut kit) = demo_app(preset, lang);
            kit.set_size(egui::vec2(width, 768.0));
            quiet(&mut kit, |_| {});
            for w in text_trouble(&kit).into_iter().chain(too_wide(&kit)) {
                wrong.push(format!("{preset} @{width}: {w}"));
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
    assert!(
        wrong.is_empty(),
        "{lang}:
{}",
        wrong.join(
            "
"
        )
    );
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
