//! Identity, the private pool and the collection (branch I: BUG-46 and the designer's review
//! 2026-11-05 №7, 14, 15, 16, 27): how their cards stand in the 1024 px window.
//!
//! Cards are frames, not in the tree: a card's edge is a label's edge less the card's margin
//! (24 + the 1 px line, `components::card`) and what stands before the label in it.

use super::*;
use crate::i18n::{Lang, spoken, t_in};
use crate::screens::Screen;
use evolite_api::{IdentityChoice, Wallet};

const WINDOW: f32 = 1024.0;
/// «Platform credits» and «In the pool»: margin 24 + line 1 + icon 18 + gap 8.
const TITLE_AFTER_ICON: f32 = 51.0;
/// A list card's title («DPNS names», «Keys»): margin 20 + line 1.
const LIST_TITLE: f32 = 21.0;
/// «How it works»: margin 24 + line 1.
const PLAIN_TITLE: f32 = 25.0;
/// The name in the profile card: margin 24 + line 1 + avatar 72 + gap 16.
const NAME_IN_PROFILE: f32 = 113.0;

/// Changes the wallet the app shows, and draws, a few times over: a snapshot of the demo core
/// in between would undo it.
pub(super) fn with_wallet(kit: &mut Kit, change: impl Fn(&mut Wallet)) {
    for _ in 0..3 {
        if let Some(wallet) = kit
            .state_mut()
            .snapshot
            .as_mut()
            .and_then(|s| s.wallet.as_mut())
        {
            change(wallet);
        }
        kit.run_steps(1);
    }
}

/// The shown identity's name, as the profile card writes it.
fn shown_name(kit: &Kit) -> String {
    let wallet = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref());
    let name = wallet
        .and_then(|w| w.identity.as_ref())
        .and_then(|i| i.name());
    format!("@{}", name.expect("the demo identity has a name"))
}

/// The name in the profile card: the topmost of its labels (the «DPNS names» card says it too).
fn profile_name(kit: &Kit) -> egui::Rect {
    let name = shown_name(kit);
    kit.query_all(By::new().role(Role::Label).value(&name))
        .map(|n| n.rect())
        .min_by(|a, b| a.top().total_cmp(&b.top()))
        .expect("the name on screen")
}

fn close(a: f32, b: f32, what: &str) {
    assert!((a - b).abs() <= 1.0, "{what}: {a} and {b}");
}

/// BUG-46, its steps: «Work as» an identity with a 24-character name. The name wraps or ends in
/// «…» inside its card; the «Platform credits» card and its «Transfer» stay in the window.
#[test]
fn a_long_name_keeps_the_credits_in_the_window() {
    let (_turn, mut kit) = demo_app("identity", "ru");
    with_wallet(&mut kit, |w| {
        if let Some(id) = &mut w.identity {
            for n in &mut id.names {
                n.label = "evolite-probe-115567219".into();
            }
        }
    });
    let credits = words(&kit, &t_in(Lang::Ru, "Platform credits")).rect();
    let transfer = button(&kit, &spoken("Transfer")).rect();
    eprintln!("«Кредиты Platform» {credits:?}, «Перевести» {transfer:?}");
    assert!(
        credits.right() <= WINDOW && transfer.right() <= WINDOW,
        "the credits card stands past the window: «Перевести» ends at {}",
        transfer.right()
    );
}

/// №7: one grid on Identity — the two cards of each row are halves with a gap of 20, and the
/// second column begins where it does in the row below.
#[test]
fn identity_cards_stand_on_one_grid() {
    let (_turn, kit) = demo_app("identity", "en");
    let profile = profile_name(&kit).left() - NAME_IN_PROFILE;
    let credits = words(&kit, "Platform credits").rect().left() - TITLE_AFTER_ICON;
    let names = words(&kit, "DPNS names").rect().left() - LIST_TITLE;
    let keys = words(&kit, "Keys").rect().left() - LIST_TITLE;
    eprintln!("profile {profile}, credits {credits}, names {names}, keys {keys}");
    close(profile, names, "the first column");
    close(credits, keys, "the second column");
}

