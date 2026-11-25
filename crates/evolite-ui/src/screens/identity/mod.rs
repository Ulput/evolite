//! 06 · Identity: profile with the verified name, credits balance and actions, DPNS names and
//! keys, all from the core's snapshot.

mod add_name;
pub(crate) mod contest;
mod contest_card;
mod imgur;
mod key_change;
mod key_reveal;
mod keys;
mod profile;
pub(crate) mod resend;
mod top_up;

pub use add_name::*;
pub use keys::{KeysState, enable_dashpay_card, identity_keys, identity_named, needs_dashpay};
pub use profile::*;
pub use top_up::*;

use super::Screen;
use crate::components::spend_flow::SpendFlow;
use crate::components::text;
use crate::components::{
    Assets, ButtonKind, CardAmount, FieldKind, RowCell, amount_field, avatar, button,
    button_enabled, button_row, card, chip, divider, field_label, inner, kv_row, list_card_in,
    notice, page_header, pill, text_action, text_field, two_columns,
};
use crate::context::Ctx;
use crate::text::{self, group_thousands, short_id};
use crate::theme::Weight;
use crate::theme::style;
use crate::theme::type_scale::{AMOUNT_L, BODY, CAPTION, HEADING, SMALL, UI};
use egui::{Align, Align2, CornerRadius, CursorIcon, Layout, Margin, Rect, Sense, Ui, pos2, vec2};
use evolite_api::{
    Balances, Command, CoreError, Fees, Identity, IdentitySearch, NameCheck, Profile, Source,
    Spend, Wallet,
};

/// 1 duff = 1000 credits on Platform.
const CREDITS_PER_DUFF: i64 = 1_000;

#[derive(Default)]
pub struct IdentityState {
    /// "Edit profile" (23) is open.
    pub editing: Option<ProfileForm>,
    /// "Top up the identity" is open; the top-up itself outlives the window.
    pub top_up: Option<TopUpForm>,
    /// «Top up the identity» (TECH-spend-flow): the duffs signed, while it waits.
    pub top_up_flow: SpendFlow<i64>,
    /// «Continue the top-up» on the screen itself: the duffs locked.
    pub resume_flow: SpendFlow<i64>,
    /// The last top-up's duffs and answer, until a new one starts.
    pub top_up_result: Option<(i64, Result<(), CoreError>)>,
    /// "Add a name" is open.
    pub adding_name: Option<NameForm>,
    /// The banner after «Create» (FEAT-0014, analyst-44), until closed or another identity is
    /// chosen.
    pub created: Option<Created>,
    /// «Create» was sent for another identity and the core has not answered yet: App sets it
    /// from the form before each frame (R-4: the creation under way is not in the snapshot).
    pub creating: bool,
    /// The contest card (FEAT-0021 N5) is open on this identity's request for this label.
    pub contest: Option<(String, String)>,
    /// A contested name typed when the identity was made (FEAT-0021 UC-5, AC-T23): Identity
    /// offers its claim, a spend of its own, until it is claimed or closed.
    pub claim_after: Option<String>,
    /// «Write the link again» on the contest card (FEAT-0021 АРХ-6, AC-T12): its password, and
    /// its answer kept until the card closes (TECH-spend-flow, BUG-207).
    pub link_flow: SpendFlow<()>,
}

/// The banner «Identity … created» over «Work as».
#[derive(Clone, Debug, PartialEq)]
pub enum Created {
    /// The core said it is made; the snapshot still shows the identity chosen before (`None`:
    /// there was none).
    Awaiting(Option<String>),
    /// Shown for this identity while it is the chosen one.
    Shown(String),
}

/// The identity the banner is shown for this frame, with `chosen` the one the wallet acts as:
/// the new one as soon as the snapshot has it chosen; none once another one is chosen (the
/// banner closes by itself, the designer).
fn created_banner(created: &mut Option<Created>, chosen: Option<&str>) -> Option<String> {
    let next = match (created.take(), chosen) {
        (Some(Created::Awaiting(before)), Some(now)) if before.as_deref() != Some(now) => {
            Some(Created::Shown(now.to_owned()))
        }
        (Some(Created::Awaiting(before)), _) => Some(Created::Awaiting(before)),
        (Some(Created::Shown(id)), Some(now)) if id == now => Some(Created::Shown(id)),
        _ => None,
    };
    *created = next;
    match created {
        Some(Created::Shown(id)) => Some(id.clone()),
        _ => None,
    }
}

impl IdentityState {
    /// The core's answer to `CheckName`, when it is about the name being added.
    pub fn on_name_checked(&mut self, label: &str, result: &Result<NameCheck, CoreError>) {
        if let Some(form) = &mut self.adding_name
            && form.asked == label
        {
            form.check = Some(result.clone());
        }
    }

    /// The core's answer to `AddName` or `SetNameLink`, taken only by the window that asked it
    /// (`ask`; BUG-207: the link written again from the contest card is not «Add a name»'s).
    pub fn on_name_done(&mut self, ask: Option<u64>, result: &Result<(), CoreError>) {
        // The contest card keeps its answer: no second write until it is closed (BUG-207).
        if self.link_flow.on_done(ask, result) {
            return;
        }
        let Some(form) = &mut self.adding_name else {
            return;
        };
        if !form.flow.on_done(ask, result) {
            return;
        }
        // A wrong password is said under its field; the name can be registered again.
        let Some(((), result)) = form.flow.take_result() else {
            return;
        };
        // FEAT-0021 R-2: the fund rose between the review and the signature. Nothing was sent;
        // the review shows the new fund and asks for the password again.
        if let (Err(CoreError::ContestFundChanged { fund }), Some(review)) =
            (&result, &mut form.review)
        {
            review.was = Some(review.fund);
            review.fund = *fund;
            form.result = None;
            return;
        }
        form.result = Some(result);
    }

