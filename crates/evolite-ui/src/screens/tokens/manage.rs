//! The token page and its «Management» (FEAT-0019 T-3; the designer's M1, M1g, M2…M7): what the
//! shown identity may do with a token it manages, and one window per action. Only the actions
//! its rules allow are drawn (R-15); one it takes only with a group says «needs the group's
//! signatures» with no button (T-3.9); the ones that cannot be undone ask for «I understand»
//! (R-23).

use super::{TokensState, sender, shown_name, state};
use crate::components::para;
use crate::components::{
    Assets, ButtonKind, FieldKind, Part, SpendPassword, button, button_enabled, button_row,
    button_row_spend, checkbox, dialog_with_footer, divider, field_label, kv_row, notice,
    password_asked, segmented, text_field, txt,
};
use crate::context::Ctx;
use crate::theme::{Weight, mono, sans};
use egui::Ui;
use evolite_api::tokens::{
    IssuerAction, IssuerOp, NamesProblem, Right, SettingChange, SettingField, SettingView, Taker,
    TokenName, TokenPage, TokenRow, names_problem,
};
use evolite_api::{Command, CoreError, Spend};

/// The token page: which token, and «Check identity» typed for the frozen list.
pub struct PageState {
    pub token_id: String,
    pub check: String,
    asked: bool,
    /// The core's failed reads of the page already taken (BUG-222).
    failures_seen: u32,
    /// A read asked again and not answered yet: «Reading…», not the failure.
    reading: bool,
    /// When the page tries again by itself.
    retry_at: Option<std::time::Instant>,
    /// The failures counted when the page was opened: the tries again count from there.
    opened_at: u32,
}

/// After which failed read the page tries again by itself, and in how many seconds (BUG-222):
/// later each time, then only on «Try again».
const AUTO_RETRY_SECS: [u64; 3] = [5, 15, 45];

impl PageState {
    /// Opened, its page not asked of the core yet.
    pub fn fresh(&self) -> bool {
        !self.asked
    }

    pub fn new(token_id: &str) -> Self {
        PageState {
            token_id: token_id.to_owned(),
            check: String::new(),
            asked: false,
            failures_seen: 0,
            reading: false,
            retry_at: None,
            opened_at: 0,
        }
    }

    /// What the core says of the page's reads (BUG-222): `failed` is how many failed in a row,
    /// if the last did. A new failure ends «Reading…» and sets the next try, later each time.
    /// True when it is time to ask again.
    fn on_reads(&mut self, failed: Option<u32>, now: std::time::Instant) -> bool {
        if let Some(times) = failed
            && times > self.failures_seen
        {
            self.failures_seen = times;
            self.reading = false;
            self.retry_at = AUTO_RETRY_SECS
                .get(times.saturating_sub(self.opened_at).saturating_sub(1) as usize)
                .map(|s| now + std::time::Duration::from_secs(*s));
        }
        if self.retry_at.is_some_and(|at| now >= at) {
            self.try_again();
            return true;
        }
        false
    }

    /// Opened, its read asked: the failures the core counted before belong to an earlier
    /// visit (BUG-222 review r1, Should): only one after this counts, and until then it reads.
    fn opened(&mut self, failed: Option<u32>) {
        self.failures_seen = failed.unwrap_or(0);
        self.opened_at = self.failures_seen;
        self.reading = true;
        self.retry_at = None;
    }

    /// The page is asked again, now.
    fn try_again(&mut self) {
        self.retry_at = None;
        self.reading = true;
    }

    /// The last read failed and none is under way: the page says so.
    fn failed(&self) -> bool {
        self.failures_seen > 0 && !self.reading
    }
}

/// Which issuer window is open.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum IssuerWindow {
    Mint,
    Burn,
    Freeze,
    Unfreeze(String),
    Destroy(String),
    Pause,
    Resume,
    Price,
    Setting(SettingField, SettingEdit),
}

/// What part of a setting is changed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SettingEdit {
    Value,
    Makers,
    Admins,
}

/// One issuer window: what is typed, the tick, the password and the core's answer.
pub struct IssuerForm {
    pub token_id: String,
    pub window: IssuerWindow,
    pub amount: String,
    /// Mint: to another identity (else to oneself).
    pub to_other: bool,
    pub who: String,
    pub understood: bool,
    /// Price: 0 not sold, 1 one price, 2 tiers; the tiers as typed («from count», «price each»).
    pub price_kind: usize,
    pub tiers: Vec<(String, String)>,
    /// A setting's new value: a choice of who (0 nobody, 1 the owner, 2 an identity) or on/off.
    pub choice: usize,
    /// The names as typed, a language each; the first `names_from` are the token's own (their
    /// language fixed), the rest added here. Taken from the page once it is read.
    pub names: Vec<TokenName>,
    pub names_from: usize,
    pub names_read: bool,
    pub password: SpendPassword,
    pub waiting: bool,
    pub result: Option<Result<(), CoreError>>,
}

impl IssuerForm {
    pub fn new(token_id: &str, window: IssuerWindow) -> Self {
        IssuerForm {
            token_id: token_id.to_owned(),
            window,
            amount: String::new(),
            to_other: false,
            who: String::new(),
            understood: false,
            price_kind: 1,
            tiers: vec![(String::new(), String::new())],
            choice: 1,
            names: Vec::new(),
            names_from: 0,
            names_read: false,
            password: SpendPassword::default(),
            waiting: false,
            result: None,
        }
    }

    /// A who-window opens on the taker as it is now: «I understand» shows only once «nobody»
    /// is chosen (the designer's M7). A group is not offered here: the contract owner.
    pub fn choose_taker(&mut self, taker: &Taker) {
        match taker {
            Taker::NoOne => self.choice = 0,
            Taker::Identity(id) => {
                self.choice = 2;
                self.who = id.clone();
            }
            Taker::ContractOwner | Taker::MainGroup | Taker::Group(_) => self.choice = 1,
        }
    }

    /// `Done { what: IssuerAction }`: only an answer this window waits for.
    pub fn on_answer(&mut self, result: &Result<(), CoreError>) {
        if !self.waiting {
            return;
        }
        self.waiting = false;
        self.password.on_answer(result);
        self.result = Some(result.clone());
    }
}

/// «Contract owner: @x — that is you», «Managed by the group …» and the like.
fn taker_text(t: &Taker, page: &TokenPage) -> String {
    match t {
        Taker::NoOne => tr!("nobody").into(),
        Taker::ContractOwner if page.owner_is_me => tr!("the contract owner — you").into(),
        Taker::ContractOwner => tr!("the contract owner").into(),
        Taker::Identity(id) => crate::i18n::own(&state::short(id)),
        Taker::MainGroup => tr!("the main group").into(),
        Taker::Group(n) => format!(tr!("group {}"), n),
    }
}

fn action_label(a: IssuerAction) -> &'static str {
    match a {
        IssuerAction::Mint => tr!("Mint"),
        IssuerAction::Burn => tr!("Burn"),
        IssuerAction::Freeze => tr!("Freeze"),
        IssuerAction::Unfreeze => tr!("Unfreeze"),
        IssuerAction::DestroyFrozen => tr!("Destroy frozen funds"),
        IssuerAction::Pause => tr!("Pause"),
        IssuerAction::Price => tr!("Price"),
    }
}

/// A who-field's title, as M7 names it: «Who may mint», not «Who may: Mint».
fn who_title(a: IssuerAction) -> &'static str {
    match a {
        IssuerAction::Mint => tr!("Who may mint"),
        IssuerAction::Burn => tr!("Who may burn"),
        IssuerAction::Freeze => tr!("Who may freeze"),
        IssuerAction::Unfreeze => tr!("Who may unfreeze"),
        IssuerAction::DestroyFrozen => tr!("Who may destroy frozen funds"),
        IssuerAction::Pause => tr!("Who may pause"),
        IssuerAction::Price => tr!("Who may set the price"),
    }
}

