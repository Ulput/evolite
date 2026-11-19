//! FEAT-0020: changing the identity's keys from K1 — disable a key (the designer's K3), DashPay's
//! pair replaced (K3b), the main key replaced (K4), a key added for a ready purpose (K5). What
//! it does is said before the password; what cannot be done is refused before it, by the same
//! plan the core makes ([`key_plan::plan`]).

use super::*;
use crate::components::{Part, button_row_destructive, dialog_with_footer, error_notice};
use crate::screens::txt;
use evolite_api::key_plan::{self, KeyRequest};
use evolite_api::{IdentityKey, KeyLevel, KeyOwner, KeyPurpose};

/// Which change the dialog makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    /// K3: disable key N.
    Disable(u32),
    /// K4.
    ReplaceMaster,
    /// K3b: DashPay's encryption keys replaced as a pair.
    ReplaceDashPay,
    /// K5: a key for a ready purpose, chosen in the dialog.
    Add,
}

/// One key change, from its dialog to the core's answer.
pub struct KeyChange {
    pub(crate) kind: ChangeKind,
    /// K5's choice.
    pub(crate) chosen: Option<KeyRequest>,
    pub(crate) password: SpendPassword,
    /// AC-T19: «I understand: a disabled key cannot be brought back» — asked when DashPay's pair
    /// would also disable another's encryption keys.
    pub(crate) understood: bool,
    pub(crate) waiting: bool,
    /// This dialog has sent its change once (BUG-184): the plan's refusal is no answer since.
    pub(crate) sent_once: bool,
    pub(crate) result: Option<Result<(), CoreError>>,
}

impl KeyChange {
    pub fn new(kind: ChangeKind) -> Self {
        Self {
            kind,
            chosen: None,
            password: SpendPassword::default(),
            understood: false,
            waiting: false,
            sent_once: false,
            result: None,
        }
    }

    /// K5 with `request` chosen (UC-6, K7 «Enable DashPay»).
    pub fn adding(request: KeyRequest) -> Self {
        let mut change = Self::new(ChangeKind::Add);
        change.chosen = Some(request);
        change
    }

    /// What it asks the core for: the keys added and those disabled. DashPay's pair takes
    /// another's encryption keys with it (AC-T19): DashPay would pick them first; the dialog
    /// sends it only after «I understand».
    pub(crate) fn asked(&self, keys: &[IdentityKey]) -> (Vec<KeyRequest>, Vec<u32>) {
        let (add, mut disable) = match self.kind {
            ChangeKind::Disable(id) => (Vec::new(), vec![id]),
            ChangeKind::ReplaceMaster => (vec![KeyRequest::ReplaceMaster], Vec::new()),
            ChangeKind::ReplaceDashPay => (vec![KeyRequest::DashPay], Vec::new()),
            ChangeKind::Add => (self.chosen.into_iter().collect(), Vec::new()),
        };
        if add.contains(&KeyRequest::DashPay) {
            disable.extend(key_plan::foreign_encryption(keys));
        }
        (add, disable)
    }

    /// The core's answer: a wrong password stays for a correction.
    pub(crate) fn on_changed(&mut self, result: Result<(), CoreError>) {
        if !self.waiting {
            return;
        }
        self.waiting = false;
        self.password.on_answer(&result);
        // A wrong password sent nothing: the form stays, for the right one (R-6).
        if matches!(result, Err(CoreError::WrongPassword)) {
            self.sent_once = false;
        }
        self.result = Some(result);
    }

    /// BUG-200: this dialog sent its change and the core answered — whatever it said, the
    /// dialog offers no second change: only «Close» and «View the keys». A plan worked out again
    /// on a snapshot that already has the change (the main key now the new one) would be a
    /// second paid change nobody asked for.
    pub(crate) fn answered(&self) -> bool {
        self.sent_once && !self.waiting && self.result.is_some()
    }
}

