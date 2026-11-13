//! BUG-80: every place that shows the Core balance goes through `CardAmount::as_core` — before
//! the first check (`CoreAsOf::NotYet`) it says «Ещё не знаем» or «—», never a 0, and no form
//! says «Больше, чем есть на Core» against a balance not known.

use super::*;
use evolite_api::{CoreAsOf, Snapshot, Wallet};

/// A label or value with `text` in it.
fn shows(kit: &Kit, text: &str) -> bool {
    kit.query_all_by(|n| {
        n.label().is_some_and(|l| l.contains(text)) || n.value().is_some_and(|v| v.contains(text))
    })
    .next()
    .is_some()
}

fn wallet(s: &mut Snapshot) -> &mut Wallet {
    s.wallet.as_mut().expect("a wallet")
}

/// Puts `change` on every snapshot and draws until `seen` holds on the same frame: the demo
/// core's own snapshots come in between.
fn shown_with(
    kit: &mut Kit,
    change: impl Fn(&mut Snapshot),
    what: &str,
    seen: impl Fn(&Kit) -> bool,
) {
    show_edited(
        kit,
        1,
        what,
        |app| {
            if let Some(s) = app.snapshot.as_mut() {
                change(s);
            }
        },
        seen,
    );
}

/// Core empty and as sure as `as_of`.
fn core(as_of: Option<CoreAsOf>) -> impl Fn(&mut Snapshot) {
    move |s| {
        let w = wallet(s);
        w.balances.core = 0;
        w.balances.core_as_of = as_of;
    }
}

const MORE: &str = "Больше, чем есть на Core";
/// FEAT-0014 (frame 9): the creation says it in place of the InstantSend hint, with both sums.
const NEEDED: &str = "Нужно не меньше ≈";
const NOT_KNOWN: &str = "Ещё не знаем";

/// «Создать identity»: the Core row says «Ещё не знаем», and the suggested 0,3 DASH is not «more
/// than there is»; with Core checked at 0 the same form says it is.
#[test]
fn create_identity_does_not_measure_against_core_not_known() {
    let (_turn, mut kit) = demo_app("create-identity", "ru");
    shown_with(
        &mut kit,
        core(Some(CoreAsOf::NotYet)),
        "Core not known yet",
        |k| has_text(k, NOT_KNOWN) && !shows(k, NEEDED),
    );
    shown_with(&mut kit, core(None), "Core checked at 0", |k| {
        shows(k, NEEDED) && !has_text(k, NOT_KNOWN)
    });
}

/// Identity → «Пополнить»: the same for the top-up's Core row.
#[test]
fn top_up_does_not_measure_against_core_not_known() {
    let (_turn, mut kit) = demo_app("top-up", "ru");
    shown_with(
        &mut kit,
        core(Some(CoreAsOf::NotYet)),
        "Core not known yet",
        |k| has_text(k, NOT_KNOWN) && !has_text(k, MORE),
    );
    shown_with(&mut kit, core(None), "Core checked at 0", |k| {
        has_text(k, MORE)
    });
}

/// Identity without an identity: «на Core сейчас …» only when Core is checked now — never «на
/// Core сейчас 0» before the first check, nor an old amount as «now».
#[test]
fn identity_says_what_core_has_only_when_checked_now() {
    let (_turn, mut kit) = demo_app("identity", "ru");
    let no_identity = |as_of: Option<CoreAsOf>| {
        move |s: &mut Snapshot| {
            core(as_of)(s);
            let w = wallet(s);
            w.identity = None;
            w.identities.clear();
            w.identity_search = evolite_api::IdentitySearch::Done;
        }
    };
    let now_line = "на Core сейчас";
    shown_with(&mut kit, no_identity(None), "Core checked: the line", |k| {
        shows(k, now_line)
    });
    shown_with(
        &mut kit,
        no_identity(Some(CoreAsOf::NotYet)),
        "Core not known: no line",
        |k| shows(k, "Identity ещё нет") && !shows(k, now_line),
    );
    let at = chrono::Local::now().timestamp() - 120;
    shown_with(
        &mut kit,
        no_identity(Some(CoreAsOf::Checked(at))),
        "Core checked earlier: no line",
        |k| shows(k, "Identity ещё нет") && !shows(k, now_line),
    );
}

/// Private pool → «Скрыть средства» from Core: the source says «Ещё не знаем», and no «От … до …»
/// is measured against it; an earlier check is shown with its time.
#[test]
fn shield_from_core_not_known_is_not_zero() {
    let (_turn, mut kit) = demo_app("shield-core", "ru");
    shown_with(
        &mut kit,
        core(Some(CoreAsOf::NotYet)),
        "Core not known yet",
        |k| shows(k, &format!("Core: {NOT_KNOWN}")) && !shows(k, "комиссия Platform берётся"),
    );
    let at = chrono::Local::now().timestamp() - 120;
    let line =
        crate::text::core_as_of(Some(CoreAsOf::Checked(at)), chrono::Local::now()).expect("a line");
    shown_with(
        &mut kit,
        core(Some(CoreAsOf::Checked(at))),
        "Core checked earlier",
        |k| shows(k, &format!(" · {line}")),
    );
}