/// №7: one grid on the pool — «In the pool» and «How it works» are halves, and the actions
/// under them stand in the same two columns: at 1024 px four in a row (responsive spec R4: each
/// at least a tile wide), the first and the third at the halves' left edges.
#[test]
fn pool_cards_stand_on_one_grid() {
    let (_turn, kit) = demo_app("shielded", "en");
    let balance = words(&kit, "In the pool").rect().left() - TITLE_AFTER_ICON;
    let how = words(&kit, "How it works").rect().left() - PLAIN_TITLE;
    let first = button(&kit, "Shield").rect().left();
    let second = button(&kit, "Private address").rect().left();
    let send = button(&kit, "Send privately").rect();
    assert!(
        (send.top() - button(&kit, "Shield").rect().top()).abs() < 0.5,
        "four in a row"
    );
    eprintln!("balance {balance}, how {how}, actions {first} and {second}");
    close(balance, first, "the first column");
    close(how, second, "the second column");
}

/// №14: with many identities «Work as» wraps, and the second row of pills begins under the
/// first pill, not under «Work as».
#[test]
fn work_as_wraps_under_the_first_pill() {
    let (_turn, mut kit) = demo_app("identity", "en");
    with_wallet(&mut kit, |w| {
        w.identities.truncate(1);
        for slot in 1..8 {
            w.identities.push(IdentityChoice {
                id: format!("{slot}Kx2LabQ7pWmR9sTzV4nB8cYd1Je3HuF6gNqA2oX7wE"),
                name: Some(format!("evolite-probe-11556721{slot}")),
                slot,
            });
        }
    });
    let label = words(&kit, "Work as").rect();
    let pills: Vec<egui::Rect> = (1..8)
        .map(|slot| button(&kit, &format!("@evolite-probe-11556721{slot}")).rect())
        .collect();
    let first = button(&kit, &shown_name(&kit)).rect();
    let wrapped: Vec<_> = pills.iter().filter(|r| r.top() > first.bottom()).collect();
    assert!(!wrapped.is_empty(), "eight pills wrap at 1024 px");
    let row_start = wrapped.iter().map(|r| r.left()).fold(f32::MAX, f32::min);
    eprintln!("«Work as» {label:?}, the first pill {first:?}, the second row from {row_start}");
    close(row_start, first.left(), "the second row's start");
    assert!(row_start > label.right(), "not under «Work as»");
}

/// №15: «Edit profile» is a text action in the card's top right corner: its words end at the
/// card's right margin, above the name.
#[test]
fn edit_profile_stands_in_the_cards_corner() {
    let (_turn, kit) = demo_app("identity", "en");
    let name = profile_name(&kit);
    let credits = words(&kit, "Platform credits").rect().left() - TITLE_AFTER_ICON;
    // The words stand beside the action's own click area (`a11y::row_click`, BUG-40).
    let words = kit
        .query_by_role_and_label(Role::Label, "Edit profile")
        .or_else(|| kit.query_by_role_and_label(Role::Label, "Create a profile"))
        .expect("the profile action's words")
        .rect();
    // The profile card ends 20 before the credits card; its words end 24 + 1 inside it.
    eprintln!("words {words:?}, name {name:?}, credits card from {credits}");
    close(words.right(), credits - 20.0 - 25.0, "the right margin");
    assert!(words.bottom() <= name.top(), "above the name");
}

