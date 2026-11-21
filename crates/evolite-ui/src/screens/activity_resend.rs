//! FEAT-0046: on the card of a payment the network has not confirmed — what that means in plain
//! words (R-3, R-3a, R-7) and «Broadcast again» (R-2): the same signed bytes to the nodes once
//! more, no password, nothing new spent. The button is behind the advanced switch (UC-4); the
//! words are for everyone.

use crate::components::{Assets, ButtonKind, button_enabled, notice};
use evolite_api::{Activity, Chain, Command, CoreError, Detail, Proof, Timestamp};

/// R-6: how long after a broadcast again the button waits, seconds (the core's own rule too).
const AGAIN_AFTER: Timestamp = 60;

/// The answer of the last «Broadcast again» of the card open, kept in the context.
#[derive(Clone, Default)]
struct Asked {
    txid: String,
    waiting: bool,
    refused: Option<String>,
}

fn asked_id() -> egui::Id {
    egui::Id::new("activity-resend")
}

/// The core's answer to `Command::Rebroadcast`.
pub fn on_resend_done(ctx: &egui::Context, result: &Result<(), CoreError>) {
    ctx.data_mut(|d| {
        if let Some(asked) = d
            .get_temp_mut_or_default::<Option<Asked>>(asked_id())
            .as_mut()
        {
            asked.waiting = false;
            asked.refused = result.as_ref().err().map(|e| match e {
                CoreError::NotAvailable(why) | CoreError::Failed(why) => why.clone(),
                other => format!("{other:?}"),
            });
        }
    });
}

/// Whether the card of `act` says anything of this: a Core payment of ours not confirmed, or
/// one broadcast again from here and still waiting.
pub fn shown(act: &Activity, detail: &Detail) -> bool {
    act.chain == Chain::Core
        && (act.proof == Proof::Unconfirmed
            || (detail.resent_at.is_some() && act.proof == Proof::Mempool))
}

/// The plate and the button; the command to send when it is pressed.
pub fn plate(
    ui: &mut egui::Ui,
    a: &mut Assets,
    act: &Activity,
    detail: &Detail,
    advanced: bool,
    now: Timestamp,
) -> Option<Command> {
    if !shown(act, detail) {
        return None;
    }
    let p = a.p;
    let txid = act.txid.clone().unwrap_or_default();
    let asked = ui
        .ctx()
        .data(|d| d.get_temp::<Option<Asked>>(asked_id()))
        .flatten()
        .filter(|asked| asked.txid == txid);
    let mut press = None;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        if act.proof == Proof::Unconfirmed {
            let can = advanced && detail.resendable && !txid.is_empty();
            let words = if detail.resent_at.is_some() {
                // R-7: 12 blocks after a broadcast again, still nothing.
                crate::i18n::t(STILL_NOT).into_owned()
            } else {
                let mut words = crate::i18n::t(LONG_UNCONFIRMED).into_owned();
                if can {
                    words.push(' ');
                    words.push_str(&crate::i18n::t(tr!(
                        "evolite already passes it on again every 10 minutes while it is open."
                    )));
                    words.push(' ');
                    words.push_str(&crate::i18n::t(tr!("You can broadcast it again.")));
                }
                words
            };
            notice(ui, a, "warning", p.accent, &words);
            if can {
                let waiting = asked.as_ref().is_some_and(|q| q.waiting);
                let left = detail
                    .resent_at
                    .map(|at| AGAIN_AFTER - now.saturating_sub(at))
                    .filter(|left| *left > 0);
                let label =
                    match (waiting, left) {
                        (true, _) => crate::i18n::t(tr!("Broadcasting…")).into_owned(),
                        (false, Some(left)) => crate::i18n::t(tr!("You can again in {} s"))
                            .replacen("{}", &left.to_string(), 1),
                        (false, None) => crate::i18n::t(tr!("Broadcast again")).into_owned(),
                    };
                let ready = !waiting && left.is_none();
                if button_enabled(ui, a, &label, ButtonKind::Secondary, None, ready).clicked()
                    && ready
                {
                    press = Some(Command::Rebroadcast { txid: txid.clone() });
                }
                if left.is_some() {
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_secs(1));
                }
            }
        }
        if let Some(at) = detail.resent_at {
            // R-10: when, after a restart too.
            let when = crate::text::time_of_day(at, chrono::Local::now());
            ui.label(crate::components::txt(
                &crate::i18n::t(tr!("Broadcast again {}")).replacen("{}", &when, 1),
                12.0,
                crate::theme::Weight::Regular,
                p.muted,
            ));
        }
        if let Some(why) = asked.as_ref().and_then(|q| q.refused.as_deref()) {
            ui.label(crate::components::txt(
                &crate::i18n::t(why),
                12.0,
                crate::theme::Weight::Regular,
                p.danger,
            ));
        }
    });
    if press.is_some() {
        ui.ctx().data_mut(|d| {
            d.insert_temp(
                asked_id(),
                Some(Asked {
                    txid,
                    waiting: true,
                    refused: None,
                }),
            )
        });
    }
    press
}

/// R-3: what a payment not confirmed for long means (also the status chip's tooltip).
pub const LONG_UNCONFIRMED: &str = tr!(
    "The payment has not been confirmed for a long time. With InstantSend this almost never happens: most likely the nodes did not take it."
);

/// R-7: still not confirmed 12 blocks after a broadcast again.
const STILL_NOT: &str = tr!(
    "The payment has still not been confirmed. Its coins may already have been spent by another payment — for example, with this phrase in another wallet. Then this payment will not go through: check the balance and history there."
);
