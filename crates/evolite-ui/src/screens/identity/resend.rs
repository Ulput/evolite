//! FEAT-0020-resend on the screens (the analyst's R-1…R-11 in BUG-231): before the password of
//! «Replace the DashPay pair», what the requests sent again will cost (R-1–R-3); on the keys
//! screen, the batch running and what is still owed (R-5, R-6, R-8); and the window «Send
//! requests again», for every contact owed one or for one from its card (R-8, R-9). No names
//! in any of it but the contact's own card (AC-A6).

use super::*;
use crate::components::{Part, dialog_with_summary, error_notice};
use crate::screens::txt;
use evolite_api::{ContactState, Duffs, Resend, Spend, Wallet};

/// «Send requests again»: for every contact owed one (`contact: None`) or for one. Its spend
/// goes through [`SpendFlow`] (TECH-spend-flow): its password, sent once, its own answer only.
pub struct ResendWindow {
    pub(crate) contact: Option<String>,
    pub(crate) flow: crate::components::spend_flow::SpendFlow<()>,
}

impl ResendWindow {
    pub fn new(contact: Option<String>) -> Self {
        Self {
            contact,
            flow: Default::default(),
        }
    }

    /// The core's answer to `Spend::ResendRequests`: taken only when it carries this window's
    /// `ask` (another window's, another wallet's is left). Whether it was this window's.
    pub fn on_done(&mut self, ask: Option<u64>, result: &Result<(), CoreError>) -> bool {
        self.flow.on_done(ask, result)
    }
}

/// The established contacts of the shown identity, and how many of them are this wallet's
/// own identities (decision 5).
fn contacts(w: &Wallet) -> (usize, usize) {
    let established: Vec<&str> = w
        .contacts
        .iter()
        .filter(|c| c.state == ContactState::Established)
        .map(|c| c.identity_id.as_str())
        .collect();
    let own = established
        .iter()
        .filter(|c| w.identities.iter().any(|i| i.id == **c))
        .count();
    (established.len(), own)
}

/// How many of `n` requests at `each` the credits cover after `first`; `None` when they cover
/// all of them or are not checked.
fn covered(w: &Wallet, first: Duffs, each: Duffs, n: usize) -> Option<usize> {
    let credits = &w.balances;
    let all = first.saturating_add(each.saturating_mul(n as Duffs));
    (credits.credits_checked && credits.credits >= first && credits.credits < all).then(|| {
        let left = credits.credits - first;
        (left / each.max(1)) as usize
    })
}

/// R-1–R-3, under the fee of «Replace the DashPay pair» (the change's own `change_fee`):
/// nothing when the identity has no established contacts.
pub(super) fn before_password(ui: &mut Ui, a: &mut Assets, w: &Wallet, change_fee: Duffs) {
    let p = a.p;
    let (n, own) = contacts(w);
    if n == 0 {
        return;
    }
    let each = w.fees.contact_request;
    let ours = each.saturating_mul(n as Duffs);
    ui.add(
        crate::components::para(
            tr!(
                "After the change, evolite will send new contact requests to your contacts, so they can pay you again."
            ),
            13.0,
            Weight::Regular,
            p.muted,
        )
        .line_height(18.0),
    );
    let line = |ui: &mut Ui, words: String| {
        ui.label(txt(&words, 13.0, Weight::Regular, p.text));
    };
    line(ui, format!(tr!("Contacts: {}"), n));
    line(
        ui,
        format!(
            tr!("Fee for the requests ≈ {}"),
            crate::format::amount(ours)
        ),
    );
    line(
        ui,
        format!(
            tr!("Total with the key change ≈ {}"),
            crate::format::amount(change_fee.saturating_add(ours))
        ),
    );
    // R-2: their requests to us are paid by them.
    if own > 0 {
        line(
            ui,
            format!(
                tr!(
                    "Of them your own identities: {}. Requests from them to you are sent too, from their credits."
                ),
                own
            ),
        );
        line(
            ui,
            format!(
                tr!("From the credits of other identities ≈ {}"),
                crate::format::amount(each.saturating_mul(own as Duffs))
            ),
        );
    }
    // R-3: the change is not refused for the requests.
    if let Some(k) = covered(w, change_fee, each, n) {
        notice(
            ui,
            a,
            "warning",
            p.accent,
            &format!(
                tr!(
                    "There are credits for the key change and {} of {} requests. Send the rest after a top-up with “Send requests again” on the keys screen."
                ),
                k, n
            ),
        );
    }
}

