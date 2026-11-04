//! 06 · Identity: profile with the verified name, credits balance and actions, DPNS names and
//! keys, all from the core's snapshot.

use super::{Screen, txt};
use crate::components::{
    Assets, ButtonKind, FieldKind, amount_field, avatar, button, button_enabled, card, chip,
    dialog, divider, field_label, inner, kv_row, list_card, notice, page_header, pill, text_field,
    two_columns,
};
use crate::context::Ctx;
use crate::text::{self, group_thousands, short_id};
use crate::theme::{Weight, mono, sans};
use egui::{Align, Align2, CornerRadius, CursorIcon, Layout, Margin, Rect, Sense, Ui, pos2, vec2};
use evolite_api::{
    Balances, Command, CoreError, Fees, Identity, IdentitySearch, NameCheck, Profile, Source,
    Wallet,
};

/// 1 duff = 1000 credits on Platform.
const CREDITS_PER_DUFF: i64 = 1_000;

/// How long "copied" stands in place of the ID, in seconds.
const COPIED_FOR: f64 = 1.5;

#[derive(Default)]
pub struct IdentityState {
    /// When the ID was copied (seconds of `ui.input(|i| i.time)`): "copied" for a moment.
    pub copied_at: Option<f64>,
    /// "Edit profile" (23) is open.
    pub editing: Option<ProfileForm>,
    /// "Top up the identity" is open; the top-up itself outlives the window.
    pub top_up: Option<TopUpForm>,
    /// The duffs of a top-up sent to the core, no answer yet.
    pub topping_up: Option<i64>,
    /// The last top-up's duffs and answer, until a new one starts.
    pub top_up_result: Option<(i64, Result<(), CoreError>)>,
    /// "Add a name" is open.
    pub adding_name: Option<NameForm>,
}

/// "Add a name" as typed: the label, the core's check of it, and the registration's answer.
#[derive(Default)]
pub struct NameForm {
    pub name: String,
    /// The label last sent to `CheckName`, and its answer.
    pub asked: String,
    pub check: Option<Result<NameCheck, CoreError>>,
    /// Sent to the core, no answer yet.
    pub waiting: bool,
    pub result: Option<Result<(), CoreError>>,
}

/// "Top up the identity" as typed, before it is sent.
pub struct TopUpForm {
    pub amount: String,
    /// 0.1 DASH in `amount` until the person changes it (`format::fill_suggested`).
    pub suggested: Option<i64>,
}

impl Default for TopUpForm {
    fn default() -> Self {
        Self {
            amount: crate::format::input_text(10_000_000),
            suggested: Some(10_000_000),
        }
    }
}

/// The DashPay profile as typed in 23, before it is written.
#[derive(Default)]
pub struct ProfileForm {
    pub display: String,
    pub about: String,
    pub avatar_url: String,
    /// Sent to the core, no answer yet.
    pub waiting: bool,
    pub result: Option<Result<(), CoreError>>,
}

impl ProfileForm {
    /// The form filled with what the profile says now.
    pub fn from(profile: Option<&Profile>) -> Self {
        let profile = profile.cloned().unwrap_or_default();
        Self {
            display: profile.display.unwrap_or_default(),
            about: profile.about.unwrap_or_default(),
            avatar_url: profile.avatar_url.unwrap_or_default(),
            ..Default::default()
        }
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

    /// The core's answer to `AddName`.
    pub fn on_name_done(&mut self, result: &Result<(), CoreError>) {
        if let Some(form) = &mut self.adding_name {
            form.waiting = false;
            form.result = Some(result.clone());
        }
    }

    /// The core's answer to `TopUpIdentity` or `ResumeTopUp`.
    pub fn on_top_up_done(&mut self, result: &Result<(), CoreError>) {
        let duffs = self.topping_up.take().unwrap_or(0);
        self.top_up_result = Some((duffs, result.clone()));
    }

    pub fn on_profile_saved(&mut self, result: &Result<(), CoreError>) {
        if let Some(form) = &mut self.editing {
            form.waiting = false;
            if result.is_ok() {
                self.editing = None;
            } else {
                form.result = Some(result.clone());
            }
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
    identity_choice(ui, a, wallet, &mut asked);
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
        let p = a.p;
        let searching = wallet.identity_search == IdentitySearch::Searching;
        let text = if searching {
            tr!(
                "Looking for this phrase's identities on Platform: the answer is checked against the masternode lists."
            )
        } else {
            tr!("No identity yet. It is made from DASH on Core: a name, keys and credits for apps.")
        };
        ui.label(txt(text, 14.0, Weight::Regular, p.muted));
        // What it takes, against what there is: the name and the creation, plus any credits.
        let least = wallet.fees.name_regular + wallet.fees.identity_create;
        ui.label(txt(
            &format!(
                tr!("At least ≈ {} is needed (name and creation) plus any amount for credits; Core has {} now."),
                dash(least),
                dash(wallet.balances.core)
            ),
            13.0,
            Weight::Regular,
            if wallet.balances.core > least {
                p.muted
            } else {
                p.danger
            },
        ));
        let mut again = search_again;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            if button(ui, a, tr!("Create an identity"), ButtonKind::Primary, None).clicked() {
                cx.go(Screen::CreateIdentity);
            }
            // An identity made elsewhere (Jembe, Dash Evo Tool) after the last search.
            if wallet.identity_search == IdentitySearch::Done
                && button(ui, a, tr!("Search again"), ButtonKind::Secondary, None).clicked()
            {
                again = true;
            }
        });
        if again {
            cx.send(Command::FindIdentities);
        }
        return;
    };
    if search_again {
        asked.push(Command::FindIdentities);
    }

    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 20.0;
            let total = ui.available_width();
            // Two rows of two cards; below 760 px of content everything goes in one column.
            let wide = total >= 760.0;
            let credits_w = if wide { 340.0 } else { total };
            let profile_w = if wide {
                total - credits_w - 20.0
            } else {
                total
            };
            let half = if wide { (total - 20.0) / 2.0 } else { total };