/// №16: «Save» in «DashPay profile» is off until something is typed or changed — a write costs
/// credits — and the reason stands beside it.
#[test]
fn an_unchanged_profile_is_not_saved() {
    let (_turn, mut kit) = demo_app("identity", "en");
    let action = if kit
        .query_by_role_and_label(Role::Button, "Edit profile")
        .is_some()
    {
        "Edit profile"
    } else {
        "Create a profile"
    };
    button(&kit, action).click();
    kit.run_steps(3);
    let save = |kit: &Kit| !button(kit, "Save").accesskit_node().is_disabled();
    assert!(!save(&kit), "«Save» with nothing changed");
    assert!(has_text(&kit, "Nothing has changed yet"), "the reason");
    let display = kit
        .state()
        .identity
        .editing
        .as_ref()
        .expect("23 open")
        .display
        .clone();
    let field = kit
        .query_all_by_role(Role::TextInput)
        .find(|n| n.accesskit_node().value().as_deref() == Some(display.as_str()))
        .expect("the «Name» field");
    field.click();
    kit.run_steps(2);
    kit.query_all_by_role(Role::TextInput)
        .find(|n| n.accesskit_node().is_focused())
        .expect("the field takes the typing")
        .type_text("x");
    kit.run_steps(2);
    assert!(save(&kit), "«Save» once the name is changed");
    assert!(!has_text(&kit, "Nothing has changed yet"));
}

/// №27: «Shield funds» opens with an empty amount and no error: nothing is put in for the
/// person, «Max» is beside the field.
#[test]
fn shielding_opens_empty_and_without_an_error() {
    let (_turn, mut kit) = demo_app("shielded", "en");
    wait_for(&mut kit, "«Shield funds» on", |kit| {
        !button(kit, "Shield funds").accesskit_node().is_disabled()
    });
    button(&kit, "Shield funds").click();
    kit.run_steps(3);
    assert!(kit.state().pool.shielding, "the dialog is open");
    let amount = kit.state().pool.amount.clone();
    assert!(amount.is_empty(), "the amount is {amount:?}");
    assert!(!has_text(&kit, "More than there is in this balance"));
    assert!(
        kit.query_by_label("Max").is_some(),
        "«Max» beside the field"
    );
}

/// The cards painted in the last frame: filled rectangles with the card's radius
/// (`components::card`; a card in a row also lays out an unpainted frame of its own).
fn card_backgrounds(kit: &Kit) -> Vec<egui::Rect> {
    fn walk(shape: &egui::Shape, found: &mut Vec<egui::Rect>) {
        match shape {
            egui::Shape::Rect(r)
                if r.corner_radius == egui::CornerRadius::same(16)
                    && r.fill != egui::Color32::TRANSPARENT =>
            {
                found.push(r.rect);
            }
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, found)),
            _ => {}
        }
    }
    let mut found = Vec::new();
    for clipped in &kit.output().shapes {
        walk(&clipped.shape, &mut found);
    }
    found
}

/// BUG-62: the cards of a row are one height from the very first frame of the screen, and stay
/// so — frame after frame nothing under the first row moves. Before, the heights were measured
/// the frame before: the first frame showed a short card, and the two cards of a row took turns
/// growing every frame (`grow` grew a card by about its own height more than asked), so the
/// tiles of Identity and the pool flickered.
#[test]
fn the_cards_stand_still_frame_after_frame() {
    for (screen, under) in [
        (Screen::Identity, "DPNS names"),
        (Screen::Shielded, "Private address"),
    ] {
        let (_turn, mut kit) = demo_app("overview", "en");
        kit.run_steps(2);
        kit.state_mut().screen = screen;
        let mut tops = Vec::new();
        for frame in 0..8 {
            kit.run_steps(1);
            let top = kit
                .query_all_by(|n| {
                    n.value().as_deref() == Some(under) || n.label().as_deref() == Some(under)
                })
                .next()
                .unwrap_or_else(|| panic!("{screen:?}, frame {frame}: no «{under}»"))
                .rect()
                .top();
            tops.push(top);
            // Every row of two cards side by side: one height.
            let cards = card_backgrounds(&kit);
            let mut rows = 0;
            for (i, a) in cards.iter().enumerate() {
                for b in &cards[i + 1..] {
                    if (a.top() - b.top()).abs() < 0.5 && a.left() != b.left() {
                        rows += 1;
                        assert!(
                            (a.height() - b.height()).abs() < 0.5,
                            "{screen:?}, frame {frame}: the cards of a row differ: {a:?}, {b:?}"
                        );
                    }
                }
            }
            assert!(
                rows >= 1,
                "{screen:?}, frame {frame}: no row of two in {cards:?}"
            );
        }
        assert!(
            tops.windows(2).all(|w| (w[0] - w[1]).abs() < 0.5),
            "{screen:?}: «{under}» under the first row moves: {tops:?}"
        );
    }
}