fn field_label_of(f: SettingField) -> String {
    match f {
        SettingField::Names => tr!("Names").into(),
        SettingField::MaxSupply => tr!("Supply limit").into(),
        SettingField::NewTokensTo => tr!("Where new tokens go").into(),
        SettingField::ChooseDestination => tr!("The minter chooses where they go").into(),
        SettingField::TradeMode => tr!("Trading").into(),
        SettingField::Who(a) => who_title(a).into(),
    }
}

/// The window's button: «Sign and …» with the action's verb (the designer's table).
fn sign_label(window: &IssuerWindow) -> &'static str {
    match window {
        IssuerWindow::Mint => tr!("Sign and mint"),
        IssuerWindow::Burn => tr!("Sign and burn"),
        IssuerWindow::Freeze => tr!("Sign and freeze"),
        IssuerWindow::Unfreeze(_) => tr!("Sign and unfreeze"),
        IssuerWindow::Destroy(_) => tr!("Sign and destroy"),
        IssuerWindow::Pause => tr!("Sign and stop"),
        IssuerWindow::Resume => tr!("Sign and resume"),
        IssuerWindow::Price | IssuerWindow::Setting(..) => tr!("Sign and save"),
    }
}

/// What a settings change makes the setting, in the row's words.
fn change_text(change: &SettingChange, page: &TokenPage, decimals: u8) -> String {
    let on = |b: bool| if b { tr!("On") } else { tr!("Off") }.to_string();
    match change {
        SettingChange::MaxSupply(max) => {
            max.map_or_else(|| tr!("none").to_string(), |m| state::written(m, decimals))
        }
        SettingChange::NewTokensTo(to) => to.as_ref().map_or_else(
            || tr!("none").to_string(),
            |id| crate::i18n::own(&state::short(id)),
        ),
        SettingChange::ChooseDestination(b) | SettingChange::TradeMode(b) => on(*b),
        SettingChange::Names(names) => names_text(names),
        SettingChange::Makers(_, who) | SettingChange::Admins(_, who) => taker_text(who, page),
    }
}

/// The names as typed: trimmed, a row left empty dropped; `None` while Platform would refuse
/// them (said under the rows).
fn names_typed(form: &IssuerForm) -> Option<Vec<TokenName>> {
    let names: Vec<TokenName> = form
        .names
        .iter()
        .map(|n| TokenName {
            lang: n.lang.trim().to_owned(),
            one: n.one.trim().to_owned(),
            many: n.many.trim().to_owned(),
            capitalize: n.capitalize,
        })
        .filter(|n| !(n.lang.is_empty() && n.one.is_empty() && n.many.is_empty()))
        .collect();
    names_problem(&names).is_none().then_some(names)
}

/// The same names, in any order.
fn same_names(a: &[TokenName], b: &[TokenName]) -> bool {
    let sorted = |v: &[TokenName]| {
        let mut v = v.to_vec();
        v.sort_by(|x, y| x.lang.cmp(&y.lang));
        v
    };
    sorted(a) == sorted(b)
}