            two_columns(ui, wide, |ui, first| {
                if first {
                    profile(ui, a, state, identity, profile_w, &mut asked);
                } else {
                    match credits(ui, a, &wallet.balances, &wallet.fees, credits_w) {
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
            two_columns(ui, wide, |ui, first| {
                if first {
                    names(ui, a, identity, half);
                } else {
                    keys(ui, a, identity, half);
                }
            });
        });
    if state.editing.is_some() {
        edit_profile(ui, a, state, identity, wallet.fees.profile, &mut asked);
    }
    if state.top_up.is_some() {
        top_up_dialog(ui, a, state, wallet, identity, &mut asked);
    }
    if state.adding_name.is_some() {
        add_name_dialog(ui, a, state, wallet, identity, &mut asked);
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
    asked: &mut Vec<Command>,
) {
    let p = a.p;
    let name = id.name().map(|n| format!("@{n}"));
    let display = id.profile.as_ref().and_then(|pr| pr.display.clone());
    card(&p, Margin::same(24)).show(ui, |ui| {
        ui.set_width(inner(width, 24));
        ui.spacing_mut().item_spacing.y = 14.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            let (face, _) = ui.allocate_exact_size(vec2(72.0, 72.0), Sense::hover());
            let label = display.clone().or_else(|| name.clone()).unwrap_or_default();
            let picture = id.profile.as_ref().and_then(|pr| pr.avatar.as_deref());
            avatar(ui, a, face, &id.id, &label, picture, &mut |c| asked.push(c));
            let edit_w = 170.0;
            ui.allocate_ui_with_layout(
                vec2((ui.available_width() - edit_w - 16.0).max(120.0), 72.0),
                Layout::top_down(Align::Min),
                |ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        match &name {
                            Some(name) => {
                                ui.label(txt(name, 22.0, Weight::Bold, p.text));
                                chip(ui, a, "confirmed", tr!("Name verified"), true);
                            }
                            None => {
                                ui.label(txt(tr!("No name"), 22.0, Weight::Bold, p.text));
                            }
                        }
                    });
                    if let Some(display) = &display {
                        ui.label(txt(display, 15.0, Weight::Medium, p.text));
                    }
                    // The ID with a copy icon; the icon turns into "copied" for a moment.
                    let now = ui.input(|i| i.time);
                    let copied = state.copied_at.is_some_and(|at| now - at < COPIED_FOR);
                    if copied {
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_millis(200));
                    }
                    let label = if copied {
                        tr!("copied").to_string()
                    } else {
                        short_id(&id.id)
                    };
                    let resp = ui
                        .horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            ui.label(
                                egui::RichText::new(crate::i18n::t(&label))
                                    .font(mono(13.0, Weight::Regular))
                                    .color(p.muted),
                            );
                            ui.add(a.icons.icon("copy", p.muted, &p, 14.0));
                        })
                        .response
                        .interact(Sense::click())
                        .on_hover_cursor(CursorIcon::PointingHand)
                        .on_hover_text(crate::i18n::t(&id.id).into_owned());
                    crate::components::named(
                        &resp,
                        egui::WidgetType::Button,
                        true,
                        &format!("{} · {}", crate::i18n::t(tr!("Copy")), id.id),
                    );
                    if resp.clicked() {
                        crate::clipboard::copy(ui.ctx(), id.id.clone());
                        state.copied_at = Some(now);
                    }
                },
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let label = if id.profile.is_some() {
                    tr!("Edit profile")
                } else {
                    tr!("Create a profile")
                };
                if button(ui, a, label, ButtonKind::Ghost, None).clicked() {
                    state.editing = Some(ProfileForm::from(id.profile.as_ref()));
                }
            });
        });
        match &id.profile {
            Some(profile) => super::contacts::about_block(ui, a, profile),
            None => {
                ui.label(txt(
                    tr!("No DashPay profile yet: your contacts will see the name, «About» and avatar."),
                    13.0,
                    Weight::Regular,
                    p.muted,
                ));
            }
        }
        divider(ui, &p);
        let value = sans(13.0, Weight::Medium);
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
            mono(13.0, Weight::Medium),
        );
        kv_row(ui, &p, tr!("Source"), &id.key_source, value);
    });
}