    /// Opens the contest card (FEAT-0021 N5) on `identity`'s request for `label`: the one way in
    /// of every entry (AC-T21).
    pub fn open_contest(&mut self, identity: &str, label: &str) {
        self.contest = Some((identity.to_owned(), label.to_owned()));
    }

    /// The duffs of a top-up sent to the core, no answer yet.
    pub fn topping_up(&self) -> Option<i64> {
        [&self.top_up_flow, &self.resume_flow]
            .into_iter()
            .find(|f| f.waiting())
            .and_then(|f| f.signed().copied())
    }

    /// The core's answer to `TopUpIdentity` or `ResumeTopUp`, taken only by the flow that
    /// asked it (`ask`): the answer of a wallet left meanwhile is not this one's (BUG-45,
    /// BUG-214). A wrong password is said under its field, not as a failed top-up.
    /// True: the answer was a top-up's of this screen (from Core, mixed coins or the pool).
    pub fn on_top_up_done(&mut self, ask: Option<u64>, result: &Result<(), CoreError>) -> bool {
        for flow in [&mut self.top_up_flow, &mut self.resume_flow] {
            if flow.on_done(ask, result) {
                if let Some(answer) = flow.take_result() {
                    self.top_up_result = Some(answer);
                }
                return true;
            }
        }
        false
    }

    /// An identity was made (FEAT-0014): its banner, once the snapshot has it chosen. `before`:
    /// the identity chosen when the creation was signed; none — not sent from this app run.
    pub fn on_created(&mut self, before: Option<Option<String>>) {
        if let Some(before) = before {
            self.created = Some(Created::Awaiting(before));
        }
    }

    /// The wallet locked: what was typed for a spend goes, and its windows wait for nothing.
    fn on_lock(&mut self) {
        self.top_up_flow.forget();
        self.resume_flow.forget();
        self.link_flow.forget();
        if let Some(form) = &mut self.adding_name {
            form.flow.forget();
        }
        if let Some(form) = &mut self.editing {
            form.flow.forget();
        }
    }

    /// The core's answer to `SaveProfile`, taken only by the form that asked it (`ask`).
    pub fn on_profile_saved(&mut self, ask: Option<u64>, result: &Result<(), CoreError>) {
        let Some(form) = &mut self.editing else {
            return;
        };
        if !form.flow.on_done(ask, result) {
            return;
        }
        // A wrong password is said under its field.
        let Some(((), result)) = form.flow.take_result() else {
            return;
        };
        if result.is_ok() {
            self.editing = None;
        } else {
            form.result = Some(result);
        }
    }
}

impl crate::screen_state::ScreenState for IdentityState {
    #[cfg(all(test, demo_core, feature = "dev-tools"))]
    fn flows(&mut self) -> Vec<&mut dyn crate::components::spend_flow::AnyFlow> {
        let mut flows: Vec<&mut dyn crate::components::spend_flow::AnyFlow> = vec![
            &mut self.top_up_flow,
            &mut self.resume_flow,
            &mut self.link_flow,
        ];
        if let Some(form) = &mut self.adding_name {
            flows.push(&mut form.flow);
        }
        if let Some(form) = &mut self.editing {
            flows.push(&mut form.flow);
        }
        flows
    }

    fn forget(&mut self, why: crate::screen_state::Why) {
        use crate::screen_state::Why;
        match why {
            Why::Lock => self.on_lock(),
            // Nothing of another wallet is shown (BUG-45, BUG-214).
            Why::WalletSwitch => *self = Self::default(),
            // A spend asked for the last identity waits for nothing now (History shows what it
            // did); a form opened for the new one (a claim's top-up after «Create») stays.
            Why::IdentitySwitch => self.on_lock(),
        }
    }
}

