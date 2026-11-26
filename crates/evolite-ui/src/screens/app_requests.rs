//! 11–13 · Requests from apps, decoded by the core: sign-in with App Connect v2 (its own
//! screen is FEAT-0023 round 4; App Connect v1 is not supported) and the key exchange over QR.
//! Plus the dialogs to paste a link and to disconnect an app. The answer goes back to the
//! core; nothing here holds a key.

use super::txt;
use crate::components::{
    Assets, ButtonKind, FieldKind, Part, button, button_enabled, button_row, dialog_with_footer,
    error_notice, field_label, kv_row, notice, password_asked, text_field,
};
use crate::context::Ctx;
use crate::text;
use crate::theme::{Weight, mono};
use egui::Ui;
use evolite_api::{AppRequest, AppRequestKind, Command, CoreError, Spend};

#[derive(Default)]
pub struct AppRequests {
    /// The request the core decoded and the user has not answered yet.
    pub open: Option<AppRequest>,
    pub pasting: bool,
    pub link: String,
    pub link_error: Option<CoreError>,
    pub show_bytes: bool,
    /// The core's answer to "Allow" / "Sign" / "The code matches".
    pub result: Option<Result<(), CoreError>>,
    /// "Disconnect" asked about this app (its id and name), not confirmed yet; then the core's
    /// answer while the dialog is open.
    pub disconnecting: Option<(String, String)>,
    pub disconnect_result: Option<Result<(), CoreError>>,
    /// "Disconnect" (TECH-spend-flow): "password for every payment" in it — it is an
    /// IdentityUpdate the identity pays for (FEAT-0023, BUG-42) — sent once, its own answer only.
    pub disconnect_flow: crate::components::spend_flow::SpendFlow<()>,
    /// "Allow" / "Sign": "password for every payment" in it — the app spends the identity's
    /// credits or adds a key to it (BUG-42) — sent once, its own answer only.
    pub flow: crate::components::spend_flow::SpendFlow<()>,
    /// Where the person's yes to [`Self::open`] is (BUG-30).
    pub consent: Consent,
    /// «Scan QR» (FEAT-0006, UC-4) and what it said when the code was not an app's.
    pub scanner: crate::qr_scan::Scanner,
    pub qr_error: Option<&'static str>,
    /// App Connect v2 (FEAT-0023 round 4): the request dialog's choices.
    pub choice: super::connect2::Choice,
    /// A refused link, said in its own dialog (E1–E3).
    pub refused: Option<evolite_api::AppLinkRefusal>,
    /// «Renew» asked about (R1): the app's name and how its limit stands now.
    pub renewing: Option<(String, String)>,
    /// «Remove from the list» asked about (D2): the app's id and name.
    pub removing: Option<(String, String)>,
    /// After a sign-in: «… connected: limit … until …» above the list, until closed.
    pub connected: Option<String>,
    /// The banner the sign-in under way will leave when it is done.
    pub connecting: Option<String>,
    /// The apps whose budgets were asked for in this visit of the screen, and when (`egui` time,
    /// seconds). Forgotten on leaving the screen, so each visit asks again (BUG-147).
    pub budgets_asked: std::collections::HashMap<String, f64>,
    /// `--screen apps-…` (dev-tools): what to open once the list has its apps — `renew`,
    /// `disconnect`, `remove` or `to-end` (the list scrolled to the disconnected ones).
    pub preset: Option<&'static str>,
    /// `--screen connect-limits`: the request scrolled to its end (A1b).
    pub body_to_end: bool,
}

/// The person's yes to a request ("Allow sign-in", "Sign", "The code matches"). The core takes
/// the request away at the first answer, so after a yes there is nothing left to answer: a
/// second yes would come back "out of date" and a "no" would not stop what is under way (BUG-30).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Consent {
    /// Not answered: both buttons work.
    #[default]
    Asking,
    /// The yes went to the core; the network has not answered yet.
    Sending,
    /// The app got the answer.
    Sent,
    /// The yes did not go through ([`AppRequests::result`] says why); the request is over. A
    /// wrong wallet password is not this: the core refused it before touching the request, so
    /// the dialog asks again (BUG-42).
    Failed,
}

/// What a click in the request dialog asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    Yes,
    No,
    /// "Close" once the yes is answered, or ✕ / Esc.
    Close,
}

