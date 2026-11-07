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