/// DashPay's limits on a profile (DIP-15).
const DISPLAY_MAX: usize = 25;
const ABOUT_MAX: usize = 140;

/// Cuts `s` to `max` characters (not bytes).
fn keep_chars(s: &mut String, max: usize) {
    if let Some((cut, _)) = s.char_indices().nth(max) {
        s.truncate(cut);
    }
}

/// 23 · Edit profile: the picture's link, the name and "Bio", what writing it costs.
fn edit_profile(
    ui: &mut Ui,
    a: &mut Assets,
    state: &mut IdentityState,
    id: &Identity,
    fee: i64,
    asked: &mut Vec<Command>,
) {
    let Some(form) = &mut state.editing else {
        return;
    };
    let me = id
        .name()
        .map_or_else(|| "identity".to_string(), |n| format!("@{n}"));
    let mut save = false;
    let close = dialog(
        ui,
        a,
        "edit-profile",
        "identity",
        tr!("DashPay profile"),
        464.0,
        |ui, a| {
            let p = a.p;
            let count =
                |n: usize, max: usize| format!(tr!("{n} of {max} characters"), n = n, max = max);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 16.0;
                let (face, _) = ui.allocate_exact_size(vec2(72.0, 72.0), Sense::hover());
                let label = if form.display.trim().is_empty() {
                    me.clone()
                } else {
                    form.display.clone()
                };
                let picture = id.profile.as_ref().and_then(|pr| pr.avatar.as_deref());
                avatar(ui, a, face, &id.id, &label, picture, &mut |c| asked.push(c));
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    field_label(ui, &p, tr!("Picture link"));
                    text_field(
                        ui,
                        &p,
                        "profile-avatar",
                        &mut form.avatar_url,
                        "https://…",
                        FieldKind::Text,
                        |_| {},
                    );
                    ui.label(txt(
                        tr!("PNG, JPEG, GIF, WebP or SVG, up to 1 MB, over https"),
                        12.0,
                        Weight::Regular,
                        p.muted,
                    ));
                });
            });
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                field_label(ui, &p, tr!("Name"));
                text_field(
                    ui,
                    &p,
                    "profile-display",
                    &mut form.display,
                    tr!("What to call you"),
                    FieldKind::Text,
                    |_| {},
                );
                keep_chars(&mut form.display, DISPLAY_MAX);
                ui.label(txt(
                    &count(form.display.chars().count(), DISPLAY_MAX),
                    12.0,
                    Weight::Regular,
                    p.muted,
                ));
            });
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                field_label(ui, &p, tr!("Bio"));
                egui::Frame::new()
                    .fill(p.raised)
                    .stroke(egui::Stroke::new(1.0, p.field_line))
                    .corner_radius(10)
                    .inner_margin(Margin::symmetric(14, 12))
                    .show(ui, |ui| {
                        crate::rtl::text_edit(
                            ui,
                            egui::TextEdit::multiline(&mut form.about)
                                .hint_text(txt(
                                    tr!("A few words about yourself"),
                                    14.0,
                                    Weight::Regular,
                                    p.muted,
                                ))
                                .font(sans(14.0, Weight::Regular))
                                .text_color(p.text)
                                .frame(egui::Frame::NONE)
                                .desired_rows(3)
                                .desired_width(f32::INFINITY),
                        );
                    });
                keep_chars(&mut form.about, ABOUT_MAX);
                ui.label(txt(
                    &count(form.about.chars().count(), ABOUT_MAX),
                    12.0,
                    Weight::Regular,
                    p.muted,
                ));
            });
            let value = mono(13.0, Weight::Medium);
            kv_row(
                ui,
                &p,
                tr!("A write to Platform"),
                &format!("≈ {}", crate::format::amount(fee)),
                value.clone(),
            );
            kv_row(
                ui,
                &p,
                tr!("Paid by"),
                &format!(tr!("{me} · credits"), me = me),
                value,
            );
            notice(
                ui,
                a,
                "eye",
                p.muted,
                tr!(
                    "Anyone who knows your @name sees the name, «About» and avatar. The picture stays at its link: evolite has no servers of its own."
                ),
            );
            let url = form.avatar_url.trim();
            let url_ok = url.is_empty() || url.starts_with("https://");
            if !url_ok {
                ui.label(txt(
                    tr!("The link must start with https://"),
                    12.0,
                    Weight::Regular,
                    p.danger,
                ));
            }
            if let Some(Err(e)) = &form.result {
                notice(
                    ui,
                    a,
                    "warning",
                    p.danger,
                    &super::contacts::contact_error(e),
                );
            }
            let mut close = false;
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let label = if form.waiting {
                    tr!("Saving…")
                } else {
                    tr!("Save")
                };
                if button_enabled(
                    ui,
                    a,
                    label,
                    ButtonKind::Primary,
                    None,
                    url_ok && !form.waiting,
                )
                .clicked()
                {
                    save = true;
                }
                if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                    close = true;
                }
            });
            close
        },
    );
    if save {
        let url = form.avatar_url.trim();
        asked.push(Command::SaveProfile {
            display: form.display.trim().to_owned(),
            about: form.about.trim().to_owned(),
            avatar_url: (!url.is_empty()).then(|| url.to_owned()),
        });
        form.waiting = true;
        form.result = None;
    } else if close && !form.waiting {
        state.editing = None;
    }
}