pub fn identity(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, state: &mut IdentityState) {
    let wallet = cx.wallet();
    let mut asked: Vec<Command> = Vec::new();
    ui.spacing_mut().item_spacing.y = 22.0;
    let has_identity = wallet.identity.is_some();
    page_header(ui, a, "Identity", None, |ui, a| {
        // Without an identity there is nothing to top up or name: "Create an identity" below.
        if has_identity {
            let why_not = top_up_blocked(wallet, state);
            let resp = button_enabled(
                ui,
                a,
                tr!("Top up"),
                ButtonKind::Primary,
                None,
                why_not.is_none(),
            );
            if let Some(why) = why_not {
                resp.on_disabled_hover_text(crate::i18n::t(why).into_owned());
            } else if resp.clicked() {
                open_top_up(state);
            }
            if button(ui, a, tr!("Add a name"), ButtonKind::Secondary, None).clicked() {
                state.adding_name = Some(NameForm::default());
            }
            // An identity or a name made elsewhere since the search (BUG-9). While one runs
            // the notice below says so, a failed pass too: the core tries again by itself.
            if wallet.identity_search == IdentitySearch::Done
                && button(ui, a, tr!("Search again"), ButtonKind::Secondary, None).clicked()
            {
                asked.push(Command::FindIdentities);
            }
        }
    });
    // BUG-204, BUG-212: the files of this screen that did not read and were set aside.
    crate::screens::set_aside::banners(
        ui,
        a,
        cx,
        &[
            evolite_api::LocalFile::NameRequests,
            evolite_api::LocalFile::TopUps,
        ],
    );
    identity_choice(ui, a, cx, state, &mut asked);
    top_up_status(ui, a, wallet, state, &mut asked);
    if let Some(unfinished) = &wallet.unfinished_identity {
        let p = a.p;
        let text = match unfinished.locked {
            Some(locked) => format!(
                tr!(
                    "Making the identity stopped: {} are locked on Core. Continue — no new money is needed."
                ),
                crate::format::amount(locked)
            ),
            None => {
                tr!("The identity is made, but the name was not written. Continue to write it.")
                    .into()
            }
        };
        notice(ui, a, "warning", p.accent, &text);
        if button(ui, a, tr!("Continue creating"), ButtonKind::Primary, None).clicked() {
            cx.go(Screen::CreateIdentity);
        }
    }
    let search_again = search_status(ui, a, &wallet.identity_search, has_identity);
    let Some(identity) = &wallet.identity else {
        // While the search runs an identity may yet be found: waiting, said neutrally, with
        // nothing to do and no amounts (CS-4, BUG-74 №2).
        if wallet.identity_search == IdentitySearch::Searching {
            searching_for_identity(ui, a);
        } else {
            no_identity(ui, a, cx, wallet, search_again);
        }
        return;
    };
    if search_again {
        asked.push(Command::FindIdentities);
    }

    let out = egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 20.0;
            let total = ui.available_width();
            // Two rows of two cards on one grid, halves with a gap of 20 (design review
            // 2026-11-05 №7); below 760 px of content everything goes in one column.
            let wide = total >= 760.0;
            let half = if wide { (total - 20.0) / 2.0 } else { total };

            two_columns(ui, wide, |ui, first, cell| {
                if first {
                    profile(ui, a, state, identity, half, cell, &mut asked);
                } else {
                    match credits(ui, a, &wallet.balances, &wallet.fees, half, cell) {
                        Some(CreditsAction::TopUp) => state.top_up_clicked(wallet),
                        // Send from credits: to this wallet's own Core address (it can be
                        // changed there), or to someone yet to type.
                        Some(CreditsAction::Withdraw) => {
                            cx.pay(Source::Credits, wallet.core_address.clone());
                        }
                        Some(CreditsAction::Transfer) => cx.pay(Source::Credits, String::new()),
                        None => {}
                    }
                }
            });
            two_columns(ui, wide, |ui, first, cell| {
                if first {
                    let mut action = None;
                    names(ui, a, identity, &wallet.fees, half, cell, &mut action);
                    match action {
                        Some(NameAction::Open(label)) => state.open_contest(&identity.id, &label),
                        Some(NameAction::Another) => state.adding_name = Some(NameForm::default()),
                        Some(NameAction::Hide(label)) => asked.push(Command::HideNameRequest {
                            identity: identity.id.clone(),
                            label,
                        }),
                        None => {}
                    }
                } else {
                    // X1 (FEAT-0020): the keys summed up; K1 has them all.
                    if keys::keys_summary(ui, a, identity, half, cell) {
                        cx.go(Screen::IdentityKeys);
                    }
                }
            });
        });
    crate::diagnostics::screen_offset(out.state.offset.y);
    if state.editing.is_some() {
        edit_profile(
            ui,
            a,
            state,
            identity,
            wallet.fees.profile,
            wallet.password_each_send,
            &mut asked,
        );
    }
    if state.top_up.is_some() {
        top_up_dialog(
            ui,
            a,
            state,
            wallet,
            identity,
            &cx.snap.sync,
            cx.now,
            &mut asked,
        );
    }
    if state.adding_name.is_some() {
        add_name_dialog(ui, a, state, wallet, identity, &mut asked);
    }
    if state.contest.is_some() {
        contest_card::contest_card(ui, a, state, identity, &mut asked);
    }
    for command in asked {
        cx.send(command);
    }
}

fn profile(
    ui: &mut Ui,
    a: &mut Assets,
    state: &mut IdentityState,
    id: &Identity,
    width: f32,
    cell: &mut RowCell,
    asked: &mut Vec<Command>,
) {
    let p = a.p;
    let name = id.name().map(|n| format!("@{n}"));
    let display = id.profile.as_ref().and_then(|pr| pr.display.clone());
    cell.card(ui, card(&p, Margin::same(24)), |ui| {
        ui.set_width(inner(width, 24));
        ui.spacing_mut().item_spacing.y = 14.0;
        // A text action in the card's top right corner, on its right margin (design review
        // №15); the name below gets the whole width beside the picture.
        ui.horizontal(|ui| {
            let label = if id.profile.is_some() {
                tr!("Edit profile")
            } else {
                tr!("Create a profile")
            };
            // Icon 16 + gap 6 + the words, as `text_action` draws them.
            let w = 22.0 + crate::components::text_width(ui, label, style(UI.weight(Weight::Medium)));
            ui.add_space((ui.available_width() - w).max(0.0));
            if text_action(ui, a, "edit", label).clicked() {
                state.editing = Some(ProfileForm::from(id.profile.as_ref()));
            }
        });
        ui.add_space(-8.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            let (face, _) = ui.allocate_exact_size(vec2(72.0, 72.0), Sense::hover());
            let label = display.clone().or_else(|| name.clone()).unwrap_or_default();
            let picture = id.profile.as_ref().and_then(|pr| pr.avatar.as_deref());
            avatar(ui, a, face, &id.id, &label, picture, &mut |c| asked.push(c));
            ui.allocate_ui_with_layout(
                vec2(ui.available_width(), 72.0),
                Layout::top_down(Align::Min),
                |ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    // A long name wraps inside the card instead of widening it past the
                    // window (BUG-46); «Name verified» goes on the next row when it is short
                    // of room.
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(10.0, 4.0);
                        match &name {
                            Some(name) => {
                                ui.label(text(name, HEADING, p.text));
                                chip(ui, a, "confirmed", tr!("Name verified"), true);
                            }
                            None => {
                                ui.label(text(tr!("No name"), HEADING, p.text));
                            }
                        }
                    });
                    if let Some(display) = &display {
                        ui.label(text(display, BODY.weight(Weight::Medium), p.text));
                    }
                    // The ID with a copy icon; the icon turns into «Copied» for a moment.
                    // SEC-12: the shared copy and «Copied», as every other copy control.
                    let what = ui.id().with("copy-identity-id");
                    let copied = crate::clipboard::just_copied(ui.ctx(), what);
                    let label = if copied {
                        tr!("Copied").to_string()
                    } else {
                        short_id(&id.id)
                    };
                    // The words do not take the click for selecting text, and the row senses
                    // it under its own id, so a click on the ID copies as one on the icon
                    // does (BUG-39 №2).
                    let row = ui
                        .horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(crate::i18n::t(&label))
                                        .font(style(UI.mono()))
                                        .color(p.muted),
                                )
                                .selectable(false),
                            );
                            ui.add(a.icons.icon("copy", p.muted, &p, 14.0));
                        })
                        .response;
                    let resp = ui
                        .interact(row.rect, what, Sense::click())
                        .on_hover_cursor(CursorIcon::PointingHand)
                        .on_hover_text(crate::i18n::t(&id.id).into_owned());
                    crate::components::named(
                        &resp,
                        egui::WidgetType::Button,
                        true,
                        &format!("{} · {}", crate::i18n::spoken(tr!("Copy")), id.id),
                    );
                    if resp.clicked() {
                        crate::clipboard::copy_answered(ui.ctx(), id.id.clone(), what);
                    }
                },
            );
        });
        match &id.profile {
            Some(profile) => super::contacts::about_block(ui, a, profile),
            None => {
                ui.add(crate::components::para(
                    tr!("No DashPay profile yet: your contacts will see the name, “About” and avatar."), UI.size, UI.weight, p.muted).line_height(UI.line));
            }
        }
        divider(ui, &p);
        let value = style(UI.weight(Weight::Medium));
        // 0: the core does not know (Platform does not keep it with the identity).
        if id.created_at > 0 {
            let created = format!(
                tr!("{} · block {}"),
                text::date_long(id.created_at),
                group_thousands(id.created_block as i64)
            );
            kv_row(ui, &p, tr!("Made"), &created, value.clone());
        }
        kv_row(
            ui,
            &p,
            tr!("Revision"),
            &id.revision.to_string(),
            style(UI.weight(Weight::Medium).mono()),
        );
        kv_row(ui, &p, tr!("Source"), &id.key_source, value);
        if let Some(line) = dashpay_android_line(id.slot) {
            ui.add(crate::components::para(line, SMALL.size, SMALL.weight, p.muted).line_height(SMALL.line));
        }
    });
}

