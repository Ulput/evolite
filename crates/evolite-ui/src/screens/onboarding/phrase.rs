//! O1 · Recovery phrase and O2 · Verification: the phrase from the core, shown once, then three of its words asked back.

use super::txt;
use super::{Onboarding, Step, actions, heading, steps};
use crate::clipboard;
use crate::components::{Assets, card, checkbox, inner, segmented, text_action, warning};
use crate::theme::{Palette, Weight, mono};
use egui::{
    Align, Align2, CornerRadius, CursorIcon, Layout, Margin, Sense, Stroke, StrokeKind, Ui, pos2,
    vec2,
};

/// Word positions asked on O2 in screenshots, from 1: the picture is the same every time.
const DEMO_ASKED: [usize; 3] = [3, 7, 11];

/// What O2 asks: word positions (from 1), each with its four choices.
pub(super) type Checks = Vec<(usize, Vec<String>)>;

/// The screenshots' check, as in Figma: fixed positions, the right word second, first, third. The
/// other three choices are words from the same phrase, so they look just as plausible.
fn demo_checks(words: &[String]) -> Checks {
    DEMO_ASKED
        .iter()
        .enumerate()
        .map(|(q, &n)| {
            let len = words.len();
            let mut choices: Vec<String> = [2, 5, 8]
                .iter()
                .map(|k| words[(n - 1 + k) % len].clone())
                .collect();
            choices.insert([1, 0, 2][q], words[n - 1].clone());
            (n, choices)
        })
        .collect()
}

/// Small numbers nobody can guess in advance, from the standard library's per-process random
/// keys. Nothing secret depends on them: they only keep the check from being passed by heart.
struct Dice(u64);

impl Dice {
    fn new() -> Self {
        use std::hash::{BuildHasher, Hasher};
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos()),
        );
        Self(h.finish() | 1)
    }

    /// 0..n (xorshift64*).
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 32) as usize % n.max(1)
    }
}

/// Three positions of the phrase at random, in order, each with three other words of the
/// phrase and the right one in a random place among them.
fn random_checks(words: &[String], dice: &mut Dice) -> Checks {
    let len = words.len();
    let mut asked: Vec<usize> = Vec::new();
    while asked.len() < 3.min(len) {
        let n = dice.below(len) + 1;
        if !asked.contains(&n) {
            asked.push(n);
        }
    }
    asked.sort_unstable();
    asked
        .into_iter()
        .map(|n| {
            let right = &words[n - 1];
            // A phrase may repeat a word: the choices are distinct and only one is right.
            let mut choices: Vec<String> = Vec::new();
            for _ in 0..200 {
                if choices.len() == 3 {
                    break;
                }
                let word = &words[dice.below(len)];
                if word != right && !choices.contains(word) {
                    choices.push(word.clone());
                }
            }
            choices.insert(dice.below(choices.len() + 1), right.clone());
            (n, choices)
        })
        .collect()
}

/// O1 · Recovery phrase.
pub(super) fn phrase(ui: &mut Ui, a: &mut Assets, o: &mut Onboarding, demo: bool) {
    let p = a.p;
    steps(ui, &p, 1);
    heading(
        ui,
        &p,
        tr!("Recovery phrase"),
        tr!(
            "These words are your wallet. With them it can be restored on any device; without them, never. evolite keeps them nowhere but on this computer, encrypted."
        ),
    );
    #[cfg(demo_core)]
    if demo {
        crate::components::demo_line(
            ui,
            a,
            tr!(
                "An example: the real phrase is made by the wallet core. Do not write these words down."
            ),
        );
    }
    #[cfg(not(demo_core))]
    let _ = demo;
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), 36.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.label(txt(tr!("Phrase length"), 13.0, Weight::Medium, p.muted));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let mut long = usize::from(o.long_phrase);
                segmented(
                    ui,
                    &p,
                    &[tr!("12 words"), tr!("24 words")],
                    &mut long,
                    false,
                );
                o.long_phrase = long == 1;
            });
        },
    );
    let words: Vec<&str> = o
        .phrase
        .as_ref()
        .map(|ph| ph.words().iter().map(String::as_str).collect())
        .unwrap_or_default();
    if words.is_empty() {
        ui.label(txt(
            tr!("Preparing the phrase…"),
            14.0,
            Weight::Regular,
            p.muted,
        ));
    }
    word_grid(ui, &p, &words, o.phrase_hidden);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 20.0;
        let (icon, label) = if o.phrase_hidden {
            ("eye", tr!("Show"))
        } else {
            ("eye-off", tr!("Hide"))
        };
        if text_action(ui, a, icon, label).clicked() {
            o.phrase_hidden = !o.phrase_hidden;
        }
        let copy_label = match clipboard::copied_at(ui.ctx()) {
            Some(at) => format!(
                tr!("Copied · wiped in {} s"),
                clipboard::KEEP.saturating_sub(at.elapsed()).as_secs() + 1
            ),
            None => tr!("Copy for 60 s").to_string(),
        };
        if text_action(ui, a, "copy", &copy_label).clicked() {
            clipboard::copy_secret(ui.ctx(), zeroize::Zeroizing::new(words.join(" ")));
        }
    });
    warning(
        ui,
        a,
        tr!(
            "Write the words on paper in order. Do not take photos or keep them in notes or the cloud. Whoever has the phrase can take all the funds."
        ),
    );
    checkbox(
        ui,
        a,
        &mut o.written,
        tr!("I wrote the phrase down and keep it in a safe place"),
    );
    if actions(ui, a, None, tr!("Continue"), o.written && !words.is_empty()).1 {
        o.next();
    }
}