/// The names in a line: «en Coffee / Coffees, ru Кофе / Кофе».
fn names_text(names: &[TokenName]) -> String {
    names
        .iter()
        .map(|n| format!("{} {} / {}", n.lang, n.one, n.many))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Why the names typed cannot be written, as the line under them says it.
fn names_problem_text(problem: &NamesProblem) -> String {
    let own = |s: &str| crate::i18n::own(s);
    match problem {
        NamesProblem::NoEnglish => tr!("An English name is needed: Platform requires one.").into(),
        NamesProblem::Language(l) => {
            format!(tr!("“{}” is not a language code Platform takes."), own(l))
        }
        NamesProblem::Twice(l) => format!(tr!("The language {} is there twice."), own(l)),
        NamesProblem::Length(n) => format!(tr!("“{}” is not 3 to 25 bytes long."), own(n)),
        NamesProblem::Spaces(n) => format!(tr!("“{}” has a space: a name has none."), own(n)),
    }
}

/// What a setting is now, as its row says it (M7).
fn setting_value(s: &SettingView, page: &TokenPage, name: &str, decimals: u8) -> String {
    match s.field {
        SettingField::Names => crate::i18n::own(name),
        SettingField::MaxSupply => page
            .max_supply
            .map_or_else(|| tr!("none").to_string(), |m| state::written(m, decimals)),
        SettingField::NewTokensTo => page.new_tokens_to.as_ref().map_or_else(
            || tr!("none").to_string(),
            |id| crate::i18n::own(&state::short(id)),
        ),
        SettingField::ChooseDestination => if page.choose_destination {
            tr!("On")
        } else {
            tr!("Off")
        }
        .to_string(),
        SettingField::TradeMode => if page.tradeable {
            tr!("On")
        } else {
            tr!("Off")
        }
        .to_string(),
        SettingField::Who(_) => taker_text(&s.makers, page),
    }
}

/// A line under a setting's name: a muted text, or a muted text action when this identity may
/// change what it says. True when clicked.
fn setting_line(ui: &mut Ui, a: &mut Assets, text: &str, action: bool) -> bool {
    if action {
        crate::components::text_action(ui, a, "edit", text).clicked()
    } else {
        ui.label(txt(text, 12.0, Weight::Regular, a.p.muted));
        false
    }
}

/// The «Group» line of an action taken only with a group (M1g).
fn group_line(needed: u32, of: u32) -> String {
    format!(
        tr!("This needs signatures from the group: {} of {}"),
        needed, of
    )
}

/// The token page (M1), a screen entered from Tokens: «‹ Tokens» above its name, numbers,
/// «Management», frozen balances, settings. The issuer's windows (M2…M7) open over it.
pub fn page_screen(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, tokens: &mut TokensState) {
    let p = a.p;
    ui.spacing_mut().item_spacing.y = 16.0;
    let Some(pst) = &mut tokens.page else {
        cx.go(crate::screens::Screen::Tokens);
        return;
    };
    // BUG-222: a read that failed says so and is tried again, not «Reading…» for ever.
    let failed_reads = cx
        .wallet()
        .tokens
        .page_failed
        .as_ref()
        .filter(|f| f.token_id == pst.token_id)
        .map(|f| f.times);
    if !pst.asked {
        pst.asked = true;
        pst.opened(failed_reads);
        cx.send(Command::ReadTokenPage {
            token_id: pst.token_id.clone(),
            check: None,
        });
    }
    let now = std::time::Instant::now();
    if pst.on_reads(failed_reads, now) {
        cx.send(Command::ReadTokenPage {
            token_id: pst.token_id.clone(),
            check: None,
        });
    }
    let failed = pst.failed();
    let retry_in = pst
        .retry_at
        .map(|at| at.saturating_duration_since(now).as_secs() + 1);
    let mut try_again = false;
    let Some(row): Option<TokenRow> = cx
        .wallet()
        .tokens
        .rows
        .iter()
        .find(|r| r.info.token_id == pst.token_id)
        .cloned()
    else {
        tokens.page = None;
        cx.go(crate::screens::Screen::Tokens);
        return;
    };
    let page = cx
        .wallet()
        .tokens
        .page
        .clone()
        .filter(|pg| pg.token_id == pst.token_id);
    let decimals = row.info.decimals;
    let mut open = None;
    let mut check = None;
    let title = state::name_in_text(&row.info);
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        if crate::components::back_link(ui, a, crate::screens::place_name(cx.back)).clicked() {
            cx.go_back();
        }
        crate::components::page_header(ui, a, &title, None, |_, _| {});
    });
    ui.scope(|ui| {
        'body: {
            ui.spacing_mut().item_spacing.y = 10.0;
            let Some(page) = &page else {
                if !failed {
                    ui.label(txt(
                        tr!("Reading the token's rules…"),
                        13.0,
                        Weight::Regular,
                        p.muted,
                    ));
                    break 'body;
                }
                ui.label(txt(
                    tr!("The token's rules could not be read from the network."),
                    13.0,
                    Weight::Regular,
                    p.danger,
                ));
                if let Some(secs) = retry_in {
                    ui.label(txt(
                        &format!(tr!("Trying again in {} s"), secs),
                        12.0,
                        Weight::Regular,
                        p.muted,
                    ));
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(500));
                }
                try_again =
                    button(ui, a, tr!("Try again"), ButtonKind::Secondary, None).clicked();
                break 'body;
            };
            let value = mono(13.0, Weight::Medium);
            let owner = if page.owner_is_me {
                format!(
                    tr!(
                        "Contract owner: {} — that is you. Only what the token’s rules allow you is here."
                    ),
                    crate::i18n::own(&shown_name(cx.wallet()))
                )
            } else {
                format!(
                    tr!("Contract owner: {}"),
                    crate::i18n::own(&state::short(&page.owner))
                )
            };
            ui.add(para(&owner, 13.0, Weight::Regular, p.muted));
            kv_row(
                ui,
                &p,
                tr!("In circulation"),
                &state::written(page.supply, decimals),
                value.clone(),
            );
            kv_row(
                ui,
                &p,
                tr!("Limit"),
                &page
                    .max_supply
                    .map_or_else(|| tr!("none").to_string(), |m| state::written(m, decimals)),
                value.clone(),
            );
            kv_row(
                ui,
                &p,
                tr!("You have"),
                &row.balance
                    .map_or("—".into(), |b| state::held_amount(b, decimals)),
                value.clone(),
            );
            kv_row(
                ui,
                &p,
                tr!("State"),
                if page.paused {
                    tr!("paused")
                } else {
                    tr!("transfers run")
                },
                sans(13.0, Weight::Medium),
            );
            if !page.manages() {
                break 'body;
            }
            divider(ui, &p);
            ui.label(txt(tr!("Management"), 15.0, Weight::SemiBold, p.text));
            // The actions taken alone: one row of buttons (M1).
            ui.horizontal_wrapped(|ui| {
                for (action, right) in &page.actions {
                    match right {
                        Right::Alone => {
                            let mut buttons = vec![];
                            match action {
                                IssuerAction::Pause if page.paused => {
                                    buttons.push((tr!("Resume"), IssuerWindow::Resume))
                                }
                                IssuerAction::Pause => {
                                    buttons.push((tr!("Pause"), IssuerWindow::Pause))
                                }
                                IssuerAction::Mint if mint_goes_somewhere(page) => {
                                    buttons.push((tr!("Mint"), IssuerWindow::Mint))
                                }
                                IssuerAction::Mint => {}
                                IssuerAction::Burn => {
                                    buttons.push((tr!("Burn"), IssuerWindow::Burn))
                                }
                                IssuerAction::Freeze => {
                                    buttons.push((tr!("Freeze"), IssuerWindow::Freeze))
                                }
                                IssuerAction::Price => {
                                    buttons.push((tr!("Price"), IssuerWindow::Price))
                                }
                                // Unfreeze and destroy: in the frozen balances' rows.
                                IssuerAction::Unfreeze | IssuerAction::DestroyFrozen => {}
                            }
                            for (label, window) in buttons {
                                if button(ui, a, label, ButtonKind::Secondary, None).clicked() {
                                    open = Some(window);
                                }
                            }
                        }
                        Right::Group { .. } | Right::No => {}
                    }
                }
            });
            if page.right(IssuerAction::Mint) == Right::Alone && !mint_goes_somewhere(page) {
                ui.add(para(
                    tr!("Minting is impossible: the token’s rule does not say whom new tokens go to."),
                    12.0,
                    Weight::Regular,
                    p.muted,
                ));
            }
            // The ones only a group takes (M1g): said, no button.
            for (action, right) in &page.actions {
                if let Right::Group { needed, of } = right {
                    // One sentence, its action a place the template translates: put in reading
                    // order once (translated first and joined, the action's words came out
                    // reversed in Arabic, and a joined template was not translated at all).
                    ui.add(para(
                        &format!(
                            tr!("{} · This needs signatures from the group: {} of {}"),
                            action_label(*action),
                            needed,
                            of
                        ),
                        12.0,
                        Weight::Regular,
                        p.muted,
                    ));
                }
            }
            if page
                .actions
                .iter()
                .any(|(_, r)| matches!(r, Right::Group { .. }))
            {
                ui.add(para(
                tr!("evolite cannot sign group actions yet. Here you see who manages the token and how many signatures are needed."),
                12.0,
                Weight::Regular,
                p.muted,
            ));
            }
            // Frozen balances (Q2): the known ones, proved.
            let unfreeze = page.right(IssuerAction::Unfreeze) == Right::Alone;
            let destroy = page.right(IssuerAction::DestroyFrozen) == Right::Alone;
            if page.right(IssuerAction::Freeze) != Right::No || unfreeze || destroy {
                divider(ui, &p);
                ui.label(txt(tr!("Frozen balances"), 13.0, Weight::SemiBold, p.text));
                for who in &page.frozen {
                    egui::Frame::new()
                        .stroke(egui::Stroke::new(1.0, p.line))
                        .corner_radius(egui::CornerRadius::same(12))
                        .inner_margin(egui::Margin::symmetric(14, 8))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.add(a.icons.icon("lock", p.muted, &p, 16.0));
                                ui.label(txt(
                                    &crate::i18n::own(&state::short(who)),
                                    13.0,
                                    Weight::Medium,
                                    p.text,
                                ));
                                // The buttons at the end of the row, in reading order.
                                let gap = ui.spacing().item_spacing.x;
                                let mut w = 0.0;
                                if unfreeze {
                                    w += crate::components::button_width(ui, tr!("Unfreeze")) + gap;
                                }
                                if destroy {
                                    w += crate::components::button_width(ui, tr!("Destroy")) + gap;
                                }
                                ui.add_space((ui.available_width() - w).max(0.0));
                                if unfreeze
                                    && button(ui, a, tr!("Unfreeze"), ButtonKind::Secondary, None)
                                        .clicked()
                                {
                                    open = Some(IssuerWindow::Unfreeze(who.clone()));
                                }
                                if destroy
                                    && button(ui, a, tr!("Destroy"), ButtonKind::DangerOutline, None)
                                        .clicked()
                                {
                                    open = Some(IssuerWindow::Destroy(who.clone()));
                                }
                            });
                        });
                }
                ui.add(para(
                    tr!("Shown are those this wallet knows of; check others by “Check”."),
                    12.0,
                    Weight::Regular,
                    p.muted,
                ));
                // The field and «Check» on one line (the designer's nit).
                let check_w = crate::components::button_width(ui, tr!("Check"));
                ui.horizontal(|ui| {
                    let gap = ui.spacing().item_spacing.x;
                    ui.scope(|ui| {
                        ui.set_max_width((ui.available_width() - check_w - gap).max(120.0));
                        // Its name for the reader: no label above it.
                        crate::a11y::name_next_field(ui, tr!("Check an identity"));
                        text_field(
                            ui,
                            &p,
                            "token-check",
                            &mut pst.check,
                            "@name",
                            FieldKind::Text,
                            |_| {},
                        );
                    });
                    if button(ui, a, tr!("Check"), ButtonKind::Secondary, None).clicked()
                        && !pst.check.trim().is_empty()
                    {
                        check = Some(pst.check.trim().to_owned());
                    }
                });
            }
            // Settings (M7): one framed list, a row each, the locked ones with a lock.
            divider(ui, &p);
            ui.label(txt(tr!("Settings"), 13.0, Weight::SemiBold, p.text));
            egui::Frame::new()
                .stroke(egui::Stroke::new(1.0, p.line))
                .corner_radius(egui::CornerRadius::same(12))
                .inner_margin(egui::Margin::symmetric(14, 10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    for (i, s) in page.settings.iter().enumerate() {
                        if i > 0 {
                            divider(ui, &p);
                        }
                        setting_row(
                            ui,
                            a,
                            page,
                            s,
                            (state::name(&row.info), decimals),
                            &mut open,
                        );
                    }
                });
        }
    });
    if try_again {
        pst.try_again();
        cx.send(Command::ReadTokenPage {
            token_id: pst.token_id.clone(),
            check: None,
        });
    }
    if let Some(check) = check {
        cx.send(Command::ReadTokenPage {
            token_id: pst.token_id.clone(),
            check: Some(check),
        });
    }
    if let Some(window) = open {
        let mut form = IssuerForm::new(&pst.token_id, window);
        if let (IssuerWindow::Setting(field, edit), Some(pg)) = (&form.window, &page)
            && let Some(s) = pg.settings.iter().find(|s| s.field == *field)
        {
            match edit {
                SettingEdit::Makers => form.choose_taker(&s.makers),
                SettingEdit::Admins => form.choose_taker(&s.admins),
                SettingEdit::Value => {}
            }
        }
        tokens.issuer = Some(form);
    }
    if tokens.issuer.is_some() {
        issuer_dialog(ui, a, cx, tokens);
    }
}