impl AppRequests {
    /// Takes the core's answer to a link or to the user's decision.
    pub fn on_request(&mut self, request: Result<AppRequest, CoreError>) {
        match request {
            Ok(r) => {
                // BUG-209: another request starts clean — the password typed for the one it
                // replaces, that one's yes under way and its banner are not this one's. Its
                // late answer then finds no flow waiting for it.
                if self.open.as_ref().is_none_or(|open| open.id != r.id) {
                    self.flow.forget();
                    self.connecting = None;
                    // The dev presets' «at its end» is for the first request they open; only a
                    // request that replaces an open one starts at its top.
                    if self.open.is_some() {
                        self.body_to_end = false;
                    }
                }
                self.open = Some(r);
                self.pasting = false;
                self.link.clear();
                self.link_error = None;
                self.result = None;
                self.show_bytes = false;
                self.consent = Consent::Asking;
            }
            // App Connect v2 says why in a dialog of its own (E1–E3); the paste window closes.
            Err(CoreError::AppLink(why)) => {
                self.refused = Some(why);
                self.pasting = false;
                self.link_error = None;
            }
            // The link window shows why; a link from "Show an example" opens it for that.
            Err(e) => {
                self.link_error = Some(e);
                self.pasting = true;
            }
        }
    }

    /// The core's answer to "Disconnect": done closes the dialog, a refusal stays in it.
    pub fn on_disconnected(&mut self, ask: Option<u64>, result: Result<(), CoreError>) {
        if !self.disconnect_flow.on_done(ask, &result) {
            return;
        }
        let _ = self.disconnect_flow.take_result();
        if result.is_ok() {
            self.disconnecting = None;
            self.disconnect_result = None;
            // «… connected: limit …» no longer holds; the list shows the app disconnected.
            self.connected = None;
        } else {
            self.disconnect_result = Some(result);
        }
    }

    /// The core's answer to a yes ("Allow", "Sign"), taken only by the dialog that asked it
    /// (`ask`), or to a "no" (`DeclineApp`, not a spend: no number).
    pub fn on_answer(&mut self, ask: Option<u64>, result: Result<(), CoreError>) {
        if ask.is_some() {
            if !self.flow.on_done(ask, &result) {
                return;
            }
            let _ = self.flow.take_result();
        } else if self.consent == Consent::Sending {
            // A "no" answered while a yes waits is not the yes's answer.
            return;
        }
        if self.consent == Consent::Sending && result == Err(CoreError::WrongPassword) {
            // The core's gate refused the password before the request was touched: it is
            // still held, so the person retypes and says yes again. The line under the field
            // says what happened (BUG-42).
            self.consent = Consent::Asking;
            self.result = Some(result);
        } else if self.consent == Consent::Sending {
            // An App Connect v2 sign-in done: the list says so (A1, after).
            if result.is_ok() && self.connecting.is_some() {
                self.connected = self.connecting.take();
                self.open = None;
                self.consent = Consent::Asking;
                self.flow.forget();
                return;
            }
            self.connecting = None;
            // The yes is answered: the dialog says how it went and offers only "Close".
            self.consent = if result.is_ok() {
                Consent::Sent
            } else {
                Consent::Failed
            };
            self.result = result.err().map(Err);
        } else if result.is_ok() {
            // Declined: the request is over.
            self.open = None;
        } else {
            self.result = Some(result);
        }
    }

    /// The person's click on the open request, and the command it sends to the core. After a
    /// yes nothing goes to the core again; closing while it is sent keeps the dialog (BUG-30).
    /// `ask`: the wallet wants its password for every spend, and a yes is one (BUG-42).
    /// An App Connect v2 sign-in takes the dialog's choices and always the password: it cannot be
    /// undone for the identity (FEAT-0023 round 4).
    pub fn pick(&mut self, pick: Pick, ask: bool, now: i64) -> Option<Command> {
        let request = self.open.clone()?;
        let id = request.id;
        match (self.consent, pick) {
            (Consent::Asking, Pick::Yes) => {
                if self.choice.for_request != id {
                    self.choice = super::connect2::Choice::of(&request, now);
                }
                let v2 = matches!(request.kind, AppRequestKind::Connect2 { .. });
                let grant = if v2 {
                    let grant = super::connect2::grant(&request, &self.choice, now)?;
                    self.connecting = Some(super::connect2::connected_banner(&request, &grant));
                    Some(grant)
                } else {
                    None
                };
                self.consent = Consent::Sending;
                self.result = None;
                self.flow
                    .command(Spend::ApproveApp { id, grant }, ask || v2, ())
            }
            (Consent::Asking, Pick::No) => {
                self.result = None;
                Some(Command::DeclineApp { id })
            }
            // Closing is declining: the app hears "no".
            (Consent::Asking, Pick::Close) => {
                self.open = None;
                self.result = None;
                self.flow.forget();
                Some(Command::DeclineApp { id })
            }
            (Consent::Sent | Consent::Failed, Pick::Close) => {
                self.open = None;
                self.result = None;
                self.consent = Consent::Asking;
                self.flow.forget();
                None
            }
            (Consent::Sending, _) | (Consent::Sent | Consent::Failed, _) => None,
        }
    }
}