/// Three words per row: raised cells, the number muted, the word in mono.
fn word_grid(ui: &mut Ui, p: &Palette, words: &[&str], hidden: bool) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        let w = (ui.available_width() - 16.0) / 3.0;
        for (r, row) in words.chunks(3).enumerate() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                for (c, word) in row.iter().enumerate() {
                    let (rect, _) = ui.allocate_exact_size(vec2(w, 40.0), Sense::hover());
                    ui.painter().rect(
                        rect,
                        CornerRadius::same(10),
                        p.raised,
                        Stroke::new(1.0, p.line),
                        StrokeKind::Inside,
                    );
                    ui.painter().text(
                        pos2(rect.left() + 12.0, rect.center().y),
                        Align2::LEFT_CENTER,
                        crate::i18n::t(&format!("{:02}", r * 3 + c + 1)),
                        mono(12.0, Weight::Regular),
                        p.muted,
                    );
                    ui.painter().text(
                        pos2(rect.left() + 40.0, rect.center().y),
                        Align2::LEFT_CENTER,
                        crate::i18n::t(if hidden { "••••••" } else { word }),
                        mono(14.0, Weight::Medium),
                        p.text,
                    );
                }
            });
        }
    });
}

/// O2 · Phrase check: pick the word at each asked position.
pub(super) fn verify(ui: &mut Ui, a: &mut Assets, o: &mut Onboarding) {
    let p = a.p;
    steps(ui, &p, 2);
    heading(
        ui,
        &p,
        tr!("Let's check it is all written down"),
        tr!(
            "Pick the words at these numbers. If you got one wrong, go back and look at the phrase again."
        ),
    );
    let width = ui.available_width();
    let words: Vec<String> = o
        .phrase
        .as_ref()
        .map(|ph| ph.words().to_vec())
        .unwrap_or_default();
    if words.len() < 12 {
        ui.label(txt(
            tr!("Preparing the phrase…"),
            14.0,
            Weight::Regular,
            p.muted,
        ));
        return;
    }
    // Chosen once per phrase: going back to look at the phrase does not reshuffle it.
    let fixed = o.fixed_check;
    let asked = o
        .checks
        .get_or_insert_with(|| {
            if fixed {
                demo_checks(&words)
            } else {
                random_checks(&words, &mut Dice::new())
            }
        })
        .clone();
    for (q, (number, choices)) in asked.iter().enumerate() {
        let right = &words[number - 1];
        card(&p, Margin::symmetric(16, 14)).show(ui, |ui| {
            ui.set_width(inner(width, 16));
            ui.spacing_mut().item_spacing.y = 10.0;
            ui.horizontal(|ui| {
                ui.label(txt(
                    &format!(tr!("Word no. {number}"), number = number),
                    14.0,
                    Weight::SemiBold,
                    p.text,
                ));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    match o.answers[q].map(|i| choices[i] == *right) {
                        Some(true) => {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            ui.label(txt(tr!("Correct"), 12.0, Weight::Medium, p.muted));
                            ui.add(a.icons.icon("check", p.muted, &p, 14.0));
                        }
                        Some(false) => {
                            ui.label(txt(
                                tr!("Not this word. Check your record."),
                                12.0,
                                Weight::Medium,
                                p.danger,
                            ));
                        }
                        None => {}
                    }
                });
            });
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let w = (ui.available_width() - 24.0) / 4.0;
                for (i, word) in choices.iter().enumerate() {
                    let on = o.answers[q] == Some(i);
                    let (rect, resp) = ui.allocate_exact_size(vec2(w, 38.0), Sense::click());
                    crate::components::named_choice(&resp, egui::WidgetType::RadioButton, on, word);
                    let (fill, stroke, color) = if on {
                        (p.ink, p.ink, p.on_ink)
                    } else if resp.hovered() {
                        (p.raised, p.muted, p.text)
                    } else {
                        (p.bg, p.line, p.text)
                    };
                    ui.painter().rect(
                        rect,
                        CornerRadius::same(10),
                        fill,
                        Stroke::new(1.0, stroke),
                        StrokeKind::Inside,
                    );
                    ui.painter().text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        crate::i18n::t(word),
                        mono(14.0, Weight::Medium),
                        color,
                    );
                    if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                        o.answers[q] = Some(i);
                    }
                }
            });
        });
    }
    let all_right = asked.iter().enumerate().all(|(q, (number, choices))| {
        o.answers[q].is_some_and(|i| choices[i] == words[number - 1])
    });
    let (show_again, confirm) = actions(
        ui,
        a,
        Some(tr!("Show the phrase again")),
        tr!("Confirm"),
        all_right,
    );
    if show_again {
        o.step = Step::Phrase;
    }
    if confirm {
        o.next();
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Flow, Onboarding};
    use super::{Checks, DEMO_ASKED, Dice, demo_checks, random_checks};
    use evolite_api::{Event, Phrase};

    fn words(n: usize) -> Vec<String> {
        (1..=n).map(|i| format!("w{i}")).collect()
    }

    fn assert_sound(words: &[String], checks: &Checks) {
        assert_eq!(checks.len(), 3);
        for (number, choices) in checks {
            assert!((1..=words.len()).contains(number));
            assert_eq!(choices.len(), 4);
            let right = &words[number - 1];
            assert_eq!(
                choices.iter().filter(|c| *c == right).count(),
                1,
                "word {number}"
            );
        }
    }

    #[test]
    fn every_check_has_the_right_word_once() {
        let demo = demo_checks(&words(12));
        assert_sound(&words(12), &demo);
        assert!(demo.iter().all(|(n, _)| DEMO_ASKED.contains(n)));
        let mut dice = Dice::new();
        for len in [12, 24] {
            for _ in 0..50 {
                let checks = random_checks(&words(len), &mut dice);
                assert_sound(&words(len), &checks);
                let numbers: Vec<usize> = checks.iter().map(|(n, _)| *n).collect();
                assert!(numbers.windows(2).all(|w| w[0] < w[1]), "{numbers:?}");
            }
        }
    }

    /// The check cannot be learnt: the asked positions and the place of the right word vary.
    #[test]
    fn the_check_is_not_the_same_every_time() {
        let words = words(12);
        let mut dice = Dice::new();
        let runs: Vec<Checks> = (0..30).map(|_| random_checks(&words, &mut dice)).collect();
        let places = |c: &Checks| -> Vec<usize> {
            c.iter()
                .map(|(n, choices)| choices.iter().position(|w| *w == words[n - 1]).unwrap())
                .collect()
        };
        assert!(runs.iter().any(|c| places(c) != places(&runs[0])));
        assert!(
            runs.iter()
                .any(|c| c[0].0 != runs[0][0].0 || c[1].0 != runs[0][1].0)
        );
    }

    /// A phrase repeating a word still gets distinct choices with one right answer.
    #[test]
    fn a_repeated_word_is_offered_once() {
        let mut words = words(12);
        words[3] = "w1".into();
        let mut dice = Dice::new();
        for _ in 0..50 {
            assert_sound(&words, &random_checks(&words, &mut dice));
        }
    }

    /// A new phrase (another length) clears the answers, the check and "I wrote it down".
    #[test]
    fn a_new_phrase_starts_the_check_again() {
        let mut o = Onboarding::new(Flow::Create);
        let phrase = |w: Vec<String>| Event::NewPhrase(Phrase::new(w));
        o.on_event(&phrase(words(12)));
        o.written = true;
        o.answers = [Some(1), Some(0), Some(2)];
        o.checks = Some(demo_checks(&words(12)));
        o.on_event(&phrase(words(24)));
        assert!(!o.written);
        assert_eq!(o.answers, [None; 3]);
        assert!(o.checks.is_none());
    }
}