/// BUG-54 part 2, BUG-93: «Max» on «Create an identity» leaves the name, the creation and the
/// Core fee of the asset lock's transaction — all of them on top of the amount — and the sum
/// it then asks for is not «More than there is on Core».
#[test]
fn max_for_a_new_identity_leaves_the_lock_its_fee() {
    let (_turn, mut kit) = demo_app("create-identity", "en");
    kit.query_by_label("Max")
        .expect("«Max» beside the field")
        .click();
    kit.run_steps(3);
    let wallet = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.clone())
        .expect("the demo wallet");
    let fees = &wallet.fees;
    let amount = crate::format::parse_amount(&kit.state().create_identity.amount)
        .expect("«Max» puts an amount in");
    assert_eq!(
        amount,
        wallet.balances.core - fees.name_on_top() - fees.identity_create - fees.core_payment,
        "everything but the name, the creation and the lock's Core fee"
    );
    assert!(!has_text(&kit, "More than there is on Core"));
}

/// BUG-93 review should 3: without a name «Max» leaves only the creation and the lock's Core
/// fee: nothing is paid for a name.
#[test]
fn max_for_a_nameless_identity_leaves_only_the_creation_and_the_core_fee() {
    let (_turn, mut kit) = demo_app("create-identity-nameless", "en");
    kit.query_by_label("Max")
        .expect("«Max» beside the field")
        .click();
    kit.run_steps(3);
    let wallet = kit
        .state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.clone())
        .expect("the demo wallet");
    let amount = crate::format::parse_amount(&kit.state().create_identity.amount)
        .expect("«Max» puts an amount in");
    assert_eq!(
        amount,
        wallet.balances.core - wallet.fees.identity_create - wallet.fees.core_payment,
        "everything but the creation and the lock's Core fee"
    );
}

/// BUG-54 part 2, review r1 must; BUG-93: the least amount is in the form's own terms, what
/// becomes credits: at testnet's prices the name's peak wants 130 000 on the identity and the
/// 90 000 on top covers part of it, so 40 000 at least — the form says so and does not
/// create below it; at the least it does.
#[test]
fn a_new_identity_asks_for_at_least_what_platform_takes() {
    let (_turn, mut kit) = demo_app("create-identity", "en");
    let least = format!(
        "At least {} — less may not cover the name's registration",
        crate::format::amount(75_000)
    );
    // Review r2 should 1: the button too, not only the hint.
    let create_on = |kit: &Kit| {
        kit.query_all_by_role(Role::Button)
            .find(|b| {
                b.accesskit_node()
                    .label()
                    .is_some_and(|l| l.starts_with("Create for "))
            })
            .is_some_and(|b| !b.accesskit_node().is_disabled())
    };
    // The demo core may send its own snapshot first: the fees are set again until they stay.
    let shown = |kit: &mut Kit, amount: &str| {
        for _ in 0..50 {
            kit.state_mut().create_identity.amount = amount.to_owned();
            if let Some(w) = kit
                .state_mut()
                .snapshot
                .as_mut()
                .and_then(|s| s.wallet.as_mut())
            {
                w.fees.name_regular = 65_000;
                w.fees.identity_create = 175_000;
                w.fees.identity_lock_min = 241_000;
            }
            kit.run_steps(1);
            let kept = kit
                .state()
                .snapshot
                .as_ref()
                .and_then(|s| s.wallet.as_ref())
                .is_some_and(|w| w.fees.identity_lock_min == 241_000);
            if kept {
                return (has_text(kit, &least), create_on(kit));
            }
        }
        panic!("the fees did not stay");
    };
    assert_eq!(
        shown(&mut kit, "0.0001"),
        (true, false),
        "0.0001 is too little: the hint, «Create» off"
    );
    assert_eq!(
        shown(&mut kit, "0.00074"),
        (true, false),
        "just under the least: the hint, «Create» off"
    );
    assert_eq!(
        shown(&mut kit, "0.00075"),
        (false, true),
        "the least is enough: no hint, «Create» on"
    );
}