/// With more than one identity: which one the wallet acts as. Credits, contacts,
/// profile, top-ups and names follow the choice.
fn identity_choice(ui: &mut Ui, a: &mut Assets, wallet: &Wallet, asked: &mut Vec<Command>) {
    if wallet.identities.len() < 2 {
        return;
    }
    let p = a.p;
    let shown = wallet.identity.as_ref().map(|i| i.id.as_str());
    // Rows as tall as a pill, everything centred in them; many identities wrap onto
    // more rows instead of pushing the page past the window.
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(ui.available_width());
        ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
        ui.label(txt(tr!("Work as"), 13.0, Weight::Medium, p.muted));
        for choice in &wallet.identities {
            let label = choice
                .name
                .as_ref()
                .map_or_else(|| short_id(&choice.id), |n| format!("@{n}"));
            let on = shown == Some(choice.id.as_str());
            if pill(ui, &p, &label, on)
                .on_hover_text(crate::i18n::t(&choice.id).into_owned())
                .clicked()
                && !on
            {
                asked.push(Command::ChooseIdentity {
                    id: choice.id.clone(),
                });
            }
        }
    });
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

/// Why "Top up" cannot start now, if it cannot.
fn top_up_blocked(wallet: &Wallet, state: &IdentityState) -> Option<&'static str> {
    top_up_blocked_by(
        wallet.identity.is_some(),
        wallet.unfinished_top_up,
        state.topping_up.is_some(),
    )
}

/// [`top_up_blocked`] on its own facts. A top-up Platform already took (only its ChainLock is
/// awaited) does not hold a new one back.
fn top_up_blocked_by(
    has_identity: bool,
    unfinished: Option<evolite_api::UnfinishedTopUp>,
    running: bool,
) -> Option<&'static str> {
    if !has_identity {
        Some(tr!("Make an identity first"))
    } else if running {
        Some(tr!("A top-up is already running"))
    } else if unfinished.is_some_and(|u| !u.awaiting_chain_lock) {
        Some(tr!("First finish the stopped top-up"))
    } else {
        None
    }
}

fn open_top_up(state: &mut IdentityState) {
    state.open_top_up();
}

impl IdentityState {
    /// "Top up the identity" opens (from Identity or from Overview). The last top-up's answer
    /// stays only while that top-up is running.
    pub fn open_top_up(&mut self) {
        if self.topping_up.is_none() {
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
            Some(_) if self.topping_up.is_some() => {}
            Some(why) => {
                self.top_up_result = Some((0, Err(CoreError::NotAvailable(why.into()))));
            }
        }
    }
}

fn dash(d: i64) -> String {
    crate::format::amount(d)
}

/// Why the top-up did not go through, in words.
fn top_up_error(e: &CoreError) -> String {
    match e {
        // The core's reason is a catalog key: the catalog adds the language's own full stop.
        CoreError::NotAvailable(why) => format!("{}.", why.trim_end_matches('.')),
        CoreError::NotSynced => {
            tr!("Wait for the sync to finish: all the wallet's coins and the masternode lists are needed.").into()
        }
        CoreError::NotEnough => tr!("Not enough DASH on Core: the amount and the fee are needed.").into(),
        CoreError::BadAmount => tr!("The amount must be more than zero.").into(),
        other => format!(
            tr!("{} If the DASH are already locked, evolite shows «Continue the top-up»."),
            crate::text::failed(tr!("Not topped up"), other)
        ),
    }
}

/// The core's answer to a top-up whose lock Platform took on an earlier attempt: the same
/// catalog key as `top_up_settled_text` in evolite-core `real/chain.rs`.
const TOP_UP_SETTLED: &str = tr!(
    "Platform says it took these DASH on an earlier attempt, so the credits are already on the identity. No new money is needed"
);