/// R-9 (FEAT-0014, AC-D5): DashPay on Android sees only the phrase's first identity, slot 0;
/// on any other this is said under the profile.
fn dashpay_android_line(slot: u32) -> Option<&'static str> {
    (slot != 0).then_some(tr!(
        "DashPay on Android shows only the first identity of this phrase: this one is not visible there."
    ))
}

/// Which identity the wallet acts as, and «+ New identity» last in the same row (FEAT-0014,
/// R-1: shown with one identity too); above it the banner after a creation, below it why a new
/// one cannot start now (R-4). Credits, contacts, profile, top-ups and names follow the choice.
fn identity_choice(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &mut Ctx,
    state: &mut IdentityState,
    asked: &mut Vec<Command>,
) {
    let wallet = cx.wallet();
    let Some(chosen) = &wallet.identity else {
        return;
    };
    let p = a.p;
    if let Some(made) = created_banner(&mut state.created, Some(chosen.id.as_str())) {
        created_notice(ui, a, chosen, &made, state);
    }
    // FEAT-0021 UC-5 (AC-T23): the contested name typed at the creation, claimed now.
    if let Some(label) = state.claim_after.clone() {
        let answer = crate::components::notice_closable(
            ui,
            a,
            "vote",
            &format!(tr!("Claim @{label} ›"), label = label),
            &format!(
                tr!(
                    "The identity is made. @{label} goes to a masternode vote: its review shows the write, the fund {fund} and when the result comes."
                ),
                label = label,
                fund = crate::format::amount(wallet.fees.name_contest_fund)
            ),
            Some(("chevron-right", tr!("Check the request"))),
        );
        if answer.action {
            state.claim_after = None;
            state.adding_name = Some(NameForm {
                name: label,
                review_asap: true,
                ..Default::default()
            });
        } else if answer.closed {
            state.claim_after = None;
        }
    }
    let blocked = new_identity_blocked(wallet, state);
    let mut picked = crate::components::Picked::default();
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        // «Work as» and the identities, wrapped within the page (BUG-265: the one picker, also
        // on Tokens and the Workshop); «+ New identity» only here.
        picked = crate::components::identity_picker(
            ui,
            a,
            wallet,
            tr!("Work as"),
            Some(blocked.is_none()),
            None,
        );
        // Why not, in words and without hovering (AC-D2), naming the identity to finish.
        if let Some(why) = &blocked {
            ui.add(
                crate::components::para(why, SMALL.size, SMALL.weight, p.muted)
                    .line_height(SMALL.line),
            );
        }
    });
    if let Some(id) = picked.chosen {
        asked.push(Command::ChooseIdentity { id });
        state.created = None;
    }
    if picked.new_one && blocked.is_none() {
        cx.go(Screen::CreateIdentity);
    }
}

/// Why «+ New identity» cannot start now (R-4), if it cannot: one operation at a time — a
/// creation cut short is finished first, and none starts beside a top-up.
fn new_identity_blocked(wallet: &Wallet, state: &IdentityState) -> Option<String> {
    if state.creating {
        return Some(
            tr!("Creating: asset lock, confirmation, identity and name — about a minute. Do not close evolite.").into(),
        );
    }
    if let Some(unfinished) = &wallet.unfinished_identity {
        return Some(match &unfinished.name {
            Some(name) => format!(
                tr!("Finish creating {} first: “Continue” is on its card"),
                crate::i18n::own(&format!("@{name}"))
            ),
            None => tr!("Finish creating the identity first: “Continue” is on its card").into(),
        });
    }
    // A running top-up only (R-4): a stopped one does not hold a creation back (the core makes
    // a new lock for it), and the pill never sends anyone to continue a top-up (review r1
    // must 2: werner555's stopped 0.5 DASH top-up must not be continued).
    top_up_blocked_by(true, None, state.topping_up().is_some()).map(String::from)
}