/// Responsive spec R4: the cards of every row of a grid — Overview's tiles and
/// the pool's two cards — are one height in the very first frame of the screen, at the
/// four widths of the spec, left to right and mirrored (`ar`); and Overview's four tiles stand
/// 4 in a row or 2 by 2, never 3 and 1.
#[test]
fn grid_rows_are_one_height_in_the_first_frame_at_every_width() {
    for lang in ["en", "ar"] {
        for width in [800.0, 1024.0, 1280.0, 1600.0] {
            for screen in [Screen::Overview, Screen::Shielded] {
                let (_turn, mut kit) = demo_app("overview", lang);
                kit.set_size(egui::vec2(width, 900.0));
                kit.run_steps(2);
                kit.state_mut().screen = screen;
                kit.run_steps(1);
                let cards = card_backgrounds(&kit);
                let mut in_rows = std::collections::BTreeMap::<i32, usize>::new();
                for (i, a) in cards.iter().enumerate() {
                    for b in &cards[i + 1..] {
                        if (a.top() - b.top()).abs() < 0.5 && a.left() != b.left() {
                            assert!(
                                (a.height() - b.height()).abs() < 0.5,
                                "{screen:?} {lang} {width}: the cards of a row differ: {a:?}, {b:?}"
                            );
                        }
                    }
                    *in_rows.entry(a.top().round() as i32).or_default() += 1;
                }
                let most = in_rows.values().copied().max().unwrap_or(0);
                // The pool's two cards stand side by side only in two columns (760 and up).
                let two = screen != Screen::Shielded || width >= 1024.0;
                assert!(
                    !two || most >= 2,
                    "{screen:?} {lang} {width}: no row in {cards:?}"
                );
                if screen == Screen::Overview {
                    assert!(
                        in_rows.values().all(|n| [1, 2, 4].contains(n)),
                        "{lang} {width}: tiles in rows of {in_rows:?}"
                    );
                }
            }
        }
    }
}

/// Responsive spec, «Длинный текст»: a tile's amount is never cut nor wrapped — in the 1024 px
/// window Overview's four tiles stand in a row (R4), and 234.56789012 DASH, too wide at 24,
/// is one step smaller: one line, inside its tile.
#[test]
fn a_wide_amount_takes_a_smaller_font_in_its_tile() {
    let (_turn, mut kit) = demo_app("overview", "en");
    let duffs = 23_456_789_012;
    with_wallet(&mut kit, |w| w.balances.core = duffs);
    let shown = crate::format::number(duffs, false);
    let amount = kit
        .query_all(By::new().role(Role::Label).value(&shown))
        .map(|n| n.rect())
        .min_by(|a, b| a.top().total_cmp(&b.top()))
        .unwrap_or_else(|| panic!("«{shown}» on Overview"));
    let tiles = card_backgrounds(&kit);
    let tile = tiles
        .iter()
        .find(|t| t.contains(amount.center()))
        .unwrap_or_else(|| panic!("«{shown}» {amount:?} in a tile of {tiles:?}"));
    let in_row = tiles
        .iter()
        .filter(|t| (t.top() - tile.top()).abs() < 0.5)
        .count();
    assert_eq!(in_row, 4, "four tiles in a row at 1024: {tiles:?}");
    assert!(amount.height() < 30.0, "one line: {amount:?}");
    assert!(
        amount.right() <= tile.right() - 20.0,
        "inside: {amount:?} in {tile:?}"
    );
}