/// The top-up is done: what to say. `None`: it did not go through ([`top_up_error`]). A lock
/// taken on an earlier attempt is done too, but no credits were added by this one (BUG-7 review).
fn top_up_done_text(duffs: evolite_api::Duffs, result: &Result<(), CoreError>) -> Option<String> {
    match result {
        Ok(()) => Some(format!(tr!("Credits added: +{}."), dash(duffs))),
        Err(CoreError::NotAvailable(why)) if why == TOP_UP_SETTLED => Some(format!("{why}.")),
        Err(_) => None,
    }
}

/// The answer to the last top-up: done, or why not with the network's words under "Details".
fn top_up_answer(
    ui: &mut Ui,
    a: &mut Assets,
    duffs: evolite_api::Duffs,
    result: &Result<(), CoreError>,
) {
    match (top_up_done_text(duffs, result), result) {
        (Some(text), _) => {
            let p = a.p;
            notice(ui, a, "confirmed", p.accent, &text);
        }
        (None, Err(e)) => crate::components::error_notice(
            ui,
            a,
            &top_up_error(e),
            crate::text::error_details(e).as_deref(),
        ),
        (None, Ok(())) => {}
    }
}

/// Above the cards: a top-up still running with its window closed, its answer, and a top-up
/// cut short with "Continue the top-up".
fn top_up_status(
    ui: &mut Ui,
    a: &mut Assets,
    wallet: &Wallet,
    state: &mut IdentityState,
    asked: &mut Vec<Command>,
) {
    let p = a.p;
    if state.top_up.is_none() {
        match (&state.topping_up, &state.top_up_result) {
            (Some(duffs), _) => notice(
                ui,
                a,
                "pending",
                p.muted,
                &format!(
                    tr!(
                        "Topping up the identity by {}: asset lock, InstantSend, Platform — usually under a minute."
                    ),
                    dash(*duffs)
                ),
            ),
            (None, Some((duffs, result))) => top_up_answer(ui, a, *duffs, result),
            (None, None) => {}
        }
    }
    let Some(unfinished) = wallet.unfinished_top_up else {
        return;
    };
    if state.topping_up.is_some() {
        return;
    }
    let (text, action) = unfinished_top_up_text(&unfinished, state.top_up_result.as_ref());
    notice(ui, a, "warning", p.accent, &text);
    if button(ui, a, action, ButtonKind::Primary, None).clicked() {
        asked.push(Command::ResumeTopUp);
        state.topping_up = Some(unfinished.locked);
        state.top_up_result = None;
    }
}

/// The notice of a cut-short top-up and its button: one that can be continued, or one
/// Platform already took and whose ChainLock is awaited ("Check again"). When the last answer
/// already explains the wait, the notice does not repeat it.
fn unfinished_top_up_text(
    unfinished: &evolite_api::UnfinishedTopUp,
    last: Option<&(evolite_api::Duffs, Result<(), CoreError>)>,
) -> (String, &'static str) {
    if unfinished.awaiting_chain_lock {
        let explained =
            matches!(last, Some((_, Err(CoreError::NotAvailable(why)))) if why != TOP_UP_SETTLED);
        let text = if explained {
            format!(
                tr!("{} of the top-up are awaiting the ChainLock of their block."),
                dash(unfinished.locked)
            )
        } else {
            format!(
                tr!(
                    "Platform has already taken the {} of the stopped top-up: the credits should be on the identity. evolite confirms it once their block gets a ChainLock, usually within a few minutes. You can top up again meanwhile."
                ),
                dash(unfinished.locked)
            )
        };
        (text, tr!("Check again"))
    } else {
        (
            format!(
                tr!(
                    "The top-up stopped: {} are already locked on Core. Continue — no new money is needed."
                ),
                dash(unfinished.locked)
            ),
            tr!("Continue the top-up"),
        )
    }
}