/// «Identity … created» (frame 5; without a name frame 6, with «Add a name»): actions now go
/// from it; «×» closes it.
fn created_notice(
    ui: &mut Ui,
    a: &mut Assets,
    chosen: &Identity,
    made: &str,
    state: &mut IdentityState,
) {
    let (title, text) = match chosen.name() {
        Some(name) => {
            let name = crate::i18n::own(&format!("@{name}"));
            (
                format!(tr!("Identity {} created"), name),
                format!(
                    tr!(
                        "Actions now go from {}: names, contacts, apps, NFTs. Switch in “Work as”."
                    ),
                    name
                ),
            )
        }
        None => (
            tr!("Identity created").to_string(),
            format!(
                tr!(
                    "Actions now go from identity {} (no name). Without a name nobody finds it by @name; you can add a name any time."
                ),
                crate::i18n::own(&short_id(made))
            ),
        ),
    };
    let action = chosen
        .name()
        .is_none()
        .then_some(("username", tr!("Add a name")));
    let answer = crate::components::notice_closable(ui, a, "check", &title, &text, action);
    if answer.closed {
        state.created = None;
    }
    if answer.action {
        state.adding_name = Some(NameForm::default());
    }
}

/// While identities are shown, a search still going on (BUG-9): more of them, or their
/// names, may come. A failed pass reads the same beside them: the core tries again by itself,
/// and an error next to the identities it found reads as if they were wrong. Without one the
/// empty state says it itself.
fn searching_text(search: &IdentitySearch, has_identity: bool) -> Option<&'static str> {
    (has_identity && *search != IdentitySearch::Done).then_some(tr!(
        "Looking for this phrase's other identities and their names on Platform…"
    ))
}

/// A search for the phrase's identities still going on, or, with none shown, a failed one: why,
/// and "Search again". Returns true when that was clicked.
fn search_status(ui: &mut Ui, a: &mut Assets, search: &IdentitySearch, has_identity: bool) -> bool {
    let p = a.p;
    if let Some(text) = searching_text(search, has_identity) {
        notice(ui, a, "pending", p.muted, text);
        return false;
    }
    let IdentitySearch::Failed(why) = search else {
        return false;
    };
    notice(
        ui,
        a,
        "warning",
        p.accent,
        &format!(
            tr!(
                "Could not ask Platform about this phrase's identities ({why}). evolite will try again by itself."
            ),
            why = why
        ),
    );
    button(ui, a, tr!("Search again"), ButtonKind::Secondary, None).clicked()
}

/// No identity shown and the search still running (CS-4): a neutral chip and what happens, no
/// buttons — «Create an identity» or a red «not enough» now could make a second one.
fn searching_for_identity(ui: &mut Ui, a: &mut Assets) {
    let p = a.p;
    let looking = tr!("Looking for this phrase's identity on Platform…");
    ui.horizontal(|ui| {
        // A chip is painted: a screen reader hears it by this name.
        let resp = chip(ui, a, "pending", looking, false);
        crate::components::named(&resp, egui::WidgetType::Label, true, looking);
    });
    ui.add(crate::components::para(
        tr!("We check the answer against masternode lists — usually under a minute. Nothing to do here yet."), BODY.size, BODY.weight, p.muted).line_height(BODY.line));
}

/// No identity after the search (or after a failed one, which [`search_status`] explains):
/// what an identity is, what it takes against what Core has, «Create an identity» and, after a
/// finished search, «Search again». `search_again`: «Search again» of a failed search clicked.
fn no_identity(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, wallet: &Wallet, search_again: bool) {
    let done = wallet.identity_search == IdentitySearch::Done;
    // BUG-262: the empty state of every screen that needs an identity; a creation cut short has
    // its own «Continue creating» above. «Search again» after a finished search stays: an
    // identity may have been made elsewhere (BUG-9).
    crate::screens::no_identity::draw(
        ui,
        a,
        cx,
        crate::screens::no_identity::Needs::Identity,
        false,
    );
    let mut again = search_again;
    if done && button(ui, a, tr!("Search again"), ButtonKind::Secondary, None).clicked() {
        again = true;
    }
    if again {
        cx.send(Command::FindIdentities);
    }
}

impl IdentityState {
    /// "Top up the identity" opens (from Identity or from Overview). The last top-up's answer
    /// stays only while that top-up is running.
    pub fn open_top_up(&mut self) {
        if self.topping_up().is_none() {
            self.top_up_result = None;
        }
        self.top_up = Some(TopUpForm::default());
    }

    /// "Top up" on the credits card: the form, or why it cannot open now — never nothing.
    pub fn top_up_clicked(&mut self, wallet: &Wallet) {
        self.top_up_clicked_when(top_up_blocked(wallet, self));
    }

    fn top_up_clicked_when(&mut self, blocked: Option<&'static str>) {
        match blocked {
            None => self.open_top_up(),
            // A top-up running shows its own notice already.
            Some(_) if self.topping_up().is_some() => {}
            Some(why) => {
                self.top_up_result = Some((0, Err(CoreError::NotAvailable(why.into()))));
            }
        }
    }
}

/// How many document writes `duffs` of credits pay for, «••••» while amounts are hidden: the
/// number gives the balance away (FEAT-0007, AC-D3).
fn operations(duffs: i64, fees: &Fees) -> String {
    if crate::format::display().hide {
        "••••".to_string()
    } else {
        group_thousands(duffs / fees.document_write.max(1))
    }
}

fn dash(d: i64) -> String {
    crate::format::amount(d)
}

/// A short action in a line of words (12 SemiBold, the accent): «Top up ›». One Tab stop, a
/// button for a screen reader.
fn inline_action(ui: &mut Ui, p: &crate::theme::Palette, label: &str) -> egui::Response {
    let resp = ui
        .add(egui::Button::new(text(label, SMALL.weight(Weight::SemiBold), p.accent)).frame(false))
        .on_hover_cursor(CursorIcon::PointingHand);
    crate::components::named(&resp, egui::WidgetType::Button, true, label);
    resp
}

/// A tile of the credits card that was clicked.
#[derive(Clone, Copy, PartialEq)]
enum CreditsAction {
    TopUp,
    Withdraw,
    Transfer,
}