/// The answer to a change of keys, in words: a failure says the keys did not change; a change
/// not known to have gone through (TECH-platform-errors, audit Д4) says only that — never
/// «did not change», which it may have.
fn said(e: &CoreError) -> String {
    match e {
        CoreError::Unclear | CoreError::UnclearWhich => text::error_text(e),
        _ => text::failed(tr!("The keys did not change"), e),
    }
}

/// What stops working when `key` is disabled (R-3), in words.
pub(crate) fn stops(key: &IdentityKey) -> String {
    // AC-T19: an encryption key not known to be DashPay's may be another wallet's or an app's data.
    if key.purpose == KeyPurpose::Encryption && !key_plan::is_dashpay(key) {
        return tr!(
            "It may have been added by another wallet or an app — that one may no longer decrypt its data"
        )
        .into();
    }
    match (&key.owner, &key.purpose, key.level) {
        (KeyOwner::App(app), ..) => format!(
            tr!(
                "Signing in to {} and to the sites connected with this key — they need to be connected again"
            ),
            app
        ),
        (_, KeyPurpose::Documents, KeyLevel::High) => tr!(
            "Signing in to apps and sites connected with this key — they need to be connected again"
        )
        .into(),
        (_, KeyPurpose::Documents, KeyLevel::Critical) => {
            tr!("Sending and managing tokens — until you add a new key for tokens").into()
        }
        (_, KeyPurpose::Transfers, _) => {
            tr!("Transferring credits to an address and into the pool").into()
        }
        _ => tr!("Nothing else: transfers, DashPay and tokens use other keys").into(),
    }
}

/// K5's ready purposes: title and what it is for.
fn purpose_words(request: KeyRequest) -> (&'static str, &'static str) {
    match request {
        KeyRequest::Tokens => (
            tr!("For tokens"),
            tr!("Important actions · CRITICAL. Needed to send and manage tokens"),
        ),
        KeyRequest::DashPay => (
            tr!("For DashPay"),
            tr!("A pair: encryption and decryption, bound to DashPay"),
        ),
        KeyRequest::Transfers => (tr!("For credit transfers"), "TRANSFER · CRITICAL"),
        KeyRequest::ReplaceMaster => (tr!("Replace the main key"), "MASTER"),
    }
}