/// "Top up the identity": DASH from Core into the identity's credits, what it costs and what
/// it gives. Closing the window does not stop a top-up that is running.
fn top_up_dialog(
    ui: &mut Ui,
    a: &mut Assets,
    state: &mut IdentityState,
    wallet: &Wallet,
    id: &Identity,
    asked: &mut Vec<Command>,
) {
    let Some(form) = &mut state.top_up else {
        return;
    };
    crate::format::fill_suggested(&mut form.amount, &mut form.suggested);
    let me = id
        .name()
        .map_or_else(|| "identity".to_string(), |n| format!("@{n}"));
    let fees = &wallet.fees;
    let core = wallet.balances.core;
    // The asset lock is one Core transaction: its fee is about a typical payment's.
    let fee = fees.core_payment;
    let running = state.topping_up;
    let mut send = None;
    let close = dialog(
        ui,
        a,
        "top-up",
        "top-up",
        tr!("Top up the identity"),
        464.0,
        |ui, a| {
            let p = a.p;
            ui.add(
                egui::Label::new(
                    txt(
                        &format!(
                            tr!("DASH from Core become credits of {me}: they pay for writes to Platform — names, contacts, profile.")
                        , me = me),
                        13.0,
                        Weight::Regular,
                        p.muted,
                    )
                    .line_height(Some(20.0)),
                )
                .wrap(),
            );
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                field_label(ui, &p, tr!("How much to turn into credits"));
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    for duffs in [10_000_000, 50_000_000, 100_000_000] {
                        let label = crate::format::short_number(duffs);
                        let on = crate::format::parse_amount(&form.amount) == Some(duffs);
                        if pill(ui, &p, &label, on).clicked() {
                            form.amount = crate::format::input_text(duffs);
                        }
                    }
                });
                let field = amount_field(ui, &p, "top-up-amount", &mut form.amount, true);
                if field.all {
                    form.amount = crate::format::input_text((core - fee).max(0));
                }
            });
            let amount = crate::format::parse_amount(&form.amount).filter(|d| *d > 0);
            let too_much = amount.is_some_and(|d| crate::format::left_after(core, d, fee) < 0);
            if too_much {
                ui.label(txt(
                    tr!("More than there is on Core"),
                    12.0,
                    Weight::Regular,
                    p.danger,
                ));
            }
            let after = wallet.balances.credits.saturating_add(amount.unwrap_or(0));
            let value = mono(13.0, Weight::Medium);
            kv_row(
                ui,
                &p,
                tr!("From the Core wallet"),
                &dash(core),
                value.clone(),
            );
            kv_row(
                ui,
                &p,
                tr!("Core fee"),
                &format!("≈ {}", dash(fee)),
                value.clone(),
            );
            kv_row(
                ui,
                &p,
                tr!("On the identity after"),
                &format!(
                    tr!("{} · ≈ {} operations"),
                    dash(after),
                    group_thousands(after / fees.document_write.max(1))
                ),
                value,
            );
            notice(
                ui,
                a,
                "instantsend",
                p.accent,
                tr!(
                    "The asset lock is confirmed by InstantSend in seconds; if not, we wait for a ChainLock."
                ),
            );
            match (&running, &state.top_up_result) {
                (Some(_), _) => notice(
                    ui,
                    a,
                    "pending",
                    p.muted,
                    tr!(
                        "Topping up: locking DASH on Core, waiting for InstantSend, Platform adds the credits — usually under a minute. You can close the window."
                    ),
                ),
                (None, Some((duffs, result))) => top_up_answer(ui, a, *duffs, result),
                (None, None) => {}
            }
            let mut close = false;
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let label = match (running, amount) {
                    (Some(_), _) => tr!("Topping up…").to_string(),
                    (None, Some(d)) => format!(tr!("Top up by {}"), dash(d)),
                    (None, None) => tr!("Top up").to_string(),
                };
                let ready = running.is_none() && amount.is_some() && !too_much;
                if button_enabled(ui, a, &label, ButtonKind::Primary, None, ready).clicked() {
                    send = amount;
                }
                if button(ui, a, tr!("Close"), ButtonKind::Ghost, None).clicked() {
                    close = true;
                }
            });
            close
        },
    );
    if let Some(amount) = send {
        asked.push(Command::TopUpIdentity { amount });
        state.topping_up = Some(amount);
        state.top_up_result = None;
    } else if close {
        state.top_up = None;
    }
}