/// One setting (M7): its name and who may change it on the left, what it is now on the right —
/// a button that opens its window where this identity may change it, a lock where nobody ever
/// can. Who may change it, and who decides that (the architect's Q4), are lines under the name,
/// text actions where this identity may change them.
fn setting_row(
    ui: &mut Ui,
    a: &mut Assets,
    page: &TokenPage,
    s: &SettingView,
    (name, decimals): (&str, u8),
    open: &mut Option<IssuerWindow>,
) {
    let p = a.p;
    let who_field = matches!(s.field, SettingField::Who(_));
    let value = setting_value(s, page, name, decimals);
    // The value opens its window: a Who field's value is who takes the action — its makers.
    let value_window = if who_field {
        (s.change_who == Right::Alone).then_some(SettingEdit::Makers)
    } else {
        (s.change == Right::Alone).then_some(SettingEdit::Value)
    };
    let label = if s.never || value.is_empty() {
        String::new()
    } else {
        value.clone()
    };
    // The right side's width, so the left one leaves room for it: the value, or «Change».
    let right_w = if !label.is_empty() {
        crate::components::button_width(ui, &label)
    } else if value_window.is_some() && !s.never {
        crate::components::button_width(ui, tr!("Change"))
    } else {
        0.0
    };
    ui.horizontal_top(|ui| {
        let gap = ui.spacing().item_spacing.x;
        let left = (ui.available_width() - right_w - gap).max(120.0);
        ui.vertical(|ui| {
            ui.set_width(left);
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.label(txt(
                &field_label_of(s.field),
                13.0,
                Weight::SemiBold,
                p.text,
            ));
            if s.never {
                ui.label(txt(
                    tr!("Can never change: “nobody” was chosen at creation"),
                    12.0,
                    Weight::Regular,
                    p.muted,
                ));
                if !value.is_empty() {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        ui.add(a.icons.icon("lock", p.muted, &p, 14.0));
                        ui.label(txt(&value, 12.0, Weight::Medium, p.muted));
                    });
                }
                return;
            }
            let may_who = s.change_who == Right::Alone;
            if !who_field {
                let who = format!(tr!("Who can change this: {}"), taker_text(&s.makers, page));
                if setting_line(ui, a, &who, may_who) {
                    *open = Some(IssuerWindow::Setting(s.field, SettingEdit::Makers));
                }
            }
            let admins = format!(tr!("Who decides who can: {}"), taker_text(&s.admins, page));
            if setting_line(ui, a, &admins, may_who) {
                *open = Some(IssuerWindow::Setting(s.field, SettingEdit::Admins));
            }
            if let Right::Group { needed, of } = s.change {
                ui.label(txt(&group_line(needed, of), 12.0, Weight::Regular, p.muted));
            }
        });
        if !label.is_empty() {
            match value_window {
                Some(edit) => {
                    if button(ui, a, &label, ButtonKind::Secondary, None).clicked() {
                        *open = Some(IssuerWindow::Setting(s.field, edit));
                    }
                }
                None => {
                    ui.label(txt(&label, 13.0, Weight::Medium, p.text));
                }
            }
        } else if let Some(edit) = value_window.filter(|_| !s.never)
            && button(ui, a, tr!("Change"), ButtonKind::Secondary, None).clicked()
        {
            *open = Some(IssuerWindow::Setting(s.field, edit));
        }
    });
}

/// Whether this identity's mint has somewhere to go (the architect's Must 2): Platform mints to
/// the identity named in the mint, else to the rule's `NewTokensDestinationIdentity`, never to
/// the signer by itself. With no choice allowed and no destination there is no mint at all.
pub(super) fn mint_goes_somewhere(page: &TokenPage) -> bool {
    page.choose_destination || page.new_tokens_to.is_some()
}

/// A price per whole token, in duffs, as credits per smallest unit (1 duff = 1000 credits):
/// only one that divides exactly, so the price kept is the price typed (Should 2).
pub(super) fn per_unit_of(duffs: i64, decimals: u8) -> Option<u64> {
    let credits = u64::try_from(duffs).ok()?.checked_mul(1_000)?;
    let unit = 10u64.checked_pow(u32::from(decimals))?;
    (credits % unit == 0 && credits >= unit).then(|| credits / unit)
}

/// The price per whole token, in duffs, of `per_unit` credits per smallest unit.
pub(super) fn price_of(per_unit: u64, decimals: u8) -> i64 {
    let whole = u128::from(per_unit) * 10u128.pow(u32::from(decimals)) / 1_000;
    i64::try_from(whole).unwrap_or(i64::MAX)
}

/// The smallest step of a price per whole token, in duffs, with `decimals`.
pub(super) fn price_step(decimals: u8) -> i64 {
    price_of(1, decimals).max(1)
}

/// What the window asks for, built from what was typed; `None`: not ready. `me`: the shown
/// identity, where «Yourself» mints to.
fn op_of(form: &IssuerForm, decimals: u8, page: &TokenPage, me: Option<&str>) -> Option<IssuerOp> {
    op_typed(form, decimals, page, me).filter(|op| !unchanged(page, op))
}

/// A settings change to what is set already (the architect's r2): Platform would take the fee
/// for nothing, so there is no operation, and the window says «It is set this way already».
fn unchanged(page: &TokenPage, op: &IssuerOp) -> bool {
    let IssuerOp::Setting(change) = op else {
        return false;
    };
    let view = |f: SettingField| page.settings.iter().find(|s| s.field == f);
    match change {
        SettingChange::MaxSupply(max) => *max == page.max_supply,
        SettingChange::NewTokensTo(to) => *to == page.new_tokens_to,
        SettingChange::ChooseDestination(on) => *on == page.choose_destination,
        SettingChange::TradeMode(on) => *on == page.tradeable,
        SettingChange::Makers(f, who) => view(*f).is_some_and(|s| s.makers == *who),
        SettingChange::Admins(f, who) => view(*f).is_some_and(|s| s.admins == *who),
        SettingChange::Names(names) => same_names(names, &page.names),
    }
}