/// The line under the credits' amount: how many credits and what they pay for; kept from an
/// earlier run and not checked in this one, that instead (BUG-115).
fn credits_note(credits: &str, operations: &str, checked: bool) -> String {
    if checked {
        format!(tr!("{} credits\n≈ {} operations"), credits, operations)
    } else {
        format!(tr!("{} credits\nnot checked yet"), credits)
    }
}

/// The credits card. Returns the tile clicked, if any.
fn credits(
    ui: &mut Ui,
    a: &mut Assets,
    balances: &Balances,
    fees: &Fees,
    width: f32,
    cell: &mut RowCell,
) -> Option<CreditsAction> {
    let duffs = balances.credits;
    let p = a.p;
    let mut clicked = None;
    cell.card(ui, card(&p, Margin::same(24)), |ui| {
        ui.set_width(inner(width, 24));
        ui.spacing_mut().item_spacing.y = 12.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.add(a.icons.icon("credits", p.muted, &p, 18.0));
            ui.label(text(
                tr!("Platform credits"),
                UI.weight(Weight::Medium),
                p.muted,
            ));
            // The app's one «Hide amounts» (FEAT-0007).
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                crate::components::amount_toggle(ui, a, false);
            });
        });
        // Kept from an earlier run, not checked in this one: dimmed (BUG-115).
        let color = if balances.credits_checked {
            p.text
        } else {
            p.text.gamma_multiply(0.6)
        };
        crate::components::amount_label(ui, &crate::format::amount(duffs), style(AMOUNT_L), color);
        // The credits and the operations they pay for give the balance away: hidden with it.
        let credits = if crate::format::display().hide {
            "••••".to_string()
        } else {
            group_thousands(duffs * CREDITS_PER_DUFF)
        };
        let operations = operations(duffs, fees);
        ui.label(
            egui::RichText::new(crate::i18n::t(&credits_note(
                &credits,
                &operations,
                balances.credits_checked,
            )))
            .font(style(SMALL.mono()))
            .color(p.muted),
        );
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let w = (ui.available_width() - 16.0) / 3.0;
            for (icon, label) in [
                ("top-up", tr!("Top up")),
                ("withdraw", tr!("Withdraw")),
                ("transfer", tr!("Transfer")),
            ] {
                let resp = tile(ui, a, icon, label, w);
                let action = match icon {
                    "top-up" => CreditsAction::TopUp,
                    "withdraw" => CreditsAction::Withdraw,
                    _ => CreditsAction::Transfer,
                };
                if resp.clicked() {
                    clicked = Some(action);
                }
            }
        });
    });
    clicked
}

/// A square-ish action: raised, radius 12, icon 18 over a 12 px label.
fn tile(ui: &mut Ui, a: &mut Assets, icon: &str, label: &str, width: f32) -> egui::Response {
    let p = a.p;
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 53.0), Sense::click());
    crate::components::named(&resp, egui::WidgetType::Button, true, label);
    let fill = if resp.hovered() { p.line } else { p.raised };
    ui.painter().rect_filled(rect, CornerRadius::same(12), fill);
    let icon_rect =
        Rect::from_center_size(pos2(rect.center().x, rect.top() + 19.0), vec2(18.0, 18.0));
    a.icons.icon(icon, p.text, &p, 18.0).paint_at(ui, icon_rect);
    ui.painter().text(
        pos2(rect.center().x, rect.bottom() - 10.0),
        Align2::CENTER_BOTTOM,
        crate::i18n::t(label),
        style(SMALL.weight(Weight::SemiBold)),
        p.text,
    );
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// What was pressed in «DPNS names» (FEAT-0021 N4).
enum NameAction {
    /// A claimed name's row: its contest card.
    Open(String),
    /// «Claim another ›».
    Another,
    /// «Hide» on a lost or blocked name (R-12).
    Hide(String),
}