/// "Add a name": one more DPNS name for this identity, checked live like on "Create an
/// identity" and paid from its credits. A contested name cannot be taken here: masternodes
/// vote on it.
fn add_name_dialog(
    ui: &mut Ui,
    a: &mut Assets,
    state: &mut IdentityState,
    wallet: &Wallet,
    id: &Identity,
    asked: &mut Vec<Command>,
) {
    let Some(form) = &mut state.adding_name else {
        return;
    };
    let me = id
        .name()
        .map_or_else(|| "identity".to_string(), |n| format!("@{n}"));
    let fee = wallet.fees.name_regular;
    let credits = wallet.balances.credits;
    // The name goes to the core once per change; the answer comes back as an event.
    let label = form.name.trim().trim_start_matches('@').to_lowercase();
    if label.len() >= 3 && label != form.asked {
        asked.push(Command::CheckName {
            label: label.clone(),
        });
        form.asked = label.clone();
        form.check = None;
        form.result = None;
    }
    let check = if label.is_empty() || label != form.asked {
        None
    } else {
        form.check.clone()
    };
    let mut register = false;
    let close = dialog(
        ui,
        a,
        "add-name",
        "username",
        tr!("Add a name"),
        464.0,
        |ui, a| {
            let p = a.p;
            ui.add(
                egui::Label::new(
                    txt(
                        &format!(
                            tr!("One more DPNS name for {me}: people can find you and pay you by it too.")
                        , me = me),
                        13.0,
                        Weight::Regular,
                        p.muted,
                    )
                    .line_height(Some(20.0)),
                )
                .wrap(),
            );
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                field_label(ui, &p, tr!("Name"));
                text_field(
                    ui,
                    &p,
                    "add-name",
                    &mut form.name,
                    tr!("e.g. satoshi-2026"),
                    FieldKind::Text,
                    |_| {},
                );
                let (line, color) = match &check {
                    None if label.len() >= 3 => {
                        (Some(tr!("Checking the name in DPNS…").to_string()), p.muted)
                    }
                    None if !label.is_empty() => (
                        Some(tr!("This is not a Dash name: 3–63 letters, digits and hyphens").to_string()),
                        p.danger,
                    ),
                    None => (None, p.muted),
                    Some(Ok(NameCheck::Available)) => (
                        Some(format!(tr!("@{label} is free · a regular name, no vote"), label = label)),
                        p.text,
                    ),
                    Some(Ok(NameCheck::Contested)) => (
                        Some(format!(
                            tr!("@{label} is a contested name: masternodes award it by vote, which evolite cannot do yet. Add a digit from 2 to 9 or make the name longer than 19 characters.")
                        , label = label)),
                        p.danger,
                    ),
                    Some(Ok(NameCheck::Taken)) => (Some(format!(tr!("@{label} is already taken"), label = label)), p.danger),
                    Some(Err(e)) => {
                        let (why, typo) = text::name_problem(e);
                        (Some(why), if typo { p.danger } else { p.muted })
                    }
                };
                if let Some(line) = line {
                    ui.add(
                        egui::Label::new(txt(&line, 12.0, Weight::Regular, color)).wrap(),
                    );
                }
            });
            let value = mono(13.0, Weight::Medium);
            kv_row(
                ui,
                &p,
                tr!("Name registration"),
                &format!("≈ {}", dash(fee)),
                value.clone(),
            );
            kv_row(
                ui,
                &p,
                tr!("Paid by"),
                &format!(tr!("{me} · credits"), me = me),
                value,
            );
            let short = credits < fee;
            if short {
                ui.label(txt(
                    tr!("Not enough credits: top up the identity."),
                    12.0,
                    Weight::Regular,
                    p.danger,
                ));
            }
            match &form.result {
                _ if form.waiting => notice(
                    ui,
                    a,
                    "pending",
                    p.muted,
                    tr!(
                        "Writing the name to Platform: preorder and domain — usually under a minute."
                    ),
                ),
                Some(Ok(())) => notice(
                    ui,
                    a,
                    "confirmed",
                    p.accent,
                    &format!(tr!("The name @{} is written."), form.asked),
                ),
                Some(Err(e)) => crate::components::error_notice(
                    ui,
                    a,
                    &name_error(e),
                    crate::text::error_details(e).as_deref(),
                ),
                None => {}
            }
            let mut close = false;
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let ready = matches!(check, Some(Ok(NameCheck::Available)))
                    && !short
                    && !form.waiting
                    && form.result.is_none();
                let label = if form.waiting {
                    tr!("Writing…")
                } else {
                    tr!("Register")
                };
                if button_enabled(ui, a, label, ButtonKind::Primary, None, ready).clicked() {
                    register = true;
                }
                if button(ui, a, tr!("Close"), ButtonKind::Ghost, None).clicked() {
                    close = true;
                }
            });
            close
        },
    );
    if register {
        asked.push(Command::AddName {
            name: form.asked.clone(),
        });
        form.waiting = true;
        form.result = None;
    } else if close && !form.waiting {
        state.adding_name = None;
    }
}

/// Why the name was not registered, in words.
fn name_error(e: &CoreError) -> String {
    match e {
        // The core's reason is a catalog key: the catalog adds the language's own full stop.
        CoreError::NotAvailable(why) => format!("{}.", why.trim_end_matches('.')),
        CoreError::NotSynced => {
            tr!("Wait for the sync to finish: the masternode lists are needed.").into()
        }
        CoreError::NotEnough => tr!("Not enough credits: top up the identity.").into(),
        CoreError::BadName => {
            tr!("This is not a Dash name: 3–63 letters, digits and hyphens.").into()
        }
        other => crate::text::failed(tr!("The name is not written"), other),
    }
}

/// A tile of the credits card that was clicked.
#[derive(Clone, Copy, PartialEq)]
enum CreditsAction {
    TopUp,
    Withdraw,
    Transfer,
}