/// The wallet menu: «open · —» while Core is not known yet, not «open · 0.0000 DASH». The
/// menu paints its lines (no AccessKit node of their own), so the line itself is checked, on
/// the snapshot the open menu draws.
#[test]
fn wallet_menu_total_is_a_dash_while_core_is_not_known() {
    let (_turn, kit) = demo_app("wallet-menu", "en");
    let mut snap = kit.state().snapshot.clone().expect("a snapshot");
    let open = snap
        .wallets
        .iter()
        .find(|w| w.open)
        .expect("an open wallet")
        .clone();
    let now = chrono::Local::now();
    let line = |snap: &Snapshot| crate::screens::wallets::menu_line(&open, snap, now);
    core(None)(&mut snap);
    assert_ne!(line(&snap), "open · —", "checked: an amount");
    core(Some(CoreAsOf::Checked(1)))(&mut snap);
    assert_ne!(line(&snap), "open · —", "checked earlier: the amount");
    core(Some(CoreAsOf::NotYet))(&mut snap);
    assert_eq!(line(&snap), "open · —");
}

/// Settings → «Удалить кошелёк»: the balance says «Ещё не знаем» while Core is not known yet.
#[test]
fn delete_dialog_balance_is_not_known_not_zero() {
    let (_turn, mut kit) = demo_app("delete", "ru");
    let zero = crate::format::amount(0);
    shown_with(
        &mut kit,
        |s| {
            core(Some(CoreAsOf::NotYet))(s);
            let b = &mut wallet(s).balances;
            b.all_credits = 0;
            b.platform_address = 0;
            b.shielded = 0;
        },
        "the balance not known",
        |k| has_text(k, "Баланс") && has_text(k, NOT_KNOWN) && !has_text(k, &zero),
    );
}

/// The one helper: as sure as Core, whatever the amount.
#[test]
fn a_core_amount_is_as_sure_as_core() {
    use crate::components::CardAmount;
    let mut b = evolite_api::Balances {
        core: 5,
        core_as_of: None,
        core_addresses: 0,
        credits: 0,
        all_credits: 7,
        platform_address: 0,
        shielded: 0,
        credits_read: true,
        credits_checked: true,
        platform_address_read: true,
        mixed: None,
    };
    assert_eq!(CardAmount::core(&b), CardAmount::Now(5));
    assert_eq!(CardAmount::total(&b), CardAmount::Now(12));
    b.core_as_of = Some(CoreAsOf::Checked(1));
    assert_eq!(CardAmount::core(&b), CardAmount::Earlier(5));
    assert_eq!(CardAmount::total(&b), CardAmount::Earlier(12));
    b.core_as_of = Some(CoreAsOf::NotYet);
    assert_eq!(CardAmount::core(&b), CardAmount::Unknown);
    assert_eq!(CardAmount::total(&b).known(), None);
    assert_eq!(CardAmount::Unknown.balance(), "—");
}

/// Platform not read yet: «—» and «Not known yet», not a 0 (BUG-91).
#[test]
fn overview_says_platform_is_not_known_until_read() {
    let (_turn, mut kit) = demo_app("overview", "en");
    kit.state_mut()
        .snapshot
        .as_mut()
        .and_then(|s| s.wallet.as_mut())
        .expect("wallet")
        .balances
        .credits_read = false;
    kit.state_mut()
        .snapshot
        .as_mut()
        .and_then(|s| s.wallet.as_mut())
        .expect("wallet")
        .balances
        .platform_address_read = false;
    kit.run_steps(3);
    assert!(has_text(&kit, "Not known yet"));
    assert!(!has_text(&kit, "0.0000"), "no zero while unknown");
    assert!(
        !has_text(&kit, "1.2000"),
        "the demo credits must not show as read"
    );
}

/// The total cannot be said while a part of it is not read.
#[test]
fn the_total_is_unknown_until_platform_is_read() {
    use crate::components::CardAmount;
    let mut b = evolite_api::Balances {
        core: 10,
        core_as_of: None,
        core_addresses: 0,
        credits: 0,
        all_credits: 7,
        platform_address: 0,
        shielded: 0,
        credits_read: false,
        credits_checked: false,
        platform_address_read: false,
        mixed: None,
    };
    assert_eq!(CardAmount::total(&b), CardAmount::Unknown);
    b.credits_read = true;
    assert_eq!(CardAmount::total(&b), CardAmount::Unknown);
    b.platform_address_read = true;
    // BUG-115: credits kept from an earlier run, not checked in this one: no total yet.
    assert_eq!(CardAmount::total(&b), CardAmount::Unknown);
    b.credits_checked = true;
    assert_eq!(CardAmount::total(&b), CardAmount::Now(17));
}