/// The amounts of Overview's tiles as drawn, left to right: in each tile of the row the tallest
/// label inside it (the title is 13, the detail 12). `core` puts that balance into the wallet.
fn tile_amounts(core: i64) -> Vec<egui::Rect> {
    let (_turn, mut kit) = demo_app("overview", "en");
    with_wallet(&mut kit, |w| w.balances.core = core);
    let shown = crate::format::number(core, false);
    let amount = kit
        .query_all(By::new().role(Role::Label).value(&shown))
        .map(|n| n.rect())
        .min_by(|a, b| a.top().total_cmp(&b.top()))
        .unwrap_or_else(|| panic!("«{shown}» on Overview"));
    let tiles = card_backgrounds(&kit);
    let core_tile = *tiles
        .iter()
        .find(|t| t.contains(amount.center()))
        .unwrap_or_else(|| panic!("«{shown}» {amount:?} in a tile of {tiles:?}"));
    let mut row: Vec<egui::Rect> = tiles
        .into_iter()
        .filter(|t| (t.top() - core_tile.top()).abs() < 0.5)
        .collect();
    row.sort_by(|a, b| a.left().total_cmp(&b.left()));
    assert_eq!(row.len(), 4, "four tiles in a row at 1024: {row:?}");
    row.iter()
        .map(|tile| {
            kit.query_all(By::new().role(Role::Label))
                .map(|n| n.rect())
                .filter(|r| tile.contains_rect(*r))
                .max_by(|a, b| a.height().total_cmp(&b.height()))
                .unwrap_or_else(|| panic!("no words in {tile:?}"))
        })
        .collect()
}

/// The designer, 2026-11-07 (responsive spec, under the table of section 1): the amounts of a
/// row of tiles are of one size — 24, or 20 for all of them when one does not fit at 24, never
/// 20 in one tile and 24 beside it. In the 1024 px window 234.56789012 DASH of Core makes the
/// short credits, address and pool amounts 20 too; with short amounts only, all are 24.
#[test]
fn a_row_of_tiles_has_one_amount_size() {
    let short = tile_amounts(12_345_678);
    let wide = tile_amounts(23_456_789_012);
    for (what, row) in [("short", &short), ("wide", &wide)] {
        assert!(
            row.iter()
                .all(|r| (r.height() - row[0].height()).abs() < 0.5),
            "{what}: the amounts of a row differ in size: {row:?}"
        );
    }
    assert!(
        wide[0].height() + 2.0 < short[0].height(),
        "a wide amount makes the row smaller ({wide:?}), short ones stay 24 ({short:?})"
    );
}

