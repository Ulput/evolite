//! BUG-123: the word that shows a password («Show», «عرض») stands 14 px inside the field's
//! frame on its side, in Arabic as in English — measured on what is painted, after the mirror.

use super::*;

/// The painted text shapes with `text` (as laid out) and where their ink is, nested ones too.
fn painted(kit: &Kit, text: &str) -> Vec<egui::Rect> {
    fn walk(shape: &egui::Shape, text: &str, out: &mut Vec<egui::Rect>) {
        match shape {
            egui::Shape::Text(t) if t.galley.job.text == text => {
                // The ink: where the glyphs are drawn, not the galley's own rect.
                let mut ink = egui::Rect::NOTHING;
                for row in &t.galley.rows {
                    for g in row.row.glyphs.iter().filter(|g| g.advance_width > 0.0) {
                        let x = t.pos.x + row.pos.x + g.pos.x;
                        let y = t.pos.y + row.pos.y;
                        ink = ink.union(egui::Rect::from_min_size(
                            egui::pos2(x, y),
                            egui::vec2(g.advance_width, row.size.y),
                        ));
                    }
                }
                out.push(ink);
            }
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, text, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for c in &kit.output().shapes {
        walk(&c.shape, text, &mut out);
    }
    out
}

/// The painted frames with a border (a field's outline) that hold `inside`.
fn frame_around(kit: &Kit, inside: egui::Rect) -> Option<egui::Rect> {
    fn walk(shape: &egui::Shape, inside: egui::Rect, out: &mut Vec<egui::Rect>) {
        match shape {
            egui::Shape::Rect(r) if r.stroke.width > 0.0 && r.rect.contains_rect(inside) => {
                out.push(r.rect)
            }
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, inside, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for c in &kit.output().shapes {
        walk(&c.shape, inside, &mut out);
    }
    // The smallest: the field's own frame, not a card around it.
    out.into_iter().min_by(|a, b| a.area().total_cmp(&b.area()))
}

/// The gap between the reveal word's ink and the frame's inner edge on the word's side.
fn reveal_inset(preset: &str, lang: &str) -> f32 {
    let (_turn, mut kit) = demo_app(preset, lang);
    kit.run_steps(3);
    let word = crate::i18n::visual(&crate::i18n::t("Show")).into_owned();
    let words = painted(&kit, &word);
    let word_at = *words
        .first()
        .unwrap_or_else(|| panic!("{preset} {lang}: «{word}» painted"));
    let frame = frame_around(&kit, word_at).expect("the field's frame");
    if crate::i18n::rtl() {
        word_at.left() - frame.left()
    } else {
        frame.right() - word_at.right()
    }
}

/// BUG-123: 14 px from the frame on the word's side (the field's inset), within a point, in
/// English and in Arabic, on the unlock screen and in a form.
#[test]
fn the_reveal_word_keeps_its_inset_in_arabic_too() {
    for preset in ["unlock", "change-password"] {
        for lang in ["en", "ar"] {
            let inset = reveal_inset(preset, lang);
            assert!(
                (13.0..=16.0).contains(&inset),
                "{preset} {lang}: «Show» {inset} px from the frame, not 14"
            );
        }
    }
}
