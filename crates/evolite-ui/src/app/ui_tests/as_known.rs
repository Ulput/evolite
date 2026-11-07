//! What the screens say they know (batch 2026-11-07): the private pool before its first read
//! (BUG-66), the date under the recovery phrase of a restored wallet (BUG-59).

use super::*;
use evolite_api::{Wallet, WalletOrigin};

/// A label or value with `text` in it.
fn shows(kit: &Kit, text: &str) -> bool {
    kit.query_all_by(|n| {
        n.label().is_some_and(|l| l.contains(text)) || n.value().is_some_and(|v| v.contains(text))
    })
    .next()
    .is_some()
}

/// Changes the shown wallet with `change` and draws until `seen` holds on the same frame: the
/// demo core's own snapshots may come first and put theirs back.
fn shown_with(
    kit: &mut Kit,
    change: impl Fn(&mut evolite_api::Snapshot),
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

fn wallet(s: &mut evolite_api::Snapshot) -> &mut Wallet {
    s.wallet.as_mut().expect("a wallet")
}

/// BUG-66: right after the login the pool is not read yet — its tile says «Ещё не знаем» with
/// «—», never «0,0000», and «Total» says it does not count the pool; an amount the pool's store
/// already has is shown dim as not checked. Once read, the tile and «Total» are as before.
#[test]
fn the_pool_before_its_first_read_is_not_known_not_zero() {
    let (_turn, mut kit) = demo_app("overview", "ru");
    let zero = crate::format::number(0, false);
    shown_with(
        &mut kit,
        |s| {
            let w = wallet(s);
            w.pool.checked_at = None;
            w.balances.shielded = 0;
        },
        "the pool not known yet",
        |k| {
            has_text(k, "Ещё не знаем")
                && has_text(k, "—")
                && has_text(k, "Без приватного пула: он ещё читается")
                && !has_text(k, &zero)
        },
    );
    // The store's amount before the first read: dim, «not checked yet», counted in «Total».
    let stored = 48_885_860;
    let amount = crate::format::number(stored, false);
    shown_with(
        &mut kit,
        |s| {
            let w = wallet(s);
            w.pool.checked_at = None;
            w.balances.shielded = stored;
        },
        "the stored pool amount",
        |k| {
            shows(k, &amount)
                && shows(k, "DASH · ещё не проверено")
                && has_text(k, "Приватный пул в нём ещё не проверен")
                && !has_text(k, "Ещё не знаем")
        },
    );
    // Read: as before the fix.
    let now = chrono::Local::now().timestamp();
    shown_with(
        &mut kit,
        |s| {
            let w = wallet(s);
            w.pool.checked_at = Some(now);
            w.balances.shielded = stored;
        },
        "the read pool",
        |k| {
            shows(k, &amount)
                && shows(k, "DASH · видите только вы")
                && !shows(k, "ещё не проверено")
                && !shows(k, "приватного пула")
                && !shows(k, "Приватный пул в нём")
        },
    );
}

/// BUG-66, the neighbour: a pool read and empty is a plain 0 — «Ещё не знаем» only before
/// the read, and the other tiles keep their amounts all along.
#[test]
fn a_read_empty_pool_is_zero_and_the_other_tiles_stay() {
    let (_turn, mut kit) = demo_app("overview", "en");
    let (core, credits) = {
        let w = kit
            .state()
            .snapshot
            .as_ref()
            .and_then(|s| s.wallet.as_ref())
            .expect("a wallet");
        (w.balances.core, w.balances.all_credits)
    };
    let core = crate::format::number(core, false);
    let credits = crate::format::number(credits, false);
    shown_with(
        &mut kit,
        |s| {
            let w = wallet(s);
            w.pool.checked_at = None;
            w.balances.shielded = 0;
        },
        "not known yet, the others shown",
        |k| has_text(k, "Not known yet") && shows(k, &core) && shows(k, &credits),
    );
    let now = chrono::Local::now().timestamp();
    shown_with(
        &mut kit,
        |s| {
            let w = wallet(s);
            w.pool.checked_at = Some(now);
            w.balances.shielded = 0;
        },
        "a read empty pool",
        |k| {
            !has_text(k, "Not known yet")
                && shows(k, "DASH · only you see it")
                && has_text(k, &crate::format::number(0, false))
        },
    );
}

/// BUG-59: Backup of a restored wallet says when it was restored here, not «создана» — the
/// phrase was made before, on a day the wallet cannot know; a wallet made here still says
/// «создана».
#[test]
fn backup_of_a_restored_wallet_says_restored_not_made() {
    let (_turn, mut kit) = demo_app("backup", "ru");
    wait_for(&mut kit, "the phrase line", |k| shows(k, "слов · создана"));
    let origin = |o: WalletOrigin| {
        move |s: &mut evolite_api::Snapshot| {
            if let Some(w) = s.wallets.iter_mut().find(|w| w.open) {
                w.origin = o;
            }
        }
    };
    shown_with(
        &mut kit,
        origin(WalletOrigin::Restored),
        "restored on this computer",
        |k| shows(k, "слов · восстановлена на этом компьютере") && !shows(k, "слов · создана"),
    );
    shown_with(
        &mut kit,
        origin(WalletOrigin::DashPay),
        "imported from DashPay",
        |k| shows(k, "слов · перенесена из DashPay"),
    );
    shown_with(&mut kit, origin(WalletOrigin::Created), "made here", |k| {
        shows(k, "слов · создана")
    });
}

/// BUG-66 with BUG-77: Core and the pool both unchecked — «Всего» has one line for both, not
/// two; Core not known at all — «Всего» is «—» with Core's «Ещё не знаем» only.
#[test]
fn total_says_core_and_the_pool_in_one_line() {
    use evolite_api::CoreAsOf;
    let (_turn, mut kit) = demo_app("overview", "ru");
    let at = chrono::Local::now().timestamp() - 120;
    let core =
        crate::text::core_as_of(Some(CoreAsOf::Checked(at)), chrono::Local::now()).expect("a line");
    let both = format!("{core} · без приватного пула: он ещё читается");
    let lines = |k: &Kit, s: &str| {
        k.query_all_by(|n| n.role() == Role::Label && n.value().is_some_and(|v| v.contains(s)))
            .count()
    };
    shown_with(
        &mut kit,
        |s| {
            let w = wallet(s);
            w.balances.core_as_of = Some(CoreAsOf::Checked(at));
            w.pool.checked_at = None;
            w.balances.shielded = 0;
        },
        "one line for Core and the pool",
        |k| has_text(k, &both) && !has_text(k, "Без приватного пула: он ещё читается"),
    );
    assert_eq!(lines(&kit, "приватного пула"), 1, "one line about the pool");
    shown_with(
        &mut kit,
        |s| {
            let w = wallet(s);
            w.balances.core_as_of = Some(CoreAsOf::NotYet);
            w.pool.checked_at = None;
            w.balances.shielded = 0;
        },
        "Core not known: only its line",
        |k| has_text(k, "Ещё не знаем") && !shows(k, "приватного пула"),
    );
}