fn names(
    ui: &mut Ui,
    a: &mut Assets,
    id: &Identity,
    fees: &evolite_api::Fees,
    width: f32,
    cell: &mut RowCell,
    action: &mut Option<NameAction>,
) {
    list_card_in(
        ui,
        a,
        cell,
        width,
        (
            tr!("DPNS names"),
            &id.names.iter().filter(|n| n.listed()).count().to_string(),
        ),
        |ui, a| {
            let p = a.p;
            for n in id.names.iter().filter(|n| n.listed()) {
                let name = &n.label;
                divider(ui, &p);
                ui.add_space(12.0);
                let row = ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    // FEAT-0021 N4: registered, in the vote, or how it ended.
                    let icon = if n.state.owned() { "username" } else { "vote" };
                    let sub = contest::state_line(n, fees, chrono::Local::now().timestamp());
                    ui.add(a.icons.icon(icon, p.text, &p, 20.0));
                    // The words stop short of the proof's icon on the right: a long state line
                    // (FEAT-0021) must not widen the card past its column, the grid's (BUG-46).
                    let words = (ui.available_width() - 20.0 - 12.0).max(0.0);
                    ui.vertical(|ui| {
                        ui.set_max_width(words);
                        ui.spacing_mut().item_spacing.y = 2.0;
                        // A name in the vote or lost is not this identity's: muted (N4). Not
                        // selectable: a claimed name's whole row takes the click (AC-D9).
                        ui.add(
                            egui::Label::new(text(
                                &format!("@{name}"),
                                BODY.weight(Weight::SemiBold),
                                if n.state.owned() { p.text } else { p.muted },
                            ))
                            .selectable(false)
                            .truncate(),
                        );
                        ui.add(
                            egui::Label::new(text(&sub, SMALL, p.muted))
                                .selectable(false)
                                .truncate(),
                        );
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let (icon, color, hint) = if n.state.open() {
                            ("pending", p.muted, tr!("Waiting for the masternode vote"))
                        } else if n.state.owned() {
                            (
                                "confirmed",
                                p.accent,
                                tr!("The name is confirmed by a Platform proof"),
                            )
                        } else {
                            (
                                "warning",
                                p.muted,
                                tr!("The vote is over: the name is not this identity's"),
                            )
                        };
                        ui.add(a.icons.icon(icon, color, &p, 18.0))
                            .on_hover_text(crate::i18n::t(hint).into_owned());
                    });
                });
                // FEAT-0021 N4: a claimed name's row opens its contest card, the whole row
                // (AC-D9, entry 1).
                if n.contest.is_some() {
                    // Not `row.response.interact(..)`: Tab lost the focus there (BUG-40).
                    let open = crate::a11y::row_click(ui, &row.response, "contest")
                        .on_hover_cursor(CursorIcon::PointingHand);
                    // A Tab stop with a name (the Tab sweep): the name and its line, a button
                    // Enter presses too.
                    let spoken = format!(
                        "@{name}: {}",
                        crate::i18n::spoken(&contest::state_line(
                            n,
                            fees,
                            chrono::Local::now().timestamp()
                        ))
                    );
                    open.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &spoken)
                    });
                    if open.clicked() {
                        *action = Some(NameAction::Open(name.clone()));
                    }
                }
                // A lost or blocked one: «Claim another ›» and «Hide», a line of their own, below
                // the row, so the row's click does not cover them (R-12).
                if matches!(
                    n.state,
                    evolite_api::NameState::Lost { .. } | evolite_api::NameState::Locked { .. }
                ) {
                    ui.horizontal(|ui| {
                        ui.add_space(32.0);
                        ui.spacing_mut().item_spacing.x = 16.0;
                        if inline_action(ui, &p, tr!("Claim another ›")).clicked() {
                            *action = Some(NameAction::Another);
                        }
                        let hide = ui
                            .add(
                                egui::Button::new(text(
                                    tr!("Hide"),
                                    SMALL.weight(Weight::Medium),
                                    p.muted,
                                ))
                                .frame(false),
                            )
                            .on_hover_cursor(CursorIcon::PointingHand);
                        if hide.clicked() {
                            *action = Some(NameAction::Hide(name.clone()));
                        }
                    });
                }
                ui.add_space(12.0);
            }
            // A nameless identity (FEAT-0014, frame 6): what a name would give.
            if !id.names.iter().any(|n| n.listed()) {
                divider(ui, &p);
                ui.add(
                    crate::components::para(
                        tr!("No names yet. With a name the identity can be found by @name and added to contacts."), UI.size, UI.weight, p.muted).line_height(UI.line),
                );
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::{
        Created, IdentityState, TOP_UP_SETTLED, TopUpForm, created_banner, searching_text,
        top_up_blocked_by, top_up_done_text, top_up_error, unfinished_top_up_text,
    };
    use evolite_api::CoreError;
    use evolite_api::IdentitySearch;

    /// BUG-182 (the rule of BUG-172): «Top up» opens empty, no amount put into what gets signed,
    /// no quick amount chosen.
    #[test]
    fn the_top_up_opens_empty() {
        let mut state = IdentityState::default();
        state.open_top_up();
        let form: &TopUpForm = state.top_up.as_ref().expect("the form");
        assert!(form.amount.is_empty(), "{:?}", form.amount);
        assert_eq!(form.suggested, None);
    }

    /// BUG-115: credits kept from an earlier run say they are not checked, not what they pay for.
    #[test]
    fn credits_not_checked_yet_say_so_on_the_identity_card() {
        assert_eq!(
            super::credits_note("475 002", "4", true),
            "475 002 credits\n≈ 4 operations"
        );
        assert_eq!(
            super::credits_note("475 002", "4", false),
            "475 002 credits\nnot checked yet"
        );
    }
    use evolite_api::UnfinishedTopUp;

    /// FEAT-0014 (analyst-44), review r1 should 7 (mutation B4): the banner waits for the
    /// snapshot to have the new identity chosen; then it is shown for it; once another one is
    /// chosen it closes by itself.
    #[test]
    fn the_created_banner_waits_for_the_new_identity_to_be_chosen() {
        let mut created = Some(Created::Awaiting(Some("old".into())));
        assert_eq!(
            created_banner(&mut created, Some("old")),
            None,
            "still the old"
        );
        assert_eq!(
            created,
            Some(Created::Awaiting(Some("old".into()))),
            "waits"
        );
        assert_eq!(
            created_banner(&mut created, Some("new")),
            Some("new".into()),
            "the new one chosen"
        );
        assert_eq!(
            created_banner(&mut created, Some("new")),
            Some("new".into()),
            "stays while it is chosen"
        );
        assert_eq!(created_banner(&mut created, Some("other")), None, "another");
        assert_eq!(created, None, "closed");
        assert_eq!(created_banner(&mut created, Some("new")), None, "for good");
        // The first identity of the wallet: nothing was chosen before.
        let mut first = Some(Created::Awaiting(None));
        assert_eq!(created_banner(&mut first, None), None);
        assert_eq!(created_banner(&mut first, Some("new")), Some("new".into()));
    }

    #[test]
    fn top_up_errors_are_words_not_debug() {
        for e in [
            CoreError::NotAvailable(tr!("Make an identity first").into()),
            CoreError::NotSynced,
            CoreError::NotEnough,
            CoreError::BadAmount,
            CoreError::Failed("timeout".into()),
            CoreError::WrongPassword,
        ] {
            let text = top_up_error(&e);
            assert!(!text.contains("NotAvailable") && !text.contains("WrongPassword"));
            assert!(text.ends_with('.') || text.ends_with('»'), "{text}");
        }
    }

    /// BUG-7 review: the core's reason gets one full stop, the language's own, from the
    /// catalog (BUG-15) — not a Latin one after a translation that already ends with its own.
    #[test]
    fn a_core_reason_gets_one_full_stop() {
        use crate::i18n::{Lang, t_in};
        let key = tr!("Make an identity first");
        let text = top_up_error(&CoreError::NotAvailable(format!("{key}.")));
        assert_eq!(text, format!("{key}."));
        let ru = t_in(Lang::Ru, key);
        assert_ne!(ru, key, "ru has the key");
        assert_eq!(t_in(Lang::Ru, &text), format!("{ru}."));
    }

    /// BUG-7 review: a lock Platform took on an earlier attempt is done, but does not read
    /// "Credits added"; any other answer is not done.
    #[test]
    fn a_settled_top_up_does_not_say_credits_added() {
        let added = top_up_done_text(50_000_000, &Ok(())).unwrap();
        assert!(added.starts_with("Credits added"), "{added}");
        let settled = top_up_done_text(
            50_000_000,
            &Err(CoreError::NotAvailable(TOP_UP_SETTLED.into())),
        )
        .unwrap();
        assert!(
            !settled.contains("Credits added") && settled.ends_with('.'),
            "{settled}"
        );
        assert_eq!(
            top_up_done_text(0, &Err(CoreError::NotAvailable("wait".into()))),
            None
        );
        assert_eq!(top_up_done_text(0, &Err(CoreError::NotEnough)), None);
        // And it does not cut the stopped top-up's notice short as if it explained a wait.
        let last = (0, Err(CoreError::NotAvailable(TOP_UP_SETTLED.into())));
        let (full, _) = unfinished_top_up_text(&stopped(true).unwrap(), None);
        let (after, _) = unfinished_top_up_text(&stopped(true).unwrap(), Some(&last));
        assert_eq!(after, full);
    }

    /// BUG-9: with identities shown, a search still going on is visible — more of them or their
    /// names may come; a failed pass reads the same there, not as an error (r1). Once done, or
    /// with no identity yet, this line is not shown: the empty state tells it, failure included.
    #[test]
    fn a_search_still_going_on_is_visible_beside_the_identities() {
        let failed = IdentitySearch::Failed("timeout".into());
        assert!(searching_text(&IdentitySearch::Searching, true).is_some());
        assert_eq!(
            searching_text(&failed, true),
            searching_text(&IdentitySearch::Searching, true)
        );
        assert_eq!(searching_text(&IdentitySearch::Searching, false), None);
        assert_eq!(searching_text(&failed, false), None);
        assert_eq!(searching_text(&IdentitySearch::Done, true), None);
    }

    #[test]
    fn a_top_up_answer_ends_the_running_one() {
        let mut state = IdentityState::default();
        let Some(evolite_api::Command::Spend { ask, .. }) = state.top_up_flow.command(
            evolite_api::Spend::TopUpIdentity { amount: 100_000 },
            false,
            100_000,
        ) else {
            panic!("sent");
        };
        assert_eq!(state.topping_up(), Some(100_000));
        // Another window's answer (another wallet's, BUG-214) is not this one's.
        state.on_top_up_done(Some(ask + 1), &Ok(()));
        assert_eq!(state.topping_up(), Some(100_000));
        state.on_top_up_done(Some(ask), &Ok(()));
        assert_eq!(state.topping_up(), None);
        assert_eq!(state.top_up_result, Some((100_000, Ok(()))));
    }

    fn stopped(awaiting_chain_lock: bool) -> Option<UnfinishedTopUp> {
        Some(UnfinishedTopUp {
            locked: 50_000_000,
            awaiting_chain_lock,
        })
    }

    /// BUG-7: a lock Platform already took does not block a new top-up; one to continue does.
    #[test]
    fn only_a_top_up_to_continue_holds_back_a_new_one() {
        assert_eq!(top_up_blocked_by(true, None, false), None);
        assert_eq!(top_up_blocked_by(true, stopped(true), false), None);
        assert_eq!(
            top_up_blocked_by(true, stopped(false), false),
            Some(tr!("First finish the stopped top-up"))
        );
        assert!(top_up_blocked_by(true, None, true).is_some());
        assert!(top_up_blocked_by(false, None, false).is_some());
    }

    /// BUG-7 (report item 31): "Top up" on the credits card says why it cannot open.
    #[test]
    fn a_blocked_top_up_says_why() {
        let mut state = IdentityState::default();
        state.top_up_clicked_when(Some(tr!("First finish the stopped top-up")));
        assert!(state.top_up.is_none());
        assert_eq!(
            state.top_up_result,
            Some((
                0,
                Err(CoreError::NotAvailable(
                    tr!("First finish the stopped top-up").into()
                ))
            ))
        );
        state.top_up_clicked_when(None);
        assert!(state.top_up.is_some());
    }

    /// BUG-7: the two kinds of a stopped top-up read and act differently.
    #[test]
    fn a_stopped_top_up_offers_continue_or_check_again() {
        let (text, action) = unfinished_top_up_text(&stopped(false).unwrap(), None);
        assert_eq!(action, tr!("Continue the top-up"));
        assert!(text.contains("no new money"));
        let (text, action) = unfinished_top_up_text(&stopped(true).unwrap(), None);
        assert_eq!(action, tr!("Check again"));
        assert!(text.contains("ChainLock") && text.contains("top up again"));
        // The answer above already explains the wait: a short line.
        let last = (0, Err(CoreError::NotAvailable("wait".into())));
        let (short, _) = unfinished_top_up_text(&stopped(true).unwrap(), Some(&last));
        assert!(short.len() < text.len());
    }

    /// Design review №16: «Save» waits for a change — a write costs credits.
    #[test]
    fn an_unchanged_profile_has_nothing_to_save() {
        use super::ProfileForm;
        use evolite_api::Profile;
        let saved = Profile {
            display: Some("Satoshi".into()),
            ..Default::default()
        };
        let mut form = ProfileForm::from(Some(&saved));
        assert!(!form.changes(Some(&saved)));
        form.display.push(' ');
        assert!(
            !form.changes(Some(&saved)),
            "spaces at the end are not a change"
        );
        form.about = "Dash".into();
        assert!(form.changes(Some(&saved)));
        // No profile yet: empty fields write nothing new.
        assert!(!ProfileForm::from(None).changes(None));
    }
}