/// The app as the core names it: its owner's `@name` from Platform, or the app's own word in
/// «…» (`real/app_jobs.rs`). The word and whether it is the app's own.
fn app_word(app: &str) -> (&str, bool) {
    match app.strip_prefix('«').and_then(|w| w.strip_suffix('»')) {
        Some(word) => (word, true),
        None => (app, false),
    }
}

/// The app for the person: its own word in the quotes of the interface language (BUG-52;
/// «…» stays the core's marker).
pub fn app_label(app: &str) -> String {
    match app_word(app) {
        (word, true) => quote(word),
        (name, false) => name.to_owned(),
    }
}

/// A word in the quotes of the interface language. Translated here: it goes into other
/// sentences as a value.
fn quote(word: &str) -> String {
    crate::i18n::t(&format!(tr!("“{}”"), word)).into_owned()
}

fn dash(d: i64) -> String {
    crate::format::amount(d)
}

/// Draws whichever dialog is open.
pub fn show(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, state: &mut AppRequests) {
    if state.pasting {
        paste_dialog(ui, a, cx, state);
    }
    if let Some(why) = state.refused.clone() {
        let mut again = false;
        let mut retry = None;
        if super::connect2::refusal(ui, a, &why, &mut again, &mut retry) {
            state.refused = None;
            state.pasting = again;
        }
        // BUG-323: «Try again» opens the same link again.
        if let Some(text) = retry {
            cx.send(Command::OpenAppLink { text });
        }
    }
    let Some(request) = state.open.clone() else {
        return;
    };
    let ask = password_asked(cx.snap);
    // App Connect v2, the one kind of request an app can make (FEAT-0023).
    let now = cx.now.timestamp();
    if let Some(pick) = super::connect2::request(ui, a, cx, state, &request)
        && let Some(command) = state.pick(pick, ask, now)
    {
        cx.send(command);
    }
}