/// R-5, R-6, R-8 on the keys screen. Whether «Send requests again» was clicked.
pub(super) fn on_keys(ui: &mut Ui, a: &mut Assets, w: &Wallet) -> bool {
    let p = a.p;
    let Some(r) = w.resend.as_ref() else {
        return false;
    };
    if let Some((i, n)) = r.running {
        ui.label(txt(
            &format!(tr!("Sending contact requests again: {} of {}"), i, n),
            13.0,
            Weight::Medium,
            p.text,
        ));
        return false;
    }
    if let Some((sent, n)) = r.done.filter(|_| r.due.is_empty()) {
        notice(
            ui,
            a,
            "check",
            p.accent,
            &format!(tr!("Contact requests sent again: {} of {}"), sent, n),
        );
    }
    if !r.blocked.is_empty() {
        notice(ui, a, "warning", p.danger, blocked_words());
    }
    if r.due.is_empty() {
        return false;
    }
    notice(
        ui,
        a,
        "warning",
        p.accent,
        &format!(
            tr!(
                "Not every contact got a new request — {} left. Until it is sent, they cannot pay you."
            ),
            r.due.len()
        ),
    );
    button(
        ui,
        a,
        &format!(tr!("Send requests again ({})"), r.due.len()),
        ButtonKind::Primary,
        None,
    )
    .clicked()
}

/// R-11, for a contact no request can go to.
pub(crate) fn blocked_words() -> &'static str {
    tr!(
        "A new request cannot be sent to this contact: the limit of repeated requests is used up. They will not be able to pay you by DashPay — give them a Dash address."
    )
}

/// Whether `contact` waits for a request from the shown identity (R-9).
pub(crate) fn waits(r: Option<&Resend>, contact: &str) -> bool {
    r.is_some_and(|r| r.due.iter().any(|c| c == contact))
}

/// Whether no request can go to `contact` (R-11).
pub(crate) fn blocked(r: Option<&Resend>, contact: &str) -> bool {
    r.is_some_and(|r| r.blocked.iter().any(|c| c == contact))
}

/// The window «Send requests again» in `slot`, for the shown identity `identity`: how many,
/// the fee, the password, «Sign and send». Closed on «Cancel», Esc, or once it is done.
pub(crate) fn window(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &mut Ctx,
    slot: &mut Option<ResendWindow>,
    identity: &str,
) {
    let Some(win) = slot.as_mut() else {
        return;
    };
    if matches!(win.flow.result(), Some(Ok(()))) {
        *slot = None;
        return;
    }
    let w = cx.wallet().clone();
    let m = match &win.contact {
        Some(_) => 1,
        None => w.resend.as_ref().map_or(0, |r| r.due.len()),
    };
    let each = w.fees.contact_request;
    let fee = each.saturating_mul(m as Duffs);
    let title = match win.contact {
        Some(_) => tr!("Send the request again"),
        None => tr!("Send requests again"),
    };
    let closed = dialog_with_summary(ui, a, "resend", "contacts", title, 440.0, |ui, a, part| {
        let p = a.p;
        // BUG-249: what is agreed to, pinned over the password: how many requests and their fee.
        if part == Part::Summary {
            ui.spacing_mut().item_spacing.y = 10.0;
            ui.label(txt(
                &format!(tr!("Contacts: {}"), m),
                13.0,
                Weight::Regular,
                p.text,
            ));
            ui.label(txt(
                &format!(tr!("Fee for the requests ≈ {}"), crate::format::amount(fee)),
                13.0,
                Weight::Regular,
                p.text,
            ));
            // FEAT-0004 FR-7: the large-spend tick with what it agrees to.
            win.flow.password.summary_tick(ui, &p);
            return false;
        }
        if part == Part::Footer {
            let mut close = false;
            win.flow.password.field(ui, &p, "resend-password", true);
            if let Some(Err(e)) = win.flow.result().filter(|_| !win.flow.password.is_wrong()) {
                error_notice(
                    ui,
                    a,
                    &text::error_text(e),
                    text::error_details(e).as_deref(),
                );
            }
            let ready = m > 0 && win.flow.password.ready(true) && win.flow.can_act();
            let mut go = false;
            button_row(ui, |ui| {
                if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                    close = true;
                }
                go = crate::components::button_enabled(
                    ui,
                    a,
                    tr!("Sign and send"),
                    ButtonKind::Primary,
                    None,
                    ready,
                )
                .clicked();
            });
            if go {
                let op = Spend::ResendRequests {
                    identity: identity.to_owned(),
                    contact: win.contact.clone(),
                };
                // Sent once: a second press while it waits sends nothing.
                if let Some(command) = win.flow.command(op, true, ()) {
                    cx.send(command);
                }
            }
            return close;
        }
        ui.spacing_mut().item_spacing.y = 10.0;
        if let Some(k) = covered(&w, 0, each, m) {
            notice(
                ui,
                a,
                "warning",
                p.accent,
                &format!(tr!("There are credits for {} of {} requests."), k, m),
            );
        }
        false
    });
    if closed {
        *slot = None;
    }
}