/// The dialog of the change in `slot`, over K1, for identity `id` named `me`. Closed on
/// «Cancel» or «Close», on Esc, and when the core says it is done.
pub(super) fn change_dialog(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &mut Ctx,
    slot: &mut Option<KeyChange>,
    id: &Identity,
    me: &str,
) -> bool {
    let Some(change) = slot.as_mut() else {
        return false;
    };
    if matches!(change.result, Some(Ok(()))) {
        *slot = None;
        return false;
    }
    let (add, disable) = change.asked(&id.keys);
    // AC-T19: another's encryption keys go with the pair only after «I understand».
    let foreign: Vec<u32> = if add.contains(&KeyRequest::DashPay) {
        key_plan::foreign_encryption(&id.keys)
    } else {
        Vec::new()
    };
    let plan = key_plan::plan(&id.keys, &add, &disable);
    // BUG-184 p. 3: the plan is refused on the snapshot only before this dialog's first send.
    // After it, a refusal means the snapshot shows the change (the key disabled, the keys there):
    // done, whatever the answer could tell.
    if change.sent_once
        && !change.waiting
        && matches!(&plan, Err(r) if *r != key_plan::KeyRefusal::Nothing)
    {
        *slot = None;
        return true;
    }
    // A refusal is said before the password; «nothing chosen yet» is not one.
    let refusal = match &plan {
        Err(key_plan::KeyRefusal::Nothing) | Ok(_) => None,
        Err(r) => Some(r.clone()),
    };
    let fee = key_plan::fee(&add, &cx.wallet().fees);
    // BUG-183 AC-T4: credits short of the fee are said before the password (BUG-108's estimate,
    // never below what testnet charged).
    let credits = &cx.wallet().balances;
    let credits_short = (plan.is_ok() && credits.credits_checked && fee > credits.credits)
        .then_some(credits.credits);
    let (title, icon, act, danger) = match change.kind {
        ChangeKind::Disable(n) => (
            format!(tr!("Disable key {}?"), n),
            "warning",
            tr!("Disable key"),
            true,
        ),
        ChangeKind::ReplaceMaster => (
            tr!("Replace the main key").to_string(),
            "key",
            tr!("Replace key"),
            false,
        ),
        ChangeKind::ReplaceDashPay => (
            tr!("Replace the DashPay pair").to_string(),
            "key",
            tr!("Replace the pair"),
            false,
        ),
        ChangeKind::Add => (tr!("Add a key").to_string(), "key", tr!("Add key"), false),
    };
    let mut switch_to: Option<ChangeKind> = None;
    let closed = dialog_with_footer(ui, a, "key-change", icon, &title, 480.0, |ui, a, part| {
        let p = a.p;
        if part == Part::Footer {
            let mut close = false;
            if change.answered() {
                button_row(ui, |ui| {
                    if button(ui, a, tr!("View the keys"), ButtonKind::Secondary, None).clicked() {
                        // K1 under the dialog, read from the network again.
                        cx.send(Command::ReadIdentityKeys {
                            identity: id.id.clone(),
                        });
                        close = true;
                    }
                    if button(ui, a, tr!("Close"), ButtonKind::Primary, None).clicked() {
                        close = true;
                    }
                });
                return close;
            }
            if refusal.is_some() {
                button_row(ui, |ui| {
                    if matches!(refusal, Some(key_plan::KeyRefusal::DashPayOnlyReplaced))
                        && button(
                            ui,
                            a,
                            tr!("Replace the DashPay pair…"),
                            ButtonKind::Secondary,
                            None,
                        )
                        .clicked()
                    {
                        switch_to = Some(ChangeKind::ReplaceDashPay);
                    }
                    if button(ui, a, tr!("Close"), ButtonKind::Primary, None).clicked() {
                        close = true;
                    }
                });
                return close;
            }
            // The password is asked for every key change (R-6).
            if !change.waiting {
                change.password.field(ui, &p, "key-change-password", true);
            }
            if let Some(Err(e)) = change
                .result
                .as_ref()
                .filter(|_| !change.password.is_wrong())
            {
                error_notice(ui, a, &said(e), text::error_details(e).as_deref());
            }
            let ready = plan.is_ok()
                && credits_short.is_none()
                && change.password.ready(true)
                && !change.waiting
                && (foreign.is_empty() || change.understood);
            let label = if change.waiting {
                tr!("Changing the keys…")
            } else {
                act
            };
            let mut go = false;
            let mut row = |ui: &mut Ui| {
                if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                    close = true;
                }
                let kind = if danger {
                    ButtonKind::Danger
                } else {
                    ButtonKind::Primary
                };
                go = button_enabled(ui, a, label, kind, None, ready).clicked();
            };
            if danger {
                button_row_destructive(ui, row);
            } else {
                button_row(ui, &mut row);
            }
            if go {
                let op = Spend::UpdateKeys {
                    identity: id.id.clone(),
                    add: add.clone(),
                    disable: disable.clone(),
                };
                cx.send(change.password.command(op, true));
                change.waiting = true;
                change.sent_once = true;
                change.result = None;
            }
            return close;
        }
        ui.spacing_mut().item_spacing.y = 10.0;
        // BUG-200: after the answer, the answer alone — no plan, no fee, nothing to sign.
        if change.answered() {
            if let Some(Err(e)) = &change.result {
                error_notice(ui, a, &said(e), text::error_details(e).as_deref());
            }
            return false;
        }
        if let Some(r) = &refusal {
            notice(
                ui,
                a,
                "warning",
                p.accent,
                &evolite_core::key_refusal::words(r),
            );
            return false;
        }
        match change.kind {
            ChangeKind::Disable(n) => {
                if let Some(key) = id.keys.iter().find(|k| k.id == n) {
                    let (purpose, _) = super::keys::key_words(key);
                    let which = format!(tr!("Key {}"), key.id);
                    ui.label(text(
                        &format!(
                            "{which} · {purpose} · {} · {}",
                            text::key_level(key.level),
                            super::keys::owner_words(&key.owner)
                        ),
                        BODY,
                        p.text,
                    ));
                    ui.label(txt(
                        tr!("What stops working"),
                        13.0,
                        Weight::SemiBold,
                        p.text,
                    ));
                    ui.add(
                        crate::components::para(&stops(key), 13.0, Weight::Regular, p.muted)
                            .line_height(18.0),
                    );
                }
                notice(
                    ui,
                    a,
                    "warning",
                    p.accent,
                    tr!(
                        "Disabling is for good: the key cannot be turned on again. If one is needed again, add a new key — it takes the next number."
                    ),
                );
            }
            ChangeKind::ReplaceMaster => {
                ui.add(
                    crate::components::para(
                    tr!(
                        "evolite derives the new main key from this wallet's phrase. The old one is disabled in the same update — an identity never has a second main key."
                    ),
                        13.0,
                        Weight::Regular,
                        p.muted,
                    )
                    .line_height(18.0),
                );
                if let Ok(plan) = &plan {
                    let old = plan.disable.first().copied().unwrap_or_default();
                    let new = plan.add.first().map_or(0, |k| k.id);
                    kv_row(
                        ui,
                        &p,
                        tr!("Is disabled"),
                        &format!(tr!("{} · main · MASTER"), old),
                        style(UI),
                    );
                    kv_row(
                        ui,
                        &p,
                        tr!("Is added"),
                        &format!(tr!("{} · main · MASTER · from the phrase"), new),
                        style(UI),
                    );
                    notice(
                        ui,
                        a,
                        "warning",
                        p.accent,
                        &format!(
                            tr!(
                                "From now on only the new key {} changes the identity's keys. The old key {} is disabled for good. If someone else has this wallet's phrase, replacing does not help: create a new wallet."
                            ),
                            new, old
                        ),
                    );
                }
            }
            ChangeKind::ReplaceDashPay => {
                ui.add(
                    crate::components::para(
                    tr!(
                        "DashPay's keys are replaced as a pair: the old ones are disabled and a new pair from this wallet's phrase is added in the same update."
                    ),
                        13.0,
                        Weight::Regular,
                        p.muted,
                    )
                    .line_height(18.0),
                );
                // What changes, as K4 says it (the designer's Must 5).
                if let Ok(plan) = &plan {
                    let list = |ids: Vec<u32>| {
                        ids.iter()
                            .map(u32::to_string)
                            .collect::<Vec<_>>()
                            .join(", ")
                    };
                    kv_row(
                        ui,
                        &p,
                        tr!("Is disabled"),
                        &list(plan.disable.clone()),
                        style(UI),
                    );
                    kv_row(
                        ui,
                        &p,
                        tr!("Is added"),
                        &format!(
                            tr!("{} · from the phrase"),
                            list(plan.add.iter().map(|k| k.id).collect())
                        ),
                        style(UI),
                    );
                }
                notice(
                    ui,
                    a,
                    "warning",
                    p.accent,
                    tr!(
                        "You will not be able to pay contacts whose requests point to the old key until they send you a new request."
                    ),
                );
            }
            ChangeKind::Add => {
                let next = id.keys.iter().map(|k| k.id + 1).max().unwrap_or(0);
                let said = format!(
                    tr!(
                        "evolite derives the key from this wallet's phrase with the next number — {}."
                    ),
                    next
                );
                ui.add(
                    crate::components::para(&said, 13.0, Weight::Regular, p.muted)
                        .line_height(18.0),
                );
                for request in [
                    KeyRequest::Tokens,
                    KeyRequest::DashPay,
                    KeyRequest::Transfers,
                ] {
                    let (name, about) = purpose_words(request);
                    let has = key_plan::has(&id.keys, request);
                    let chosen = change.chosen == Some(request) && !has;
                    // A taken one says so on a line of its own under its words (two translated
                    // pieces glued together would read wrong in Arabic).
                    let note = has.then_some(tr!("This identity has one already"));
                    let resp = purpose_row(ui, a, name, about, note, chosen);
                    if resp.clicked() && !has {
                        change.chosen = Some(request);
                        change.result = None;
                    }
                }
            }
        }
        // AC-T19: DashPay takes the lowest-numbered encryption key — another's, now. The pair
        // goes with those keys disabled, said and agreed to (R-4).
        if !foreign.is_empty() {
            let list = foreign
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            notice(
                ui,
                a,
                "warning",
                p.accent,
                &format!(
                    tr!(
                        "DashPay takes the encryption key with the lowest number. Now that is key {} of another wallet or an app — contacts would not work here. Enabling DashPay adds this wallet's pair and disables {}. That wallet or app may lose access to its encrypted data."
                    ),
                    foreign[0], list
                ),
            );
            crate::components::checkbox(
                ui,
                a,
                &mut change.understood,
                tr!("I understand: a disabled key cannot be brought back"),
            );
        }
        if plan.is_ok() {
            kv_row(
                ui,
                &p,
                tr!("Fee, in credits, roughly"),
                &format!("≈ {}", crate::format::amount(fee)),
                crate::theme::mono(13.0, Weight::Medium),
            );
            kv_row(
                ui,
                &p,
                tr!("Pays"),
                &format!(tr!("{} · credits"), me),
                style(UI),
            );
            if let Some(has) = credits_short {
                ui.label(text(
                    &format!(
                        tr!("Not enough credits: ≈ {} needed, {} has {}."),
                        crate::format::amount(fee),
                        crate::i18n::own(me),
                        crate::format::amount(has)
                    ),
                    SMALL,
                    p.danger,
                ));
            }
        }
        false
    });
    if let Some(kind) = switch_to {
        *slot = Some(KeyChange::new(kind));
    } else if closed {
        // The password goes with the dialog.
        *slot = None;
    }
    false
}