/// R4a (the designer, 2026-11-07): in the 1024 px window 1,234.56789012 DASH of Core is too
/// wide even at 20. It is not wrapped: fewer decimals, rounded, «≈» in front, on one line inside
/// its tile, of one size with the row; the full amount with its unit is the screen reader's
/// description and the tooltip on Tab focus (nothing of it before). «Hide amounts» hides it all.
#[test]
fn a_four_digit_balance_is_shortened_in_its_tile_and_said_in_full() {
    const CORE: i64 = 123_456_789_012;
    const FULL: &str = "1,234.56789012 DASH";
    let (_turn, mut kit) = demo_app("overview", "en");
    kit.set_size(egui::vec2(WINDOW, 800.0));
    with_wallet(&mut kit, |w| w.balances.core = CORE);
    let node = kit
        .query_all(By::new().role(Role::Label))
        .find(|n| n.value().is_some_and(|v| v.starts_with("≈ 1,234.")))
        .unwrap_or_else(|| panic!("a shortened «≈ 1,234.…» on Overview"));
    let shown = node.value().unwrap_or_default();
    let amount = node.rect();
    assert_eq!(node.accesskit_node().description().as_deref(), Some(FULL));
    let decimals = shown.rsplit_once('.').map_or(0, |(_, f)| f.len());
    assert!(
        (2..8).contains(&decimals),
        "at least two decimals, fewer than eight: «{shown}»"
    );
    let rounded = format!("{:.*}", decimals, CORE as f64 / 1e8);
    assert!(
        shown.ends_with(&rounded[1..]),
        "rounded half up: «{shown}» for {rounded}"
    );
    let tiles = card_backgrounds(&kit);
    let tile = tiles
        .iter()
        .find(|t| t.contains(amount.center()))
        .unwrap_or_else(|| panic!("«{shown}» {amount:?} in a tile of {tiles:?}"));
    assert!(amount.height() < 30.0, "one line: {amount:?}");
    assert!(
        amount.right() <= tile.right() - 20.0,
        "inside: {amount:?} in {tile:?}"
    );
    // The other amounts of the row: the same size as the shortened one.
    let row: Vec<egui::Rect> = tiles
        .iter()
        .filter(|t| (t.top() - tile.top()).abs() < 0.5)
        .map(|t| {
            kit.query_all(By::new().role(Role::Label))
                .map(|n| n.rect())
                .filter(|r| t.contains_rect(*r))
                .max_by(|a, b| a.height().total_cmp(&b.height()))
                .unwrap_or_else(|| panic!("no words in {t:?}"))
        })
        .collect();
    assert_eq!(row.len(), 4, "four tiles in a row at 1024: {row:?}");
    assert!(
        row.iter()
            .all(|r| (r.height() - amount.height()).abs() < 0.5),
        "one size in the row: {row:?}, the shortened {amount:?}"
    );
    // On Tab focus the full amount shows; not before.
    let full_shown = |kit: &Kit| {
        kit.query_all(By::new().role(Role::Label))
            .any(|n| n.value().as_deref() == Some(FULL))
    };
    assert!(!full_shown(&kit), "the full amount before any focus");
    let mut reached = false;
    for _ in 0..40 {
        with_wallet(&mut kit, |w| w.balances.core = CORE);
        if kit
            .query_by(|n| n.is_focused())
            .is_some_and(|n| n.value().is_some_and(|v| v.starts_with("≈ ")))
        {
            reached = true;
            break;
        }
        tab(&mut kit, false);
    }
    assert!(reached, "Tab never reached the shortened amount");
    kit.run_steps(2);
    assert!(full_shown(&kit), "no full amount on keyboard focus");
    // Hidden: «••••», no description, no tooltip.
    kit.state_mut().settings.flip_hide_amounts();
    with_wallet(&mut kit, |w| w.balances.core = CORE);
    kit.run_steps(2);
    assert!(
        !kit.query_all(By::new().role(Role::Label))
            .any(|n| n.value().is_some_and(|v| v.contains("1,234"))
                || n.accesskit_node().description().as_deref() == Some(FULL)),
        "a hidden amount says nothing of itself"
    );
}

/// Responsive spec R4 and the designer, 2026-11-07: Collection's NFT cards (195 at the least,
/// gap 16) stand as many in a row as fit beside the details panel (300 + 24) — left to right and
/// mirrored (`ar`). At 800 the panel goes under the grid: 676 (margins 24, R2) - 10 (the scroll
/// bar) holds 3; at 1024 868 - 324 - 10 = 534 holds 2; at 1600 the content is 1200 (R2) and the
/// grid 866 holds 4. At 1280 the grid is 952 - 324 - 10 = 618 (the scroll bar of a list taller
/// than the window takes its 10): 3 x 195 + 2 x 16 = 617 fit, so 3 (the designer, after R4 r2).
#[test]
fn the_collection_takes_as_many_columns_as_fit_at_every_width() {
    for lang in ["en", "ar"] {
        for (width, cols) in [(800.0, 3), (1024.0, 2), (1280.0, 3), (1600.0, 4)] {
            let (_turn, mut kit) = demo_app("collection", lang);
            kit.set_size(egui::vec2(width, 900.0));
            kit.run_steps(2);
            let names: Vec<String> = kit
                .state()
                .snapshot
                .as_ref()
                .and_then(|s| s.wallet.as_ref())
                .map(|w| w.nfts.iter().map(|n| spoken(&n.name)).collect())
                .unwrap_or_default();
            assert!(names.len() > 4, "{lang} {width}: the demo NFTs: {names:?}");
            let cards: Vec<egui::Rect> = names.iter().map(|n| button(&kit, n).rect()).collect();
            let first = cards.iter().map(|r| r.top()).fold(f32::MAX, f32::min);
            let in_row = cards
                .iter()
                .filter(|r| (r.top() - first).abs() < 0.5)
                .count();
            let row: Vec<_> = cards
                .iter()
                .filter(|r| (r.top() - first).abs() < 0.5)
                .collect();
            let grid = row.iter().map(|r| r.right()).fold(f32::MIN, f32::max)
                - row.iter().map(|r| r.left()).fold(f32::MAX, f32::min);
            eprintln!("{lang} {width}: {in_row} columns in {grid}");
            assert_eq!(in_row, cols, "{lang} {width}: the first row of {cards:?}");
        }
    }
}