/// The credits card. Returns the tile clicked, if any.
fn credits(
    ui: &mut Ui,
    a: &mut Assets,
    balances: &Balances,
    fees: &Fees,
    width: f32,
) -> Option<CreditsAction> {
    let duffs = balances.credits;
    let p = a.p;
    let mut clicked = None;
    card(&p, Margin::same(24)).show(ui, |ui| {
        ui.set_width(inner(width, 24));
        ui.spacing_mut().item_spacing.y = 12.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.add(a.icons.icon("credits", p.muted, &p, 18.0));
            ui.label(txt(tr!("Platform credits"), 13.0, Weight::Medium, p.muted));
        });
        crate::components::amount_label(
            ui,
            &crate::format::amount(duffs),
            mono(28.0, Weight::Medium),
            p.text,
        );
        ui.label(
            egui::RichText::new(crate::i18n::t(&format!(
                tr!("{} credits\n≈ {} operations"),
                group_thousands(duffs * CREDITS_PER_DUFF),
                group_thousands(duffs / fees.document_write.max(1))
            )))
            .font(mono(12.0, Weight::Regular))
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
        sans(12.0, Weight::SemiBold),
        p.text,
    );
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

fn names(ui: &mut Ui, a: &mut Assets, id: &Identity, width: f32) {
    list_card(
        ui,
        a,
        width,
        tr!("DPNS names"),
        &id.names.len().to_string(),
        |ui, a| {
            let p = a.p;
            for n in &id.names {
                let (name, primary, voting) = (&n.label, n.primary, n.voting_days_left);
                divider(ui, &p);
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    let (icon, sub) = match voting {
                        Some(days) => (
                            "vote",
                            format!(tr!("masternode vote · {days} more days"), days = days),
                        ),
                        None if primary => ("username", tr!("main · verified").to_string()),
                        None => ("username", tr!("verified").to_string()),
                    };
                    ui.add(a.icons.icon(icon, p.text, &p, 20.0));
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(txt(&format!("@{name}"), 14.0, Weight::SemiBold, p.text));
                        ui.label(txt(&sub, 12.0, Weight::Regular, p.muted));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let (icon, color, hint) = if voting.is_some() {
                            ("pending", p.muted, tr!("Waiting for the masternode vote"))
                        } else {
                            (
                                "confirmed",
                                p.accent,
                                tr!("The name is confirmed by a Platform proof"),
                            )
                        };
                        ui.add(a.icons.icon(icon, color, &p, 18.0))
                            .on_hover_text(crate::i18n::t(hint).into_owned());
                    });
                });
                ui.add_space(12.0);
            }
        },
    );
}

fn keys(ui: &mut Ui, a: &mut Assets, id: &Identity, width: f32) {
    // The note is the count, as on "DPNS names": every key's details are already in its row, and
    // a "Details" there read as a link that led nowhere.
    let count = id.keys.len().to_string();
    list_card(ui, a, width, tr!("Keys"), &count, |ui, a| {
        let p = a.p;
        for key in &id.keys {
            let level = text::key_level(key.level);
            let purpose = text::key_role(key);
            divider(ui, &p);
            let (rect, _) =
                ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::hover());
            let cy = rect.center().y;
            let icon = Rect::from_min_size(pos2(rect.left(), cy - 9.0), vec2(18.0, 18.0));
            a.icons.icon("key", p.muted, &p, 18.0).paint_at(ui, icon);
            let painter = ui.painter();
            let mut x = icon.right() + 12.0;
            x = painter
                .text(
                    pos2(x, cy),
                    Align2::LEFT_CENTER,
                    crate::i18n::t(&format!("#{}", key.id)),
                    mono(13.0, Weight::Medium),
                    p.text,
                )
                .right()
                + 12.0;
            x = painter
                .text(
                    pos2(x, cy),
                    Align2::LEFT_CENTER,
                    crate::i18n::t(level),
                    mono(12.0, Weight::Medium),
                    p.muted,
                )
                .right()
                + 12.0;
            painter.text(
                pos2(x, cy),
                Align2::LEFT_CENTER,
                crate::i18n::t(&purpose),
                sans(13.0, Weight::Regular),
                p.text,
            );
            painter.text(
                pos2(rect.right(), cy),
                Align2::RIGHT_CENTER,
                crate::i18n::t(text::key_kind(key.kind)),
                mono(11.0, Weight::Regular),
                p.muted,
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{
        IdentityState, TOP_UP_SETTLED, searching_text, top_up_blocked_by, top_up_done_text,
        top_up_error, unfinished_top_up_text,
    };
    use evolite_api::CoreError;
    use evolite_api::IdentitySearch;
    use evolite_api::UnfinishedTopUp;

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
        let mut state = IdentityState {
            topping_up: Some(100_000),
            ..Default::default()
        };
        state.on_top_up_done(&Ok(()));
        assert_eq!(state.topping_up, None);
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
}
