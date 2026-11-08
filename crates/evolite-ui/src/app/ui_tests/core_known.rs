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
    for _ in 0..50 {
        if let Some(s) = kit.state_mut().snapshot.as_mut() {
            change(s);
        }
        kit.run_steps(1);
        if seen(kit) {
            return;
        }
    }
    panic!("not shown: {what}");
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