/// «Total» with Platform unread and Core empty: a dash, not a sum of what happens to be known.
/// The sum here is one no tile shows, so it can only be the total (BUG-91 r1, must 1).
#[test]
fn overview_total_is_a_dash_until_platform_is_read() {
    let (_turn, mut kit) = demo_app("overview", "en");
    let sum = crate::format::amount(2_000 + 300 + 40_000);
    let parts = |read: bool| {
        move |s: &mut Snapshot| {
            let b = &mut wallet(s).balances;
            b.core = 0;
            b.core_as_of = None;
            b.all_credits = 2_000;
            b.platform_address = 300;
            b.shielded = 40_000;
            b.credits_read = read;
            b.platform_address_read = read;
            // No mixed coins here (FEAT-0030): the sum is of these.
            b.mixed = None;
        }
    };
    shown_with(&mut kit, parts(false), "no sum while unread", |k| {
        has_text(k, "Total") && has_text(k, "Not known yet") && !has_text(k, &sum)
    });
    shown_with(&mut kit, parts(true), "the sum once read", |k| {
        has_text(k, &sum)
    });
}

/// The total's fiat line goes with the total.
#[test]
fn overview_has_no_fiat_line_while_the_total_is_not_known() {
    let (_turn, mut kit) = demo_app("overview", "en");
    kit.state_mut().settings.turn_on("fiat-rate");
    let with_rate = |read: bool| {
        move |s: &mut Snapshot| {
            s.fiat = Some(evolite_api::FiatRate {
                currency: "USD".into(),
                price: 4_800_000_000,
                at: chrono::Utc::now().timestamp(),
            });
            let b = &mut wallet(s).balances;
            b.credits_read = read;
            b.platform_address_read = read;
        }
    };
    shown_with(&mut kit, with_rate(true), "the fiat line once read", |k| {
        shows(k, "at the mainnet rate")
    });
    shown_with(&mut kit, with_rate(false), "no fiat while unread", |k| {
        !shows(k, "at the mainnet rate")
    });
}

/// Empty lists while Platform's search runs say «still reading», on the Overview and in History;
/// «none yet» only once it ended (BUG-91 r1, should 3).
#[test]
fn no_operations_waits_for_the_search_of_platform() {
    const READING: &str =
        "Still reading the operations — they appear here as soon as they are found";
    const NONE: &str = "No operations yet. Receive DASH on the “Receive” screen.";
    for screen in ["overview", "activity"] {
        let (_turn, mut kit) = demo_app(screen, "en");
        let empty = |read: bool| {
            move |s: &mut Snapshot| {
                let w = wallet(s);
                w.activity.clear();
                w.balances.credits_read = read;
            }
        };
        shown_with(&mut kit, empty(false), "still reading", |k| {
            has_text(k, READING) && !has_text(k, NONE)
        });
        shown_with(&mut kit, empty(true), "none yet", |k| {
            has_text(k, NONE) && !has_text(k, READING)
        });
    }
}

/// The pool's list says «still reading» until the pool was read once (BUG-91 r1, should 1).
#[test]
fn the_pool_list_is_still_reading_until_checked() {
    let (_turn, mut kit) = demo_app("shielded", "en");
    let empty = |checked: bool| {
        move |s: &mut Snapshot| {
            let w = wallet(s);
            w.pool.operations.clear();
            if !checked {
                w.pool.checked_at = None;
            }
        }
    };
    shown_with(&mut kit, empty(false), "still reading", |k| {
        has_text(
            k,
            "Still reading the operations — they appear here as soon as they are found",
        ) && !has_text(k, "No operations in the pool yet")
    });
}

/// Shielding: unread credits and address say «—», no 0, and such a row cannot be chosen
/// (BUG-91 r1, should 2).
#[test]
fn shield_sources_are_dashes_until_read_and_cannot_be_chosen() {
    let (_turn, mut kit) = demo_app("shield", "en");
    let unread = |s: &mut Snapshot| {
        let b = &mut wallet(s).balances;
        b.credits = 0;
        b.platform_address = 0;
        b.credits_read = false;
        b.platform_address_read = false;
    };
    kit.state_mut().pool.sources_open = true;
    let zero = crate::format::balance(0);
    shown_with(&mut kit, unread, "dashes", |k| {
        shows(k, "Platform address: —") && !shows(k, &format!(": {zero}"))
    });
    // Coming from the Core row, the unread address cannot be taken.
    kit.state_mut().pool.from = 2;
    kit.run_steps(2);
    unread(kit.state_mut().snapshot.as_mut().expect("a snapshot"));
    kit.run_steps(1);
    kit.get_by_role_and_label(Role::ComboBox, "Platform address: —")
        .click();
    kit.run_steps(2);
    assert_eq!(kit.state().pool.from, 2, "the unread row is not taken");
}