/// What was typed, whatever is set now.
fn op_typed(
    form: &IssuerForm,
    decimals: u8,
    page: &TokenPage,
    me: Option<&str>,
) -> Option<IssuerOp> {
    let amount = || state::typed(&form.amount, decimals).ok().filter(|a| *a > 0);
    let who = || Some(form.who.trim().to_owned()).filter(|w| !w.is_empty());
    Some(match &form.window {
        IssuerWindow::Mint => {
            let amount = amount()?;
            // Must 1: over the supply limit there is no mint; the line under it says why.
            if page
                .max_supply
                .is_some_and(|max| page.supply.saturating_add(amount) > max)
            {
                return None;
            }
            // Must 2: «Yourself» names this identity; with no choice, the rule's destination.
            let to = if page.choose_destination {
                Some(if form.to_other {
                    who()?
                } else {
                    me?.to_owned()
                })
            } else if page.new_tokens_to.is_some() {
                None
            } else {
                return None;
            };
            IssuerOp::Mint { to, amount }
        }
        IssuerWindow::Burn => IssuerOp::Burn { amount: amount()? },
        IssuerWindow::Freeze => IssuerOp::Freeze { who: who()? },
        IssuerWindow::Unfreeze(w) => IssuerOp::Unfreeze { who: w.clone() },
        IssuerWindow::Destroy(w) => IssuerOp::DestroyFrozen { who: w.clone() },
        IssuerWindow::Pause => IssuerOp::Pause,
        IssuerWindow::Resume => IssuerOp::Resume,
        IssuerWindow::Price => IssuerOp::Price(match form.price_kind {
            0 => None,
            _ => {
                let mut tiers = Vec::new();
                for (from, price) in &form.tiers {
                    let from = if form.price_kind == 1 {
                        1
                    } else {
                        state::typed(from, decimals).ok()?
                    };
                    let price = crate::format::parse_amount(price).filter(|d| *d > 0)?;
                    // A price per whole token, kept per smallest unit in credits: exactly.
                    tiers.push((from, per_unit_of(price, decimals)?));
                    if form.price_kind == 1 {
                        break;
                    }
                }
                tiers.sort();
                Some(tiers)
            }
        }),
        IssuerWindow::Setting(field, edit) => IssuerOp::Setting(match (field, edit) {
            (SettingField::MaxSupply, SettingEdit::Value) => {
                SettingChange::MaxSupply(if form.amount.trim().is_empty() {
                    None
                } else {
                    let max = amount()?;
                    if max < page.supply {
                        return None;
                    }
                    Some(max)
                })
            }
            (SettingField::NewTokensTo, SettingEdit::Value) => SettingChange::NewTokensTo(who()),
            (SettingField::ChooseDestination, SettingEdit::Value) => {
                SettingChange::ChooseDestination(form.choice == 1)
            }
            (SettingField::TradeMode, SettingEdit::Value) => {
                SettingChange::TradeMode(form.choice == 1)
            }
            (SettingField::Names, SettingEdit::Value) => SettingChange::Names(names_typed(form)?),
            (field, SettingEdit::Makers | SettingEdit::Admins) => {
                let taker = match form.choice {
                    0 => Taker::NoOne,
                    1 => Taker::ContractOwner,
                    _ => Taker::Identity(who()?),
                };
                if *edit == SettingEdit::Makers {
                    SettingChange::Makers(*field, taker)
                } else {
                    SettingChange::Admins(*field, taker)
                }
            }
            _ => return None,
        }),
    })
}

/// The window's title.
fn window_title(w: &IssuerWindow, token: &str) -> String {
    let token = state::in_text(token);
    match w {
        IssuerWindow::Mint => format!(tr!("Mint {}"), token),
        IssuerWindow::Burn => format!(tr!("Burn {}"), token),
        IssuerWindow::Freeze => format!(tr!("Freeze a balance of {}"), token),
        IssuerWindow::Unfreeze(_) => format!(tr!("Unfreeze a balance of {}"), token),
        IssuerWindow::Destroy(_) => tr!("Destroy frozen funds").into(),
        IssuerWindow::Pause => format!(tr!("Pause {}"), token),
        IssuerWindow::Resume => format!(tr!("Resume {}"), token),
        IssuerWindow::Price => format!(tr!("Price of {}"), token),
        IssuerWindow::Setting(..) => format!(tr!("Settings of {}"), token),
    }
}

/// The names window's rows: a language each — its code (typed for one added here), the forms for
/// one and for many, «Remove» but for English — then «Add a language».
fn names_rows(ui: &mut Ui, a: &mut Assets, form: &mut IssuerForm) {
    let p = a.p;
    let fixed = form.names_from;
    let mut remove = None;
    let gap = ui.spacing().item_spacing.x;
    let remove_w = crate::components::button_width(ui, tr!("Remove"));
    for (i, n) in form.names.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            let lang_w = 70.0;
            let form_w = ((ui.available_width() - lang_w - remove_w - 3.0 * gap) / 2.0).max(80.0);
            ui.scope(|ui| {
                // As wide in every row, so the names stand in columns.
                ui.set_min_width(lang_w);
                ui.set_max_width(lang_w);
                if i < fixed {
                    ui.label(txt(&n.lang, 13.0, Weight::Medium, p.text));
                } else {
                    crate::a11y::name_next_field(ui, tr!("Language"));
                    text_field(
                        ui,
                        &p,
                        &format!("token-name-lang-{i}"),
                        &mut n.lang,
                        "de",
                        FieldKind::Text,
                        |_| {},
                    );
                }
            });
            ui.scope(|ui| {
                ui.set_max_width(form_w);
                crate::a11y::name_next_field(ui, tr!("One"));
                text_field(
                    ui,
                    &p,
                    &format!("token-name-one-{i}"),
                    &mut n.one,
                    tr!("One"),
                    FieldKind::Text,
                    |_| {},
                );
            });
            ui.scope(|ui| {
                ui.set_max_width(form_w);
                crate::a11y::name_next_field(ui, tr!("Many"));
                text_field(
                    ui,
                    &p,
                    &format!("token-name-many-{i}"),
                    &mut n.many,
                    tr!("Many"),
                    FieldKind::Text,
                    |_| {},
                );
            });
            if n.lang != "en" && button(ui, a, tr!("Remove"), ButtonKind::Ghost, None).clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        form.names.remove(i);
        if i < form.names_from {
            form.names_from -= 1;
        }
    }
    if button(ui, a, tr!("Add a language"), ButtonKind::Ghost, None).clicked() {
        form.names.push(TokenName {
            lang: String::new(),
            one: String::new(),
            many: String::new(),
            capitalize: true,
        });
    }
}