/// One of K5's ready purposes (the designer's frame): a radio, the title in Inter 13 SemiBold
/// and what it is for in Inter 12 muted. `note`: why it cannot be chosen — then it is dimmed.
fn purpose_row(
    ui: &mut Ui,
    a: &mut Assets,
    title: &str,
    about: &str,
    note: Option<&str>,
    chosen: bool,
) -> egui::Response {
    let p = a.p;
    let enabled = note.is_none();
    let width = ui.available_width();
    let words = width - 16.0 - 20.0 - 12.0 - 16.0;
    let small = crate::theme::sans(12.0, Weight::Regular);
    // A purpose that cannot be chosen: everything at about 45 % (the designer).
    let dim = |c: egui::Color32| if enabled { c } else { c.gamma_multiply(0.45) };
    let ink = dim(p.text);
    let muted = dim(p.muted);
    let layout = |s: &str, font: egui::FontId, color| {
        ui.painter().layout(
            crate::components::lines(ui.ctx(), s, &font, words),
            font,
            color,
            words,
        )
    };
    let mut lines = vec![
        layout(title, crate::theme::sans(13.0, Weight::SemiBold), ink),
        layout(about, small.clone(), muted),
    ];
    if let Some(note) = note {
        lines.push(layout(note, small, muted));
    }
    let gap = 2.0;
    let words_h: f32 =
        lines.iter().map(|g| g.size().y).sum::<f32>() + gap * (lines.len() - 1) as f32;
    let (rect, resp) = ui.allocate_exact_size(
        egui::vec2(width, 14.0 + words_h + 14.0),
        if enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    crate::components::named_choice(&resp, egui::WidgetType::RadioButton, chosen, title);
    ui.painter().rect(
        rect,
        CornerRadius::same(10),
        if chosen { p.raised } else { p.bg },
        egui::Stroke::new(1.0, if chosen { p.ink } else { p.line }),
        egui::StrokeKind::Inside,
    );
    let dot = egui::pos2(
        rect.left() + 16.0 + 10.0,
        rect.top() + 14.0 + lines[0].size().y / 2.0,
    );
    ui.painter()
        .circle_stroke(dot, 8.0, egui::Stroke::new(1.5, dim(p.ink)));
    if chosen {
        ui.painter().circle_filled(dot, 4.0, p.ink);
    }
    let x = rect.left() + 16.0 + 20.0 + 12.0;
    let mut y = rect.top() + 14.0;
    for g in lines {
        let h = g.size().y;
        ui.painter().galley(egui::pos2(x, y), g, ink);
        y += h + gap;
    }
    if enabled {
        resp.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        resp
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolite_api::KeyKind;

    /// TECH-platform-errors (audit Д4): a change not known to have gone through is not said as
    /// «The keys did not change»; a refusal is. Mutant: `text::failed` for every answer.
    #[test]
    fn a_change_not_known_is_not_said_as_not_done() {
        assert!(!said(&CoreError::Unclear).contains("did not change"));
        assert!(said(&CoreError::Unclear).contains("Could not find out"));
        assert!(said(&CoreError::NotEnough).contains("The keys did not change"));
    }

    fn key(id: u32, level: KeyLevel, purpose: KeyPurpose, owner: KeyOwner) -> IdentityKey {
        IdentityKey {
            id,
            level,
            purpose,
            kind: KeyKind::Ecdsa,
            decrypts: false,
            disabled_at: None,
            bound_to: None,
            owner,
        }
    }

    /// R-3: what disabling a key stops is said for that key.
    #[test]
    fn what_stops_is_said_for_each_key() {
        let app = key(
            3,
            KeyLevel::High,
            KeyPurpose::Documents,
            KeyOwner::App("Yappr".into()),
        );
        assert!(stops(&app).contains("Yappr"));
        let tokens = key(2, KeyLevel::Critical, KeyPurpose::Documents, KeyOwner::This);
        assert!(stops(&tokens).contains("tokens"));
        let transfers = key(3, KeyLevel::Critical, KeyPurpose::Transfers, KeyOwner::This);
        assert!(stops(&transfers).contains("credits"));
        let medium = key(7, KeyLevel::Medium, KeyPurpose::Documents, KeyOwner::Other);
        assert!(stops(&medium).starts_with("Nothing else"));
    }

    /// Each dialog asks the core for its own change; a wrong password keeps the dialog.
    #[test]
    fn each_change_asks_for_its_own() {
        assert_eq!(
            KeyChange::new(ChangeKind::Disable(3)).asked(&[]),
            (vec![], vec![3])
        );
        assert_eq!(
            KeyChange::new(ChangeKind::ReplaceMaster).asked(&[]),
            (vec![KeyRequest::ReplaceMaster], vec![])
        );
        assert_eq!(
            KeyChange::adding(KeyRequest::DashPay).asked(&[]),
            (vec![KeyRequest::DashPay], vec![])
        );
        assert_eq!(KeyChange::new(ChangeKind::Add).asked(&[]), (vec![], vec![]));
        let mut change = KeyChange::new(ChangeKind::ReplaceMaster);
        change.waiting = true;
        change.on_changed(Err(CoreError::WrongPassword));
        assert!(change.password.is_wrong() && !change.waiting);
    }
}