/// "Disconnect": what it does and costs, before the core is asked.
pub fn disconnect_dialog(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, state: &mut AppRequests) {
    let Some((id, name)) = state.disconnecting.clone() else {
        return;
    };
    let cost = cx.wallet().fees.add_key;
    let closed = crate::components::dialog_with_summary(
        ui,
        a,
        "app-disconnect",
        "lock",
        &format!(tr!("Disconnect {}"), app_label(&name)),
        462.0,
        |ui, a, part| {
            let p = a.p;
            // BUG-249: what is agreed to, pinned over the password: the identity update and
            // who pays it. What it does stays in the middle above.
            if part == Part::Summary {
                kv_row(
                    ui,
                    &p,
                    tr!("Identity update"),
                    &format!("≈ {}", dash(cost)),
                    mono(13.0, Weight::Medium),
                );
                let pays = cx
                    .wallet()
                    .identity
                    .as_ref()
                    .and_then(|i| i.name())
                    .map_or_else(|| "identity".to_owned(), |n| format!("@{n}"));
                kv_row(
                    ui,
                    &p,
                    tr!("Pays"),
                    &format!(tr!("{} · credits"), pays),
                    mono(13.0, Weight::Medium),
                );
                // FEAT-0057 П3: the credits now and after the update.
                crate::components::credits_after(
                    ui,
                    a,
                    &pays,
                    crate::components::Credits::of(cx.wallet()),
                    crate::components::Charge::About(cost),
                );
                // FEAT-0004 FR-7: the large-spend tick with what it agrees to.
                if !state.disconnect_flow.waiting() {
                    state.disconnect_flow.password.summary_tick(ui, &p);
                }
                return false;
            }
            if part == Part::Footer {
                // The footer (BUG-122): always in the window.
                let mut close = false;
                let ask = password_asked(cx.snap);
                let entered = !state.disconnect_flow.waiting()
                    && state
                        .disconnect_flow
                        .password
                        .field(ui, &p, "app-disconnect-password", ask);
                let ready = state.disconnect_flow.password.ready(ask);
                if let Some(Err(e)) = state
                    .disconnect_result
                    .as_ref()
                    .filter(|_| !state.disconnect_flow.password.is_wrong())
                {
                    error_notice(
                        ui,
                        a,
                        &text::failed(tr!("Not disconnected"), e),
                        text::error_details(e).as_deref(),
                    );
                }
                // The keys cannot be turned back on: «Cancel» first (the designer's rule b′).
                crate::components::button_row_destructive(ui, |ui| {
                    if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                        close = true;
                    }
                    let label = if state.disconnect_flow.waiting() {
                        tr!("Disconnecting…")
                    } else {
                        tr!("Disconnect")
                    };
                    if button_enabled(
                        ui,
                        a,
                        label,
                        ButtonKind::Danger,
                        None,
                        !state.disconnect_flow.waiting() && ready,
                    )
                    .clicked()
                        || (entered && ready)
                    {
                        let op = Spend::DisconnectApp { id: id.clone() };
                        if let Some(command) = state.disconnect_flow.command(op, ask, ()) {
                            cx.send(command);
                            state.disconnect_result = None;
                        }
                    }
                });
                return close;
            }
            ui.add(
                crate::components::para(
                        &format!(
                            tr!("The {name} keys on the identity will be disabled: the app can no longer sign with them. They cannot be turned back on — only signed in again.")
                        , name = name),
                        14.0,
                        Weight::Regular,
                        p.muted,
                    )
                    .line_height(21.0),
            );
            // D1: the encryption key stays (DIP, "Disconnect").
            notice(
                ui,
                a,
                "info",
                p.muted,
                &format!(
                    tr!(
                        "The encryption key for {} messages stays: it spends nothing, and without it old messages cannot be read."
                    ),
                    app_label(&name)
                ),
            );
            false
        },
    );
    if closed && !state.disconnect_flow.waiting() {
        state.disconnecting = None;
        state.disconnect_result = None;
        state.disconnect_flow.password.forget();
    }
}

/// "Paste a link": an App Connect v2 link from an app (`dashpay://connect`, or `dash-key:` v2).
fn paste_dialog(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, state: &mut AppRequests) {
    let closed = dialog_with_footer(
        ui,
        a,
        "app-link",
        "link",
        tr!("Paste a link"),
        462.0,
        |ui, a, part| {
            let p = a.p;
            if part == Part::Footer {
                // The footer (BUG-122): always in the window.
                let mut close = false;
                button_row(ui, |ui| {
                    if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                        close = true;
                    }
                    let ready = !state.link.trim().is_empty();
                    if button_enabled(ui, a, tr!("Open"), ButtonKind::Primary, None, ready)
                        .clicked()
                    {
                        cx.send(Command::OpenAppLink {
                            text: state.link.clone(),
                        });
                    }
                });
                return close;
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                field_label(ui, &p, tr!("Link from an app"));
                if text_field(
                    ui,
                    &p,
                    "app-link",
                    &mut state.link,
                    tr!("dashpay://connect?…"),
                    FieldKind::Text,
                    |_| {},
                )
                .changed()
                {
                    state.link_error = None;
                }
                if let Some(e) = &state.link_error {
                    let line = match e {
                        CoreError::BadLink => tr!("This is not a Dash app request").to_string(),
                        CoreError::NotAvailable(why) => why.clone(),
                        other => text::error_text(other),
                    };
                    ui.label(txt(&line, 12.0, Weight::Regular, p.danger));
                }
            });
            notice(
                ui,
                a,
                "info",
                p.muted,
                tr!("evolite decodes the link itself and shows exactly what the app asks for."),
            );
            false
        },
    );
    if closed {
        state.pasting = false;
        state.link.clear();
        state.link_error = None;
    }
}

impl crate::screen_state::ScreenState for AppRequests {
    #[cfg(all(test, demo_core, feature = "dev-tools"))]
    fn flows(&mut self) -> Vec<&mut dyn crate::components::spend_flow::AnyFlow> {
        vec![&mut self.flow, &mut self.disconnect_flow]
    }