/// M2…M7: one window per issuer action.
pub(super) fn issuer_dialog(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, tokens: &mut TokensState) {
    let p = a.p;
    let Some(form) = &mut tokens.issuer else {
        return;
    };
    let Some(page) = cx
        .wallet()
        .tokens
        .page
        .clone()
        .filter(|pg| pg.token_id == form.token_id)
    else {
        // Until the page is read (a preset opens the window with it), nothing to draw yet.
        return;
    };
    let Some(row) = cx
        .wallet()
        .tokens
        .rows
        .iter()
        .find(|r| r.info.token_id == form.token_id)
        .cloned()
    else {
        tokens.issuer = None;
        return;
    };
    let wallet = cx.wallet();
    let who = sender(wallet);
    let from = shown_name(wallet);
    let decimals = row.info.decimals;
    // A burn of more than this identity has is refused before the password.
    let over = form.window == IssuerWindow::Burn
        && state::typed(&form.amount, decimals).is_ok_and(|n| n > row.balance.unwrap_or(0));
    // The names window starts from the token's names, once its page is read.
    if form.window == IssuerWindow::Setting(SettingField::Names, SettingEdit::Value)
        && !form.names_read
    {
        form.names = page.names.clone();
        form.names_from = form.names.len();
        form.names_read = true;
    }
    let me = wallet.identity.as_ref().map(|i| i.id.clone());
    let op = op_of(form, decimals, &page, me.as_deref()).filter(|_| !over);
    let same =
        op_typed(form, decimals, &page, me.as_deref()).is_some_and(|op| unchanged(&page, &op));
    // Should 2: a typed price that the smallest unit cannot carry exactly, and the price kept.
    let inexact = form.window == IssuerWindow::Price
        && form.price_kind > 0
        && form.tiers.iter().any(|(_, price)| {
            crate::format::parse_amount(price)
                .is_some_and(|d| d > 0 && per_unit_of(d, decimals).is_none())
        });
    let kept: Vec<(u64, u64)> = match &op {
        Some(IssuerOp::Price(Some(tiers))) => tiers.clone(),
        _ => vec![],
    };
    let tick = op.as_ref().is_some_and(|o| o.needs_tick())
        || matches!(
            form.window,
            IssuerWindow::Burn | IssuerWindow::Destroy(_) | IssuerWindow::Pause
        );
    // Q3: two document writes; a settings change four.
    let writes = if matches!(form.window, IssuerWindow::Setting(..)) {
        4
    } else {
        2
    };
    let fee = wallet.fees.document_write.saturating_mul(writes).max(0) as u64;
    let no_credits = who.credits.is_some_and(|c| c < fee);
    let ask = password_asked(cx.snap);
    let mut send = None;
    let title = window_title(&form.window, state::name(&row.info));
    let danger = matches!(form.window, IssuerWindow::Destroy(_));
    let closed = dialog_with_footer(
        ui,
        a,
        "token-issuer",
        "token",
        &title,
        460.0,
        |ui, a, part| {
            if part == Part::Footer {
                let mut close = false;
                if tick {
                    let words = match &form.window {
                        IssuerWindow::Burn => {
                            tr!("I understand: burned tokens disappear for good").to_string()
                        }
                        IssuerWindow::Destroy(w) => format!(
                            tr!("I understand: the funds of {} disappear for good"),
                            crate::i18n::own(&state::short(w))
                        ),
                        IssuerWindow::Pause => tr!(
                            "I understand: nobody can transfer the token until the pause is lifted"
                        )
                        .to_string(),
                        _ => tr!("I understand: this cannot be changed back").to_string(),
                    };
                    checkbox(ui, a, &mut form.understood, &words);
                }
                form.password.field(ui, &p, "token-issuer-password", ask);
                match form.result.as_ref().filter(|_| !form.password.is_wrong()) {
                    Some(Ok(())) => {
                        notice(ui, a, "confirmed", p.accent, tr!("Done: it is in History."));
                        button_row(ui, |ui| {
                            if button(ui, a, tr!("Done"), ButtonKind::Primary, None).clicked() {
                                close = true;
                            }
                        });
                        return close;
                    }
                    Some(Err(e)) => crate::components::error_notice(
                        ui,
                        a,
                        &crate::text::failed(tr!("Not done"), e),
                        crate::text::error_details(e).as_deref(),
                    ),
                    None => {}
                }
                button_row_spend(ui, |ui| {
                    if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                        close = true;
                    }
                    let ready = !form.waiting
                        && op.is_some()
                        && (!tick || form.understood)
                        && !no_credits
                        && form.password.ready(ask);
                    let kind = if danger {
                        ButtonKind::Danger
                    } else {
                        ButtonKind::Primary
                    };
                    if button_enabled(ui, a, sign_label(&form.window), kind, None, ready).clicked()
                        && let Some(op) = op.clone()
                    {
                        send = Some(Spend::IssuerAction {
                            token_id: form.token_id.clone(),
                            op,
                        });
                    }
                });
                return close;
            }
            ui.spacing_mut().item_spacing.y = 10.0;
            let value = mono(13.0, Weight::Medium);
            match &form.window {
                IssuerWindow::Mint | IssuerWindow::Burn => {
                    field_label(
                        ui,
                        &p,
                        if form.window == IssuerWindow::Mint {
                            tr!("How much to mint")
                        } else {
                            tr!("How much to burn")
                        },
                    );
                    text_field(
                        ui,
                        &p,
                        "token-issuer-amount",
                        &mut form.amount,
                        "0",
                        FieldKind::Amount,
                        |ui| {
                            ui.label(txt(
                                &state::name_in_text(&row.info),
                                13.0,
                                Weight::Regular,
                                p.muted,
                            ));
                        },
                    );
                    let n = state::typed(&form.amount, decimals).ok().unwrap_or(0);
                    if form.window == IssuerWindow::Mint {
                        if page.choose_destination {
                            let mut to = usize::from(form.to_other);
                            segmented(
                                ui,
                                &p,
                                &[tr!("Yourself"), tr!("Another identity")],
                                &mut to,
                                true,
                            );
                            form.to_other = to == 1;
                            if form.to_other {
                                text_field(
                                    ui,
                                    &p,
                                    "token-issuer-who",
                                    &mut form.who,
                                    "@name",
                                    FieldKind::Text,
                                    |_| {},
                                );
                            }
                        } else if let Some(to) = &page.new_tokens_to {
                            kv_row(
                                ui,
                                &p,
                                tr!("To"),
                                &crate::i18n::own(&state::short(to)),
                                value.clone(),
                            );
                        }
                        if let Some(max) = page.max_supply {
                            let left = max.saturating_sub(page.supply);
                            ui.label(txt(
                                &format!(
                                    tr!("The supply limit allows {} more."),
                                    state::written(left, decimals)
                                ),
                                12.0,
                                Weight::Regular,
                                if n > left { p.danger } else { p.muted },
                            ));
                        }
                        kv_row(
                            ui,
                            &p,
                            tr!("In circulation after"),
                            &state::written(page.supply.saturating_add(n), decimals),
                            value.clone(),
                        );
                    } else {
                        let have = row.balance.unwrap_or(0);
                        kv_row(
                            ui,
                            &p,
                            tr!("You will have left"),
                            &state::written(have.saturating_sub(n), decimals),
                            value.clone(),
                        );
                        if n > have {
                            ui.label(txt(
                                &format!(
                                    tr!("More than you have: {} available."),
                                    state::held_amount(have, decimals)
                                ),
                                12.0,
                                Weight::Regular,
                                p.danger,
                            ));
                        }
                        ui.label(txt(
                            tr!("Burned tokens are gone for good."),
                            12.0,
                            Weight::Regular,
                            p.muted,
                        ));
                    }
                }
                IssuerWindow::Freeze => {
                    field_label(ui, &p, tr!("Whose balance"));
                    text_field(
                        ui,
                        &p,
                        "token-issuer-who",
                        &mut form.who,
                        "@name",
                        FieldKind::Text,
                        |_| {},
                    );
                }
                IssuerWindow::Unfreeze(w) | IssuerWindow::Destroy(w) => {
                    kv_row(
                        ui,
                        &p,
                        tr!("Identity"),
                        &crate::i18n::own(&state::short(w)),
                        value.clone(),
                    );
                    if danger {
                        crate::components::danger_warning(
                            ui,
                            a,
                            tr!(
                                "The funds disappear from this identity for good, and nobody can bring them back — neither it nor you."
                            ),
                        );
                    }
                }
                IssuerWindow::Pause => {
                    ui.add(para(
                    &format!(
                        tr!("All transfers of {} stop — for every holder and for you too. Buying too. You can lift the pause here."),
                        state::name_in_text(&row.info)
                    ),
                    13.0,
                    Weight::Regular,
                    p.text,
                ));
                }
                IssuerWindow::Resume => {}
                IssuerWindow::Price => {
                    segmented(
                        ui,
                        &p,
                        &[tr!("Do not sell"), tr!("One price"), tr!("In tiers")],
                        &mut form.price_kind,
                        true,
                    );
                    if form.price_kind > 0 {
                        let tiers = form.price_kind == 2;
                        for (i, (from, price)) in form.tiers.iter_mut().enumerate() {
                            ui.horizontal(|ui| {
                                if tiers {
                                    crate::a11y::name_next_field(ui, tr!("from"));
                                    text_field(
                                        ui,
                                        &p,
                                        &format!("tier-from-{i}"),
                                        from,
                                        tr!("from"),
                                        FieldKind::Amount,
                                        |_| {},
                                    );
                                }
                                crate::a11y::name_next_field(ui, tr!("DASH each"));
                                text_field(
                                    ui,
                                    &p,
                                    &format!("tier-price-{i}"),
                                    price,
                                    tr!("DASH each"),
                                    FieldKind::Amount,
                                    |_| {},
                                );
                            });
                            if !tiers {
                                break;
                            }
                        }
                        if tiers
                            && button(ui, a, tr!("Add a tier"), ButtonKind::Ghost, None).clicked()
                        {
                            form.tiers.push((String::new(), String::new()));
                        }
                        if inexact {
                            ui.add(para(
                                &format!(
                                    tr!("With {} decimals the price can be set in steps of {} per token."),
                                    decimals,
                                    crate::format::amount(price_step(decimals))
                                ),
                                12.0,
                                Weight::Regular,
                                p.danger,
                            ));
                        }
                        // «Check»: the price as it will be kept, worked back from the credits.
                        for (from, per_unit) in &kept {
                            let each = format!(
                                tr!("{} per token"),
                                crate::format::amount(price_of(*per_unit, decimals))
                            );
                            let label = if form.price_kind == 1 {
                                tr!("Price").to_string()
                            } else {
                                format!(tr!("From {}"), state::written(*from, decimals))
                            };
                            kv_row(ui, &p, &label, &each, value.clone());
                        }
                    }
                    ui.label(txt(
                    &format!(
                        tr!("Everyone sees the price. Buyers pay in credits, which go to the contract owner — {}."),
                        crate::i18n::own(&state::short(&page.owner))
                    ),
                    12.0,
                    Weight::Regular,
                    p.muted,
                ));
                }
                IssuerWindow::Setting(field, edit) => {
                    kv_row(
                        ui,
                        &p,
                        tr!("Setting"),
                        &field_label_of(*field),
                        sans(13.0, Weight::Medium),
                    );
                    match (field, edit) {
                        (SettingField::MaxSupply, SettingEdit::Value) => {
                            field_label(ui, &p, tr!("Supply limit"));
                            text_field(
                                ui,
                                &p,
                                "token-issuer-amount",
                                &mut form.amount,
                                tr!("none"),
                                FieldKind::Amount,
                                |_| {},
                            );
                            ui.label(txt(
                                &format!(
                                    tr!("Not below what is out: {}"),
                                    state::written(page.supply, decimals)
                                ),
                                12.0,
                                Weight::Regular,
                                p.muted,
                            ));
                        }
                        (SettingField::NewTokensTo, SettingEdit::Value) => {
                            text_field(
                                ui,
                                &p,
                                "token-issuer-who",
                                &mut form.who,
                                "@name",
                                FieldKind::Text,
                                |_| {},
                            );
                        }
                        (
                            SettingField::ChooseDestination | SettingField::TradeMode,
                            SettingEdit::Value,
                        ) => {
                            segmented(ui, &p, &[tr!("Off"), tr!("On")], &mut form.choice, true);
                        }
                        (SettingField::Names, SettingEdit::Value) => {
                            names_rows(ui, a, form);
                            ui.add(para(
                                &format!(
                                    tr!(
                                        "Each name takes 3 to 25 bytes and no spaces, English is needed; the decimals stay {}."
                                    ),
                                    decimals
                                ),
                                12.0,
                                Weight::Regular,
                                p.muted,
                            ));
                            let typed: Vec<TokenName> = form
                                .names
                                .iter()
                                .map(|n| TokenName {
                                    lang: n.lang.trim().to_owned(),
                                    one: n.one.trim().to_owned(),
                                    many: n.many.trim().to_owned(),
                                    capitalize: n.capitalize,
                                })
                                .filter(|n| {
                                    !(n.lang.is_empty() && n.one.is_empty() && n.many.is_empty())
                                })
                                .collect();
                            if let Some(problem) = names_problem(&typed) {
                                ui.add(para(
                                    &names_problem_text(&problem),
                                    12.0,
                                    Weight::Regular,
                                    p.danger,
                                ));
                            }
                        }
                        (_, SettingEdit::Makers | SettingEdit::Admins) => {
                            segmented(
                                ui,
                                &p,
                                &[
                                    tr!("nobody"),
                                    tr!("the contract owner"),
                                    tr!("another identity"),
                                ],
                                &mut form.choice,
                                true,
                            );
                            if form.choice == 2 {
                                text_field(
                                    ui,
                                    &p,
                                    "token-issuer-who",
                                    &mut form.who,
                                    "@name",
                                    FieldKind::Text,
                                    |_| {},
                                );
                            }
                        }
                        _ => {}
                    }
                }
            }
            // M7: what it is now and what it becomes.
            if let (IssuerWindow::Setting(field, edit), Some(IssuerOp::Setting(change))) =
                (&form.window, &op)
                && let Some(s) = page.settings.iter().find(|s| s.field == *field)
            {
                let now = match edit {
                    SettingEdit::Makers => taker_text(&s.makers, &page),
                    SettingEdit::Admins => taker_text(&s.admins, &page),
                    SettingEdit::Value => setting_value(s, &page, state::name(&row.info), decimals),
                };
                let then = change_text(change, &page, decimals);
                ui.add(para(
                    &format!(tr!("was {}, becomes {}"), now, then),
                    12.0,
                    Weight::Regular,
                    p.muted,
                ));
            }
            if same {
                ui.label(txt(
                    tr!("It is set this way already."),
                    12.0,
                    Weight::Regular,
                    p.danger,
                ));
            }
            divider(ui, &p);
            kv_row(
                ui,
                &p,
                &format!(tr!("Fee, in credits of {}"), crate::i18n::own(&from)),
                &format!("≈ {}", crate::format::amount(fee as i64)),
                value,
            );
            false
        },
    );
    if let Some(op) = send {
        cx.send(form.password.command(op, ask));
        form.waiting = true;
        form.result = None;
    }
    if closed {
        form.password.forget();
        let done = matches!(form.result, Some(Ok(())));
        let token = form.token_id.clone();
        tokens.issuer = None;
        if done {
            // The page is read again: what changed shows.
            cx.send(Command::ReadTokenPage {
                token_id: token,
                check: None,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(choose: bool, to: Option<&str>, supply: u64, max: Option<u64>) -> TokenPage {
        TokenPage {
            token_id: "T".into(),
            supply,
            max_supply: max,
            owner: "Owner".into(),
            owner_is_me: true,
            actions: vec![(IssuerAction::Mint, Right::Alone)],
            paused: false,
            choose_destination: choose,
            new_tokens_to: to.map(Into::into),
            frozen: vec![],
            tradeable: false,
            names: vec![],
            settings: vec![],
            read_ms: 0,
        }
    }

    /// Names: the typed rows as Platform takes them, an empty row dropped; none while Platform
    /// would refuse them, and none while they are the token's own.
    #[test]
    fn names_typed_are_an_operation_only_when_new_and_fit() {
        let name = |lang: &str, one: &str, many: &str| TokenName {
            lang: lang.into(),
            one: one.into(),
            many: many.into(),
            capitalize: true,
        };
        let mut p = page(true, None, 0, None);
        p.names = vec![name("en", "Coffee", "Coffees")];
        let mut form = IssuerForm::new(
            "T",
            IssuerWindow::Setting(SettingField::Names, SettingEdit::Value),
        );
        form.names = p.names.clone();
        assert!(op_of(&form, 2, &p, None).is_none(), "the token's own");
        form.names.push(name(" ru ", " Кофе ", "Кофе"));
        form.names.push(name("", "", ""));
        match op_of(&form, 2, &p, None) {
            Some(IssuerOp::Setting(SettingChange::Names(n))) => {
                assert_eq!(
                    n,
                    vec![name("en", "Coffee", "Coffees"), name("ru", "Кофе", "Кофе")]
                )
            }
            other => panic!("{other:?}"),
        }
        form.names[0].one = "Evo Coffee".into();
        assert!(op_of(&form, 2, &p, None).is_none(), "a space");
        form.names.remove(0);
        assert!(op_of(&form, 2, &p, None).is_none(), "no English");
    }

    /// The architect's r2: a change to what is set already is no operation — in the values
    /// and in who may change them; a real change is one.
    #[test]
    fn an_unchanged_setting_is_no_operation() {
        let mut p = page(true, Some("Dest"), 0, Some(10_000));
        p.settings = vec![SettingView {
            field: SettingField::MaxSupply,
            makers: Taker::ContractOwner,
            admins: Taker::ContractOwner,
            change: Right::Alone,
            change_who: Right::Alone,
            never: false,
            to_no_one_allowed: true,
        }];
        let setting = |field, edit, f: &dyn Fn(&mut IssuerForm)| {
            let mut form = IssuerForm::new("T", IssuerWindow::Setting(field, edit));
            f(&mut form);
            op_of(&form, 2, &p, None)
        };
        let v = SettingEdit::Value;
        // The limit: 100.00 is set.
        assert!(setting(SettingField::MaxSupply, v, &|f| f.amount = "100".into()).is_none());
        assert!(setting(SettingField::MaxSupply, v, &|f| f.amount = "200".into()).is_some());
        // Where new tokens go: «Dest» is set.
        assert!(setting(SettingField::NewTokensTo, v, &|f| f.who = "Dest".into()).is_none());
        assert!(setting(SettingField::NewTokensTo, v, &|f| f.who = "Other".into()).is_some());
        // The minter chooses: on now.
        assert!(setting(SettingField::ChooseDestination, v, &|f| f.choice = 1).is_none());
        assert!(setting(SettingField::ChooseDestination, v, &|f| f.choice = 0).is_some());
        // Trading: off now.
        assert!(setting(SettingField::TradeMode, v, &|f| f.choice = 0).is_none());
        assert!(setting(SettingField::TradeMode, v, &|f| f.choice = 1).is_some());
        // Who may change the limit: the contract owner now.
        let makers = SettingEdit::Makers;
        assert!(setting(SettingField::MaxSupply, makers, &|f| f.choice = 1).is_none());
        assert!(setting(SettingField::MaxSupply, makers, &|f| f.choice = 0).is_some());
        let admins = SettingEdit::Admins;
        assert!(setting(SettingField::MaxSupply, admins, &|f| f.choice = 1).is_none());
        assert!(
            setting(SettingField::MaxSupply, admins, &|f| {
                f.choice = 2;
                f.who = "Abc".into();
            })
            .is_some()
        );
    }

    fn mint(amount: &str, to_other: bool) -> IssuerForm {
        let mut form = IssuerForm::new("T", IssuerWindow::Mint);
        form.amount = amount.into();
        form.to_other = to_other;
        form.who = "@bob".into();
        form
    }

    /// Must 2: «Yourself» names this identity; no choice — the rule's destination; neither —
    /// no mint and no button.
    #[test]
    fn a_mint_names_where_it_goes() {
        let me = Some("Me111");
        let to = |op: Option<IssuerOp>| match op {
            Some(IssuerOp::Mint { to, .. }) => Some(to),
            _ => None,
        };
        let free = page(true, None, 0, None);
        assert_eq!(
            to(op_of(&mint("1", false), 2, &free, me)),
            Some(Some("Me111".into()))
        );
        assert_eq!(
            to(op_of(&mint("1", true), 2, &free, me)),
            Some(Some("@bob".into()))
        );
        let ruled = page(false, Some("Dest"), 0, None);
        assert_eq!(to(op_of(&mint("1", false), 2, &ruled, me)), Some(None));
        let nowhere = page(false, None, 0, None);
        assert_eq!(to(op_of(&mint("1", false), 2, &nowhere, me)), None);
        assert!(!mint_goes_somewhere(&nowhere));
        assert!(mint_goes_somewhere(&free) && mint_goes_somewhere(&ruled));
    }

    /// Must 1: the limit 100.00, 90.00 out — 10.00 is a mint, 20.00 is none.
    #[test]
    fn a_mint_over_the_limit_is_no_operation() {
        let limited = page(true, None, 9_000, Some(10_000));
        assert!(op_of(&mint("10", false), 2, &limited, Some("Me")).is_some());
        assert!(op_of(&mint("20", false), 2, &limited, Some("Me")).is_none());
    }

    /// The designer's M7: a who-window opens on the taker as it is now.
    #[test]
    fn a_who_window_opens_on_who_it_is_now() {
        let mut form = IssuerForm::new(
            "T",
            IssuerWindow::Setting(SettingField::MaxSupply, SettingEdit::Makers),
        );
        form.choose_taker(&Taker::NoOne);
        assert_eq!(form.choice, 0);
        form.choose_taker(&Taker::ContractOwner);
        assert_eq!(form.choice, 1);
        form.choose_taker(&Taker::Identity("Abc".into()));
        assert_eq!((form.choice, form.who.as_str()), (2, "Abc"));
        assert!(
            op_of(&form, 2, &page(true, None, 0, None), None).is_some_and(|op| !op.needs_tick()),
            "no tick but for «nobody»"
        );
    }

    /// Should 2: 2 decimals, 0.0001 DASH a token — 100 000 credits a unit; 8 decimals, the
    /// same price — no operation, and the step said is 0.001 DASH.
    #[test]
    fn a_price_is_kept_exactly_or_not_at_all() {
        let mut form = IssuerForm::new("T", IssuerWindow::Price);
        form.price_kind = 1;
        form.tiers = vec![(String::new(), "0.0001".into())];
        let p = page(true, None, 0, None);
        match op_of(&form, 2, &p, None) {
            Some(IssuerOp::Price(Some(tiers))) => assert_eq!(tiers, vec![(1, 100_000)]),
            other => panic!("{other:?}"),
        }
        assert!(op_of(&form, 8, &p, None).is_none());
        assert_eq!(per_unit_of(10_000, 2), Some(100_000));
        assert_eq!(per_unit_of(10_000, 8), None);
        assert_eq!(price_step(8), 100_000, "0.001 DASH");
        assert_eq!(price_of(100_000, 2), 10_000, "worked back: 0.0001 DASH");
        assert_eq!(price_step(2), 1, "any duff");
    }

    /// BUG-222: a failed read ends «Reading…»: the page says so and tries again by itself after
    /// 5, 15 and 45 s, then only on «Try again». A failure already taken changes nothing.
    #[test]
    fn a_page_that_cannot_be_read_says_so_and_tries_again_later_each_time() {
        use std::time::{Duration, Instant};
        let mut pst = super::PageState::new("T");
        let t0 = Instant::now();
        assert!(!pst.on_reads(None, t0));
        assert!(!pst.failed(), "reading");
        assert!(!pst.on_reads(Some(1), t0), "not at once");
        assert!(pst.failed());
        assert_eq!(pst.retry_at, Some(t0 + Duration::from_secs(5)));
        assert!(!pst.on_reads(Some(1), t0 + Duration::from_secs(4)));
        assert!(
            pst.on_reads(Some(1), t0 + Duration::from_secs(5)),
            "after 5 s"
        );
        assert!(!pst.failed(), "reading again");
        let t1 = t0 + Duration::from_secs(8);
        assert!(!pst.on_reads(Some(2), t1));
        assert!(pst.failed());
        assert_eq!(pst.retry_at, Some(t1 + Duration::from_secs(15)), "later");
        pst.try_again();
        assert!(!pst.failed(), "«Try again»: reading");
        assert!(!pst.on_reads(Some(3), t1));
        assert_eq!(pst.retry_at, Some(t1 + Duration::from_secs(45)));
        pst.try_again();
        assert!(!pst.on_reads(Some(4), t1));
        assert!(pst.failed());
        assert_eq!(pst.retry_at, None, "then only on «Try again»");
    }

    /// BUG-222 review r1, Should: reopened after three failures, the page reads, and does not
    /// say «could not be read» nor wait 45 s for a failure of the earlier visit; a new one counts.
    #[test]
    fn a_page_reopened_after_failures_reads_again() {
        use std::time::{Duration, Instant};
        let mut pst = super::PageState::new("T");
        let t0 = Instant::now();
        pst.opened(Some(3));
        assert!(!pst.on_reads(Some(3), t0));
        assert!(!pst.failed(), "reading");
        assert_eq!(pst.retry_at, None, "no 45 s wait");
        assert!(!pst.on_reads(Some(4), t0));
        assert!(pst.failed(), "a failure after opening");
        assert_eq!(
            pst.retry_at,
            Some(t0 + Duration::from_secs(5)),
            "the first since opening: 5 s"
        );
        let mut fresh = super::PageState::new("U");
        fresh.opened(None);
        assert!(!fresh.on_reads(Some(1), t0));
        assert_eq!(fresh.retry_at, Some(t0 + Duration::from_secs(5)));
    }
}