/// BUG-95: a system that cannot isolate pictures is said once, in words that fit, at the usual
/// window and in the narrow one; not on every tile.
#[test]
fn pictures_off_is_said_once_on_the_collection() {
    use evolite_api::{CoreError, PicturesOff};
    let text = crate::art::pictures_off_text(PicturesOff::NoFileIsolation);
    for size in [egui::vec2(1024.0, 768.0), egui::vec2(853.0, 533.0)] {
        let (_turn, mut kit) = demo_app("collection", "en");
        kit.set_size(size);
        let ctx = kit.ctx.clone();
        kit.state_mut().collection.art.on_answer(
            &ctx,
            "any",
            64,
            false,
            &Err(CoreError::PicturesOff(PicturesOff::NoFileIsolation)),
        );
        kit.run_steps(4);
        let notices: Vec<_> = kit.query_all_by_label(text).collect();
        assert_eq!(notices.len(), 1, "once, at {size:?}");
        let rect = notices[0].rect();
        assert!(
            rect.left() >= 0.0 && rect.right() <= size.x && rect.bottom() <= size.y,
            "inside the window at {size:?}: {rect:?}"
        );
    }
}

/// R4b, the rule of R4a on the pool's own card: at 1024 «1,234.56789012 DASH» at 36 is wider than
/// its half of the screen. It stays on one line inside the card, with fewer decimals and «≈», and
/// says the whole amount to a screen reader; wide enough, it is shown whole. Hidden, nothing.
#[test]
fn a_long_pool_balance_stays_on_one_line_and_is_said_in_full() {
    const POOL: i64 = 123_456_789_012;
    const FULL: &str = "1,234.56789012 DASH";
    let (_turn, mut kit) = demo_app("shielded", "en");
    kit.set_size(egui::vec2(WINDOW, 800.0));
    with_wallet(&mut kit, |w| w.balances.shielded = POOL);
    let node = kit
        .query_all(By::new().role(Role::Label))
        .find(|n| n.value().is_some_and(|v| v.starts_with("≈ 1,234.")))
        .unwrap_or_else(|| panic!("a shortened «≈ 1,234.…» on the pool's card"));
    let shown = node.value().unwrap_or_default();
    assert!(shown.ends_with(" DASH"), "with its unit: «{shown}»");
    assert_eq!(node.accesskit_node().description().as_deref(), Some(FULL));
    let amount = node.rect();
    assert!(amount.height() < 60.0, "one line at 36: {amount:?}");
    let cards = card_backgrounds(&kit);
    let inside = cards.iter().find(|c| c.contains(amount.center()));
    let card = inside.unwrap_or_else(|| panic!("{amount:?} in a card of {cards:?}"));
    assert!(
        amount.right() <= card.right() - 24.0,
        "inside the card's margin: {amount:?} in {card:?}"
    );

    kit.set_size(egui::vec2(1600.0, 800.0));
    with_wallet(&mut kit, |w| w.balances.shielded = POOL);
    assert!(has_text(&kit, FULL), "room for the whole amount");

    kit.set_size(egui::vec2(WINDOW, 800.0));
    kit.state_mut().settings.flip_hide_amounts();
    with_wallet(&mut kit, |w| w.balances.shielded = POOL);
    kit.run_steps(2);
    assert!(
        !kit.query_all(By::new().role(Role::Label))
            .any(|n| n.value().is_some_and(|v| v.contains("1,234"))
                || n.accesskit_node().description().as_deref() == Some(FULL)),
        "a hidden amount says nothing of itself"
    );
}