    fn forget(&mut self, why: crate::screen_state::Why) {
        match why {
            crate::screen_state::Why::WalletSwitch => *self = Self::default(),
            _ => {
                self.flow.forget();
                self.disconnect_flow.forget();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asked(id: u64) -> AppRequests {
        let mut state = AppRequests::default();
        state.on_request(Ok(AppRequest {
            id,
            app: "@yappr".into(),
            kind: AppRequestKind::Connect2 {
                app_id: "A".into(),
                label: "Yappr".into(),
                owner: "O".into(),
                owner_name: None,
                identity: "@me".into(),
                sessions: vec![evolite_api::SessionOffer {
                    binding: evolite_api::AppBinding::Group { id: "G".into() },
                    level: evolite_api::KeyLevel::High,
                    proposed_budget: None,
                    proposed_expiry: None,
                    budget: 50_000_000,
                    expires_at: 90 * 24 * 60 * 60,
                    changed: false,
                    primary: true,
                }],
                encryption: Vec::new(),
                limits: evolite_api::SessionLimits::wallet(),
                breaks_old_clients: true,
                renews: None,
                created_at: 0,
                callback_host: None,
                cost: 0,
            },
        }));
        state
    }

    fn out_of_date() -> CoreError {
        CoreError::NotAvailable("The request is out of date: open the app's link again".into())
    }

    fn is_yes(command: Option<Command>, want: u64) -> bool {
        matches!(
            command,
            Some(Command::Spend {
                op: Spend::ApproveApp { id, grant: Some(_) },
                ..
            }) if id == want
        )
    }

    fn is_no(command: Option<Command>, want: u64) -> bool {
        matches!(command, Some(Command::DeclineApp { id }) if id == want)
    }

    /// BUG-30: after a yes, neither a second yes nor "Decline" nor ✕ goes to the core, and the
    /// dialog stays until the network answers.
    #[test]
    fn after_a_yes_nothing_more_goes_to_the_core() {
        let mut state = asked(7);
        assert!(is_yes(state.pick(Pick::Yes, false, 0), 7));
        assert_eq!(state.consent, Consent::Sending);
        for pick in [Pick::Yes, Pick::No, Pick::Close] {
            assert!(state.pick(pick, false, 0).is_none(), "{pick:?} while sent");
        }
        assert!(state.open.is_some(), "the dialog waits for the network");
    }

    /// BUG-30, FEAT-0023 round 4: the network's yes closes the dialog and the list says the app
    /// is connected (A1, after); nothing more goes to the core.
    #[test]
    fn a_yes_that_went_through_is_shown_as_done() {
        let mut state = asked(7);
        state.pick(Pick::Yes, false, 0);
        state.on_answer(state.flow.waiting_ask(), Ok(()));
        assert!(state.open.is_none(), "the dialog closed");
        assert!(
            state
                .connected
                .as_deref()
                .is_some_and(|c| c.starts_with("Yappr connected")),
            "{:?}",
            state.connected
        );
        assert_eq!(state.consent, Consent::Asking, "ready for the next request");
        assert!(state.pick(Pick::No, false, 0).is_none(), "no request left");
    }

    /// BUG-30: a yes that failed says why and is over: no second try, closing sends no "no".
    #[test]
    fn a_failed_yes_says_why_and_is_over() {
        let mut state = asked(7);
        state.pick(Pick::Yes, false, 0);
        state.on_answer(state.flow.waiting_ask(), Err(out_of_date()));
        assert_eq!(state.consent, Consent::Failed);
        assert_eq!(state.result, Some(Err(out_of_date())));
        assert!(state.pick(Pick::Yes, false, 0).is_none());
        assert!(state.pick(Pick::Close, false, 0).is_none());
        assert!(state.open.is_none());
        assert_eq!(state.result, None);
    }

    /// BUG-30 with BUG-42: a wrong wallet password on "Allow" ends nothing. The core's gate
    /// refused it before the request was touched, so the dialog asks again: the line under the
    /// field says why, and the next yes goes to the core with the retyped password.
    #[test]
    fn a_wrong_password_on_yes_asks_again() {
        let mut state = asked(7);
        assert!(is_yes(state.pick(Pick::Yes, true, 0), 7));
        state.on_answer(state.flow.waiting_ask(), Err(CoreError::WrongPassword));
        assert_eq!(state.consent, Consent::Asking, "the request is still held");
        assert!(state.open.is_some());
        assert!(state.flow.password.is_wrong(), "said under the field");
        assert_eq!(state.result, Some(Err(CoreError::WrongPassword)));
        assert!(
            is_yes(state.pick(Pick::Yes, true, 0), 7),
            "a second try goes"
        );
        assert_eq!(state.consent, Consent::Sending);
        state.on_answer(state.flow.waiting_ask(), Ok(()));
        assert!(
            state.open.is_none() && state.connected.is_some(),
            "connected"
        );
        assert!(!state.flow.password.is_wrong());
    }

    /// Before any yes, "Decline" and ✕ still say "no" to the app; a new request asks again.
    #[test]
    fn before_a_yes_declining_still_says_no() {
        let mut state = asked(7);
        assert!(is_no(state.pick(Pick::No, false, 0), 7));
        state.on_answer(state.flow.waiting_ask(), Ok(()));
        assert!(state.open.is_none(), "declined: the request is over");
        let mut state = asked(8);
        state.pick(Pick::Yes, false, 0);
        state.on_answer(state.flow.waiting_ask(), Ok(()));
        state.on_request(asked(9).open.ok_or_else(out_of_date));
        assert_eq!(state.consent, Consent::Asking, "a new request asks again");
        assert!(is_no(state.pick(Pick::Close, false, 0), 9));
    }

    /// BUG-209: a new request replaces the open one with nothing of it — the password typed
    /// for it is not this one's, and its yes under way answers nothing here.
    #[test]
    fn another_request_takes_nothing_of_the_one_it_replaces() {
        let password = |command: Option<Command>| match command {
            Some(Command::Spend {
                op: Spend::ApproveApp { id, .. },
                password,
                ..
            }) => Some((id, password.is_some_and(|p| !p.expose().is_empty()))),
            _ => None,
        };
        // Typed for 7; 8 comes before the yes: its yes goes without that password.
        let mut state = asked(7);
        state.flow.password.type_in("seven's");
        assert!(state.flow.password.ready(true));
        state.on_request(asked(8).open.ok_or_else(out_of_date));
        assert!(!state.flow.password.ready(true), "the field empty for 8");
        assert_eq!(password(state.pick(Pick::Yes, true, 0)), Some((8, false)));

        // 7's yes under way; 8 comes: 8 is asked anew, and 7's answer is not 8's.
        let mut state = asked(7);
        state.flow.password.type_in("seven's");
        assert_eq!(password(state.pick(Pick::Yes, true, 0)), Some((7, true)));
        let seven = state.flow.waiting_ask();
        state.on_request(asked(8).open.ok_or_else(out_of_date));
        assert!(state.connecting.is_none(), "7's banner");
        assert_eq!(state.consent, Consent::Asking);
        state.on_answer(seven, Ok(()));
        assert!(state.connected.is_none(), "7's sign-in is not shown as 8's");
        assert_eq!(state.open.as_ref().map(|r| r.id), Some(8), "8 still asks");
        assert_eq!(state.result, None);
        assert!(state.flow.can_act(), "8's yes can go");

        // The same request again (its link opened twice) keeps what was typed.
        let mut state = asked(7);
        state.flow.password.type_in("seven's");
        state.on_request(asked(7).open.ok_or_else(out_of_date));
        assert!(state.flow.password.ready(true));
    }

    /// BUG-52, review should: the app's own word, which the core marks «…», is quoted as the
    /// interface language quotes; an `@name` from Platform is shown as it is.
    #[test]
    fn an_apps_own_word_is_quoted_in_the_interface_language() {
        use crate::i18n::{Lang, set};
        set(Lang::En);
        assert_eq!(app_label("«yappr»"), "“yappr”");
        assert_eq!(app_label("@yappr"), "@yappr");
        set(Lang::Ru);
        assert_eq!(app_label("«yappr»"), "«yappr»");
        set(Lang::En);
    }

    /// FEAT-0023 round 5 (the coordinator's live pass): «Yappr connected: limit …» does not stay
    /// above the list once an app is disconnected; a refused disconnect leaves it.
    #[test]
    fn a_disconnect_takes_the_connected_banner_away() {
        let mut state = AppRequests {
            connected: Some("Yappr connected: limit 0.51 DASH".into()),
            ..AppRequests::default()
        };
        state
            .disconnect_flow
            .set_stage(crate::components::spend_flow::Stage::Waiting { ask: 0, signed: () });
        state.on_disconnected(Some(0), Err(CoreError::WrongPassword));
        assert!(state.connected.is_some(), "refused: nothing changed");
        state
            .disconnect_flow
            .set_stage(crate::components::spend_flow::Stage::Waiting { ask: 0, signed: () });
        state.on_disconnected(Some(0), Ok(()));
        assert_eq!(state.connected, None);
    }
}
