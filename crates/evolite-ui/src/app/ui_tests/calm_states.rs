//! Calm states CS-1 (designer 2026-11-07): red only for errors — what is missing before anything
//! was tried is muted and has its way out; a list that did not refresh says the saved one stays.

use super::*;
use evolite_api::{NftsStatus, Snapshot, Wallet};

fn wallet(s: &mut Snapshot) -> &mut Wallet {
    s.wallet.as_mut().expect("a wallet")
}

/// A label or value with `text` in it.
fn shows(kit: &Kit, text: &str) -> bool {
    kit.query_all_by(|n| {
        n.label().is_some_and(|l| l.contains(text)) || n.value().is_some_and(|v| v.contains(text))
    })
    .next()
    .is_some()
}

/// Puts `change` on every snapshot and draws until `seen` holds on the same frame.
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

/// CS-1 №5: «Add a name» with too few credits says what is needed and what there is — not the
/// old red «top up the identity» — and «Top up ›» takes the person to the top-up.
#[test]
fn add_name_short_of_credits_offers_the_top_up() {
    let (_turn, mut kit) = demo_app("add-name", "ru");
    let no_credits = |s: &mut Snapshot| wallet(s).balances.credits = 0;
    shown_with(&mut kit, no_credits, "what the name needs", |k| {
        shows(k, "Кредитов на имя не хватает: нужно ≈")
            && !shows(k, "Кредитов не хватает: пополните identity.")
    });
    kit.get_by_role_and_label(Role::Button, "Пополнить ›")
        .click();
    shown_with(&mut kit, no_credits, "the name dialog closed", |k| {
        !shows(k, "Кредитов на имя не хватает")
    });
    shown_with(&mut kit, no_credits, "the top-up", |k| {
        has_text(k, "Пополнить identity")
    });
}

/// CS-1 №8: a list of NFTs that did not refresh says so in a muted line, and that the saved NFTs
/// are still there.
#[test]
fn a_collection_not_refreshed_keeps_the_saved_list() {
    let (_turn, mut kit) = demo_app("collection", "ru");
    shown_with(
        &mut kit,
        |s| wallet(s).nfts_status = NftsStatus::Failed("timeout".into()),
        "the saved list stays",
        |k| shows(k, "(timeout). Сохранённые NFT на месте."),
    );
}

/// CS-4.2: the private pool not read yet shows «—», never «0,0000», with a neutral line — while
/// it connects and before it is asked to; the store's amount from before is shown dim.
#[test]
fn the_pool_waiting_to_connect_shows_a_dash_not_a_zero() {
    let (_turn, mut kit) = demo_app("shielded", "en");
    let zero = crate::format::amount(0);
    let dash = format!("— {}", crate::format::unit_name());
    let waiting = |connecting: bool| {
        move |s: &mut Snapshot| {
            let w = wallet(s);
            w.pool.checked_at = None;
            w.pool.address = None;
            w.pool.connecting = connecting;
            w.balances.shielded = 0;
        }
    };
    shown_with(&mut kit, waiting(true), "the pool connecting", |k| {
        has_text(k, &dash) && shows(k, "Connecting the pool") && !shows(k, &zero)
    });
    shown_with(&mut kit, waiting(false), "the pool not asked yet", |k| {
        has_text(k, &dash)
            && has_text(
                k,
                "The pool connects after syncing with Platform — usually a minute",
            )
            && !shows(k, "The pool is not connected yet")
            && !shows(k, &zero)
    });
    let stored = 48_885_860;
    let amount = crate::format::amount(stored);
    shown_with(
        &mut kit,
        |s| {
            waiting(true)(s);
            wallet(s).balances.shielded = stored;
        },
        "the stored amount",
        |k| has_text(k, &amount),
    );
}

/// CS-1a: «Try again» under a list that did not refresh asks the core now; the demo core's
/// answer is the list as good as new, and the line goes. The failure is put on the snapshot the
/// app shows once (the demo core sends a new one only when something changed).
#[test]
fn try_again_asks_for_the_nfts_now() {
    let (_turn, mut kit) = demo_app("collection", "ru");
    let failed = |s: &mut Snapshot| wallet(s).nfts_status = NftsStatus::Failed("timeout".into());
    shown_with(&mut kit, failed, "the line and its button", |k| {
        shows(k, "Сохранённые NFT на месте.")
    });
    kit.get_by_role_and_label(Role::Button, "Попробовать снова")
        .click();
    wait_for(&mut kit, "the refreshed list", |k| {
        !shows(k, "Сохранённые NFT на месте.")
    });
    assert_eq!(
        kit.state()
            .snapshot
            .as_ref()
            .and_then(|s| s.wallet.as_ref())
            .map(|w| &w.nfts_status),
        Some(&NftsStatus::Ready)
    );
}

/// CS-1a r2 should 1: an empty list after a failed load has «Try again» in its card; it asks
/// the core now, and the demo core's answer is the list. The failure is put on the snapshot the
/// app shows until seen (the demo core sends a new one only when something changed).
#[test]
fn try_again_on_an_empty_list_asks_for_the_nfts_now() {
    let (_turn, mut kit) = demo_app("collection", "ru");
    let failed = |s: &mut Snapshot| {
        let w = wallet(s);
        w.nfts.clear();
        w.nfts_status = NftsStatus::Failed("timeout".into());
    };
    shown_with(&mut kit, failed, "the empty card and its button", |k| {
        has_text(k, "Не удалось загрузить NFT")
    });
    kit.get_by_role_and_label(Role::Button, "Попробовать снова")
        .click();
    wait_for(&mut kit, "the list again", |k| {
        !has_text(k, "Не удалось загрузить NFT")
            && k.state()
                .snapshot
                .as_ref()
                .and_then(|s| s.wallet.as_ref())
                .is_some_and(|w| !w.nfts.is_empty() && w.nfts_status == NftsStatus::Ready)
    });
}
