//! 14–17 · Settings: a section list on the left (network, security, backup, appearance,
//! notifications, about) and rows on the right; the phrase dialog asks for the password.
//! Only the theme really changes something in the prototype.

mod about;
mod appearance;
mod backup;
mod network;
mod parts;
pub mod report;
pub mod resync;
mod security;
mod wallets;

use about::{about, notifications};
use appearance::appearance;
pub use backup::check_reminder;
use backup::{backup, check_dialog, phrase_dialog};
use network::network;
pub use network::nodes_dialog;
use security::password_dialog;
use security::security;
pub use wallets::RenameForm;

use super::txt;
use crate::components::Assets;
use crate::context::Ctx;
use crate::prefs::Prefs;
use crate::theme::{Weight, sans};
use egui::{Align2, CornerRadius, CursorIcon, Rect, Response, Sense, Ui, pos2, vec2};
use evolite_api::{Command, CoreError, FiatRate, Phrase, PhraseQuestion, Timestamp};
use std::time::Instant;
use zeroize::{Zeroize, Zeroizing};

#[derive(Clone, Copy, PartialEq)]
pub enum ThemeChoice {
    Dark,
    Light,
    System,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Section {
    ///, Figma 27: the wallets of this computer.
    Wallets,
    Network,
    Security,
    Backup,
    Appearance,
    Notifications,
    About,
}

const SECTIONS: [(Section, &str, &str); 7] = [
    (Section::Wallets, "wallets", tr!("Wallets")),
    (Section::Network, "peers", tr!("Network and nodes")),
    (Section::Security, "lock", tr!("Security")),
    (Section::Backup, "seed", tr!("Backup")),
    (Section::Appearance, "theme-dark", tr!("Appearance")),
    (Section::Notifications, "bell", tr!("Notifications")),
    (Section::About, "info", tr!("About")),
];

pub struct SettingsState {
    pub section: Section,
    pub theme: ThemeChoice,
    pub lock_after: usize,
    pub unit: usize,
    /// NFT animations: 0 as the system says, 1 on, 2 off.
    pub nft_motion: usize,
    /// Switches by key.
    toggles: std::collections::HashMap<&'static str, bool>,
    /// "Number format": 0 as the language writes them, 1 "1 234,56", 2 "1,234.56".
    pub number_style: usize,
    /// "Fiat rate": which currency, by [`CURRENCIES`].
    pub fiat_currency: usize,
    /// "Own node": the form under the switch, while it is open.
    pub node: Option<NodeForm>,
    /// the nodes window, on its Core or Platform tab, while open.
    pub nodes: Option<NodesTab>,
    /// "Remind me later" on the phrase-check reminder: not before this.
    pub remind_snoozed: Option<Timestamp>,
    /// "Restart evolite" was pressed: the app starts again (a new Core node).
    pub restart: bool,
    pub phrase_dialog: bool,
    pub password: Zeroizing<String>,
    /// The core's answer to "Show for 60 seconds", and when it came: the phrase hides after
    /// a minute.
    pub revealed: Option<Result<Phrase, CoreError>>,
    pub revealed_at: Option<Instant>,
    /// "Change password": the form, and the core's answer.
    pub change: Option<PasswordChange>,
    /// "Delete the wallet from this computer": the form, while open.
    pub delete: Option<crate::screens::delete_wallet::DeleteForm>,
    /// "Check the phrase" is open.
    pub check: Option<PhraseCheckForm>,
    /// "Sync again" is open.
    pub resync: Option<resync::ResyncForm>,
    /// The interface language the person chose; `None`: the system's.
    pub lang: Option<crate::i18n::Lang>,
    /// The list of languages is open.
    pub lang_picker: bool,
    /// `--screen about-end`: the section kept scrolled to its end (this far), for screenshots.
    pub scroll_to_end: Option<f32>,
    /// When the phrase check last went right; kept between runs.
    pub phrase_checked: Option<Timestamp>,
    /// "Report a problem" is open.
    pub report: Option<report::ReportForm>,
    /// What "Clear journal" did, shown under it.
    pub journal_note: Option<String>,
    /// Wallets: the wallet whose "…" is open.
    pub wallet_more: Option<String>,
    /// "Rename the wallet", while open.
    pub rename: Option<RenameForm>,
    /// The phrase dialog shows another wallet's phrase (its "…" → "Show the phrase"), by that
    /// wallet's password; none: the open one's.
    pub phrase_wallet: Option<String>,
}

/// which list the nodes window shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NodesTab {
    Core,
    Platform,
}

/// "Own node": the address typed, and the core's answer to saving it.
#[derive(Default)]
pub struct NodeForm {
    pub address: String,
    pub waiting: bool,
    pub result: Option<Result<(), CoreError>>,
}

/// "Fiat rate": the currencies offered, as the rate service names them.
pub const CURRENCIES: [&str; 3] = ["USD", "EUR", "RUB"];

/// How long after the last check (or the wallet's creation) "Remind me to check" asks
/// again: three months, as the switch says.
pub const REMIND_AFTER: Timestamp = 90 * 24 * 60 * 60;
/// "Remind me later": a week.
pub const SNOOZE: Timestamp = 7 * 24 * 60 * 60;

/// "Check the phrase": the password, then the words the core asks. The password is wiped when
/// it goes to the core; the choices go with the dialog.
#[derive(Default)]
pub struct PhraseCheckForm {
    pub password: Zeroizing<String>,
    /// Sent to the core, no answer yet.
    pub waiting: bool,
    /// Press "Start" on the first frame (the screenshot of the asked words).
    pub ask_on_open: bool,
    pub questions: Option<Vec<PhraseQuestion>>,
    /// The choice picked per question.
    pub picks: Vec<Option<usize>>,
    /// The core's verdict: all right, or the number of the first word that did not match.
    pub verdict: Option<Option<usize>>,
    /// Why the words could not be asked (a wrong password).
    pub error: Option<CoreError>,
}

/// The change-password dialog. What is typed is wiped when it goes to the core or the dialog
/// closes.
#[derive(Default)]
pub struct PasswordChange {
    pub old: Zeroizing<String>,
    pub new: Zeroizing<String>,
    pub repeat: Zeroizing<String>,
    pub waiting: bool,
    pub result: Option<Result<(), CoreError>>,
}

/// How long the auto-lock waits, by the segmented control's position; the last one is
/// "on close".
const LOCK_AFTER_SECS: [Option<u64>; 4] = [Some(5 * 60), Some(15 * 60), Some(60 * 60), None];

/// Every switch in Settings, by key: the saved file is read against this list only.
const TOGGLES: [&str; 12] = [
    "clipboard",
    "fiat-rate",
    "hide-amounts",
    "new-address",
    "nft-ask-unknown",
    "notify-apps",
    "notify-incoming",
    "notify-locks",
    "own-node",
    "password-each-send",
    "proxy",
    "remind-check",
];

impl SettingsState {
    /// `section`: which section opens (the phrase dialog opens over Backup).
    pub fn new(dark: bool, section: Section, phrase: bool) -> Self {
        let on = [
            "password-each-send",
            "new-address",
            "clipboard",
            "notify-incoming",
            "notify-locks",
            "remind-check",
            "nft-ask-unknown",
        ];
        Self {
            section,
            theme: if dark {
                ThemeChoice::Dark
            } else {
                ThemeChoice::Light
            },
            lock_after: 1,
            unit: 0,
            nft_motion: 0,
            toggles: on.into_iter().map(|k| (k, true)).collect(),
            number_style: 0,
            fiat_currency: 0,
            node: None,
            nodes: None,
            remind_snoozed: None,
            restart: false,
            phrase_dialog: phrase,
            password: Zeroizing::default(),
            revealed: None,
            revealed_at: None,
            change: None,
            delete: None,
            scroll_to_end: None,
            check: None,
            resync: None,
            lang: None,
            lang_picker: false,
            phrase_checked: None,
            report: None,
            journal_note: None,
            wallet_more: None,
            rename: None,
            phrase_wallet: None,
        }
    }

    /// The core's answer to `AskPhraseCheck`.
    pub fn on_phrase_questions(&mut self, result: &Result<Vec<PhraseQuestion>, CoreError>) {
        if let Some(form) = &mut self.check {
            form.waiting = false;
            match result {
                Ok(questions) => {
                    form.picks = vec![None; questions.len()];
                    form.questions = Some(questions.clone());
                    form.error = None;
                }
                Err(e) => form.error = Some(e.clone()),
            }
        }
    }

    /// The core's answer to `CheckPhraseWords`; `now` becomes "Checked …" when all is right.
    pub fn on_phrase_checked(&mut self, wrong: Option<usize>, now: Timestamp) {
        if let Some(form) = &mut self.check {
            form.waiting = false;
            form.questions = None;
            form.picks.clear();
            form.verdict = Some(wrong);
        }
        if wrong.is_none() {
            self.phrase_checked = Some(now);
        }
    }

    /// The wallet was deleted: what was about it (the phrase checked, its dialogs) goes; the
    /// person's own choices stay for the next wallet.
    pub fn forget_wallet(&mut self) {
        self.forget_secrets();
        self.phrase_checked = None;
    }

    /// Another wallet is open: its own settings start from the defaults (then its file
    /// is applied over them); the app's — theme, language, how amounts look — stay.
    pub fn reset_wallet(&mut self) {
        self.forget_wallet();
        let fresh = Self::new(true, self.section, false);
        self.lock_after = fresh.lock_after;
        self.remind_snoozed = None;
        self.toggles
            .retain(|k, _| crate::prefs::APP_TOGGLES.contains(k));
        for (k, v) in fresh.toggles {
            if !crate::prefs::APP_TOGGLES.contains(&k) {
                self.toggles.insert(k, v);
            }
        }
        self.wallet_more = None;
        self.rename = None;
    }

    /// The check's dialog lives only while Settings is on screen.
    pub fn close_check_unless_shown(&mut self, shown: bool) {
        if !shown && let Some(mut form) = self.check.take() {
            form.password.zeroize();
        }
    }

    fn toggle(&mut self, key: &'static str) -> &mut bool {
        self.toggles.entry(key).or_insert(false)
    }

    /// Whether NFT pictures move, with what the system says (`system_on`).
    pub fn nft_animations(&self, system_on: bool) -> bool {
        match self.nft_motion {
            1 => true,
            2 => false,
            _ => system_on,
        }
    }

    /// A picture from an issuer the person has not trusted waits for "Show".
    pub fn ask_unknown_issuers(&self) -> bool {
        self.toggles.get("nft-ask-unknown").copied().unwrap_or(true)
    }

    /// "Password for every payment".
    pub fn password_each_send(&self) -> bool {
        self.toggles
            .get("password-each-send")
            .copied()
            .unwrap_or(false)
    }

    /// What the core is told for `Command::AutoLock`.
    pub fn auto_lock_secs(&self) -> Option<u64> {
        LOCK_AFTER_SECS[self.lock_after]
    }

    #[cfg(any(feature = "dev-tools", test))]
    pub fn set(&mut self, key: &'static str, on: bool) {
        *self.toggle(key) = on;
    }

    /// A switch by key, with its default when the file had none.
    pub fn is_on(&self, key: &str) -> bool {
        self.toggles.get(key).copied().unwrap_or(false)
    }

    /// How amounts look; `fiat`: the price the core has, if any.
    pub fn display(&self, fiat: Option<&FiatRate>) -> crate::format::Display {
        let rate = self.fiat_currency().and(fiat).and_then(|f| {
            Some(crate::format::Rate {
                price: f.price,
                currency: f.currency.as_bytes().try_into().ok()?,
            })
        });
        crate::format::Display {
            unit: crate::format::Unit::from_index(self.unit),
            style: crate::format::NumberStyle::from_index(self.number_style),
            hide: self.is_on("hide-amounts"),
            rate,
        }
    }

    /// What the core is told for `Command::FiatRate`: the currency while "Fiat rate" is on.
    pub fn fiat_currency(&self) -> Option<&'static str> {
        self.is_on("fiat-rate")
            .then(|| CURRENCIES[self.fiat_currency])
    }

    /// The commands that carry these settings to the core: at the start, and again when a new
    /// core takes over the folder (after a deletion).
    pub fn core_commands(&self) -> [Command; 3] {
        [
            Command::AutoLock {
                after_secs: self.auto_lock_secs(),
            },
            Command::ReceiveAddress {
                fresh: self.is_on("new-address"),
            },
            Command::FiatRate {
                currency: self.fiat_currency().map(str::to_owned),
            },
        ]
    }

    /// "Remind me to check": three months after the last right check — or after the wallet
    /// was made, when it never was — unless put off with "Remind me later".
    pub fn remind_check(&self, created_at: Timestamp, now: Timestamp) -> bool {
        self.is_on("remind-check")
            && now - self.phrase_checked.unwrap_or(created_at).max(created_at) >= REMIND_AFTER
            && self.remind_snoozed.is_none_or(|until| now >= until)
    }

    /// The core's answer to `CoreNode`.
    pub fn on_node_saved(&mut self, result: &Result<(), CoreError>) {
        if let Some(form) = &mut self.node {
            form.waiting = false;
            form.result = Some(result.clone());
        }
    }

    pub fn on_revealed(&mut self, result: Result<Phrase, CoreError>) {
        self.revealed_at = result.is_ok().then(Instant::now);
        self.revealed = Some(result);
    }

    pub fn on_password_changed(&mut self, result: &Result<(), CoreError>) {
        if let Some(change) = &mut self.change {
            change.waiting = false;
            change.result = Some(result.clone());
        }
    }

    /// The shown phrase lives only while its dialog is on screen: another screen, another
    /// section or a lock drops it (and `Phrase` wipes itself), not the next time the dialog
    /// happens to be drawn.
    pub fn hide_phrase_unless_shown(&mut self, shown: bool) {
        if !shown && (self.revealed.is_some() || !self.password.is_empty()) {
            self.revealed = None;
            self.revealed_at = None;
            self.password.zeroize();
        }
    }

    /// What is kept between runs; `trusted` are Collection's trusted issuers.
    pub fn prefs(&self, trusted: &std::collections::HashSet<String>) -> Prefs {
        Prefs {
            theme: Some(
                match self.theme {
                    ThemeChoice::Dark => "dark",
                    ThemeChoice::Light => "light",
                    ThemeChoice::System => "system",
                }
                .into(),
            ),
            lock_after: Some(self.lock_after),
            unit: Some(self.unit),
            nft_motion: Some(self.nft_motion),
            toggles: self
                .toggles
                .iter()
                .map(|(k, v)| ((*k).to_owned(), *v))
                .collect(),
            trusted_issuers: trusted.iter().cloned().collect(),
            // The app fills it in (`App::save_prefs`): the core knows which one is shown.
            identity: None,
            phrase_checked: self.phrase_checked,
            number_style: Some(self.number_style),
            fiat_currency: Some(self.fiat_currency),
            remind_snoozed: self.remind_snoozed,
            lang: self.lang.map(|l| l.code().to_owned()),
        }
    }

    /// Takes back what was saved; anything unknown or out of range keeps its default.
    pub fn apply(&mut self, prefs: &Prefs, trusted: &mut std::collections::HashSet<String>) {
        match prefs.theme.as_deref() {
            Some("dark") => self.theme = ThemeChoice::Dark,
            Some("light") => self.theme = ThemeChoice::Light,
            Some("system") => self.theme = ThemeChoice::System,
            _ => {}
        }
        if let Some(i) = prefs.lock_after.filter(|i| *i < LOCK_AFTER_SECS.len()) {
            self.lock_after = i;
        }
        if let Some(i) = prefs.unit.filter(|i| *i < 3) {
            self.unit = i;
        }
        if let Some(i) = prefs.nft_motion.filter(|i| *i < 3) {
            self.nft_motion = i;
        }
        if let Some(i) = prefs.number_style.filter(|i| *i < 3) {
            self.number_style = i;
        }
        if let Some(i) = prefs.fiat_currency.filter(|i| *i < CURRENCIES.len()) {
            self.fiat_currency = i;
        }
        if prefs.remind_snoozed.is_some() {
            self.remind_snoozed = prefs.remind_snoozed;
        }
        for (key, on) in &prefs.toggles {
            if let Some(known) = TOGGLES.iter().find(|k| **k == key.as_str()) {
                self.toggles.insert(known, *on);
            }
        }
        trusted.extend(prefs.trusted_issuers.iter().cloned());
        if let Some(lang) = prefs.lang.as_deref().and_then(crate::i18n::Lang::from_code) {
            self.lang = Some(lang);
        }
        if prefs.phrase_checked.is_some() {
            self.phrase_checked = prefs.phrase_checked;
        }
    }

    /// The wallet locked: nothing typed or shown that opens it stays on screen or in memory.
    pub fn forget_secrets(&mut self) {
        self.phrase_dialog = false;
        self.phrase_wallet = None;
        self.hide_phrase_unless_shown(false);
        // The deletion form may hold the password.
        self.delete = None;
        self.close_check_unless_shown(false);
        if let Some(mut change) = self.change.take() {
            change.old.zeroize();
            change.new.zeroize();
            change.repeat.zeroize();
        }
    }
}

pub fn settings(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, state: &mut SettingsState) {
    let wallet = cx.wallet();
    let p = a.p;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 32.0;
        // Section list, 208 px.
        ui.vertical(|ui| {
            ui.set_width(208.0);
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.add_space(-4.0);
            ui.label(txt(tr!("Settings"), 28.0, Weight::Bold, p.text));
            ui.add_space(14.0);
            for (section, icon, label) in SECTIONS {
                if section_item(ui, a, icon, label, state.section == section).clicked() {
                    state.section = section;
                }
            }
        });
        ui.vertical(|ui| {
            let mut scroll = egui::ScrollArea::vertical().auto_shrink(false);
            if let Some(end) = state.scroll_to_end {
                scroll = scroll.vertical_scroll_offset(end);
            }
            let out = scroll.show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 20.0;
                ui.add_space(4.0);
                match state.section {
                    Section::Wallets => wallets::wallets(ui, a, cx, state),
                    Section::Network => network(ui, a, cx, state),
                    Section::Security => security(ui, a, cx, state),
                    Section::Backup => backup(ui, a, state, wallet),
                    Section::Appearance => appearance(ui, a, state, cx.snap.fiat.as_ref()),
                    Section::Notifications => notifications(ui, a, state),
                    Section::About => about(ui, a, state),
                }
                // The last row stays clear of the window's edge when scrolled to the end.
                ui.add_space(24.0);
            });
            // Kept at the end next frame: what is there to scroll, from this frame's sizes.
            if state.scroll_to_end.is_some() {
                state.scroll_to_end = Some((out.content_size.y - out.inner_rect.height()).max(0.0));
            }
        });
    });
    if state.phrase_dialog {
        phrase_dialog(ui, a, cx, state);
    }
    if state.change.is_some() {
        password_dialog(ui, a, cx, state);
    }
    if let Some(form) = &mut state.delete {
        let unfinished = wallet.unfinished_identity.is_some() || wallet.unfinished_top_up.is_some();
        if crate::screens::delete_wallet::delete_dialog(
            ui,
            a,
            cx,
            form,
            Some(wallet.balances.total()),
            unfinished,
        ) {
            state.delete = None;
        }
    }
    if state.lang_picker {
        appearance::lang_dialog(ui, a, state);
    }
    if let Some(form) = &mut state.resync
        && resync::resync_dialog(ui, a, cx, form)
    {
        state.resync = None;
    }
    if let Some(tab) = &mut state.nodes
        && nodes_dialog(ui, a, cx, tab)
    {
        state.nodes = None;
    }
    if state.check.is_some() {
        check_dialog(ui, a, cx, state);
    }
    if let Some(form) = &mut state.report
        && report::report_dialog(ui, a, cx, form)
    {
        state.report = None;
    }
    if let Some(form) = &mut state.rename
        && wallets::rename_dialog(ui, a, cx, form)
    {
        state.rename = None;
    }
}

fn section_item(ui: &mut Ui, a: &mut Assets, icon: &str, label: &str, on: bool) -> Response {
    let p = a.p;
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::click());
    crate::components::named_choice(&resp, egui::WidgetType::SelectableLabel, on, label);
    if on || resp.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(10), p.raised);
    }
    let color = if on || resp.hovered() {
        p.text
    } else {
        p.muted
    };
    let icon_rect = Rect::from_min_size(
        pos2(rect.left() + 12.0, rect.center().y - 9.0),
        vec2(18.0, 18.0),
    );
    a.icons.icon(icon, color, &p, 18.0).paint_at(ui, icon_rect);
    ui.painter().text(
        pos2(icon_rect.right() + 10.0, rect.center().y),
        Align2::LEFT_CENTER,
        crate::i18n::t(label),
        sans(14.0, if on { Weight::SemiBold } else { Weight::Medium }),
        color,
    );
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(state: &mut SettingsState) {
        state.phrase_dialog = true;
        state.password = Zeroizing::new("password".into());
        state.on_revealed(Ok(Phrase::new(vec!["abandon".into(); 12])));
    }

    #[test]
    fn the_phrase_goes_when_its_dialog_is_not_on_screen() {
        let mut state = SettingsState::new(true, Section::Backup, false);
        shown(&mut state);
        state.hide_phrase_unless_shown(true);
        assert!(state.revealed.is_some(), "on screen: still shown");
        state.hide_phrase_unless_shown(false);
        assert!(state.revealed.is_none() && state.revealed_at.is_none());
        assert!(state.password.is_empty());
    }

    #[test]
    fn settings_come_back_as_they_were_chosen() {
        let mut chosen = SettingsState::new(true, Section::Security, false);
        chosen.theme = ThemeChoice::Light;
        chosen.lock_after = 0;
        chosen.nft_motion = 2;
        *chosen.toggle("password-each-send") = true;
        *chosen.toggle("nft-ask-unknown") = false;
        let trusted: std::collections::HashSet<String> = ["neon-forge".to_owned()].into();
        let saved = chosen.prefs(&trusted);

        let mut next = SettingsState::new(true, Section::Network, false);
        let mut next_trusted = Default::default();
        next.apply(&saved, &mut next_trusted);
        assert!(next.theme == ThemeChoice::Light);
        assert_eq!(next.auto_lock_secs(), Some(5 * 60));
        assert!(next.password_each_send() && !next.ask_unknown_issuers());
        assert!(!next.nft_animations(true));
        assert!(next_trusted.contains("neon-forge"));
    }

    #[test]
    fn unknown_or_odd_values_keep_the_defaults() {
        let mut state = SettingsState::new(true, Section::Network, false);
        let odd = Prefs {
            theme: Some("purple".into()),
            lock_after: Some(9),
            toggles: [("format-disk".to_owned(), true)].into(),
            ..Prefs::default()
        };
        state.apply(&odd, &mut Default::default());
        assert!(state.theme == ThemeChoice::Dark);
        assert_eq!(state.auto_lock_secs(), Some(15 * 60));
        assert!(!state.toggles.contains_key("format-disk"));
    }

    #[test]
    fn amounts_follow_the_appearance_and_privacy_switches() {
        let mut state = SettingsState::new(true, Section::Appearance, false);
        let usd = FiatRate {
            currency: "USD".into(),
            price: 4_796_150_000,
            at: 0,
        };
        let shown = state.display(Some(&usd));
        assert_eq!(
            shown,
            crate::format::Display::default(),
            "rate off by default"
        );
        state.unit = 2;
        state.number_style = 2;
        state.set("hide-amounts", true);
        state.set("fiat-rate", true);
        let shown = state.display(Some(&usd));
        assert_eq!(shown.unit, crate::format::Unit::Duffs);
        assert_eq!(shown.style, crate::format::NumberStyle::CommaDot);
        assert!(shown.hide && shown.rate.is_some());
        // A price in another currency than chosen is not shown as this one's.
        state.fiat_currency = 1;
        assert_eq!(state.fiat_currency(), Some("EUR"));
        let saved = state.prefs(&Default::default());
        let mut next = SettingsState::new(true, Section::Network, false);
        next.apply(&saved, &mut Default::default());
        assert_eq!(
            (next.unit, next.number_style, next.fiat_currency),
            (2, 2, 1)
        );
    }

    #[test]
    fn the_core_hears_the_receive_address_and_the_rate() {
        let mut state = SettingsState::new(true, Section::Security, false);
        let [_, fresh, rate] = state.core_commands();
        assert!(matches!(fresh, Command::ReceiveAddress { fresh: true }));
        assert!(matches!(rate, Command::FiatRate { currency: None }));
        state.set("new-address", false);
        state.set("fiat-rate", true);
        state.fiat_currency = 2;
        let [_, fresh, rate] = state.core_commands();
        assert!(matches!(fresh, Command::ReceiveAddress { fresh: false }));
        assert!(matches!(rate, Command::FiatRate { currency: Some(c) } if c == "RUB"));
    }

    #[test]
    fn the_phrase_check_is_asked_again_after_three_months() {
        let mut state = SettingsState::new(true, Section::Backup, false);
        let made = 1_780_000_000;
        assert!(!state.remind_check(made, made + REMIND_AFTER - 1));
        assert!(state.remind_check(made, made + REMIND_AFTER));
        // A check restarts the three months.
        state.phrase_checked = Some(made + REMIND_AFTER);
        assert!(!state.remind_check(made, made + REMIND_AFTER + 10));
        assert!(state.remind_check(made, made + 2 * REMIND_AFTER));
        // "Remind me in a week", and the switch.
        let now = made + 2 * REMIND_AFTER;
        state.remind_snoozed = Some(now + SNOOZE);
        assert!(!state.remind_check(made, now + SNOOZE - 1));
        assert!(state.remind_check(made, now + SNOOZE));
        state.set("remind-check", false);
        assert!(!state.remind_check(made, now + SNOOZE));
    }

    #[test]
    fn a_lock_takes_the_phrase_and_the_passwords() {
        let mut state = SettingsState::new(true, Section::Backup, false);
        shown(&mut state);
        state.change = Some(PasswordChange {
            old: Zeroizing::new("old".into()),
            new: Zeroizing::new("new".into()),
            repeat: Zeroizing::new("new".into()),
            ..Default::default()
        });
        state.forget_secrets();
        assert!(!state.phrase_dialog);
        assert!(state.revealed.is_none() && state.password.is_empty());
        assert!(state.change.is_none());
    }
}

#[cfg(test)]
mod phrase_check_tests {
    use super::*;

    fn asked() -> Vec<PhraseQuestion> {
        vec![PhraseQuestion {
            number: 7,
            choices: vec!["museum".into(), "ribbon".into(), "sugar".into()],
        }]
    }

    #[test]
    fn a_right_check_is_dated_and_kept_between_runs() {
        let mut state = SettingsState::new(true, Section::Backup, false);
        state.check = Some(PhraseCheckForm {
            waiting: true,
            ..Default::default()
        });
        state.on_phrase_questions(&Ok(asked()));
        let form = state.check.as_ref().unwrap();
        assert_eq!(form.picks, vec![None]);
        assert!(!form.waiting);
        state.on_phrase_checked(None, 1_793_404_800);
        assert_eq!(state.check.as_ref().unwrap().verdict, Some(None));
        assert!(state.check.as_ref().unwrap().questions.is_none());
        let saved = state.prefs(&Default::default());
        let mut next = SettingsState::new(true, Section::Network, false);
        next.apply(&saved, &mut Default::default());
        assert_eq!(next.phrase_checked, Some(1_793_404_800));
    }

    #[test]
    fn a_wrong_word_is_named_and_not_dated() {
        let mut state = SettingsState::new(true, Section::Backup, false);
        state.check = Some(PhraseCheckForm::default());
        state.on_phrase_questions(&Ok(asked()));
        state.on_phrase_checked(Some(7), 1_793_404_800);
        assert_eq!(state.check.as_ref().unwrap().verdict, Some(Some(7)));
        assert_eq!(state.phrase_checked, None);
        // A lock or another screen closes the dialog and wipes what was typed.
        state.close_check_unless_shown(false);
        assert!(state.check.is_none());
    }
}

#[cfg(test)]
mod forget_wallet_tests {
    use super::*;

    /// a new wallet in the same folder does not show the deleted one's check.
    #[test]
    fn a_deleted_wallet_takes_its_check_but_not_the_persons_choices() {
        let mut state = SettingsState::new(true, Section::Backup, false);
        state.phrase_checked = Some(1_793_404_800);
        state.theme = ThemeChoice::Light;
        state.lock_after = 0;
        state.forget_wallet();
        assert_eq!(state.phrase_checked, None);
        assert!(state.theme == ThemeChoice::Light);
        assert_eq!(state.lock_after, 0);
        assert_eq!(state.prefs(&Default::default()).phrase_checked, None);
    }

    /// another wallet starts from the defaults for its own settings; the app's stay.
    #[test]
    fn another_wallet_has_its_own_settings_and_the_apps_stay() {
        let mut state = SettingsState::new(true, Section::Wallets, false);
        state.theme = ThemeChoice::Light;
        state.unit = 2;
        state.set("hide-amounts", true);
        state.lock_after = 0;
        state.set("password-each-send", false);
        state.phrase_checked = Some(1_793_404_800);
        state.reset_wallet();
        assert!(state.theme == ThemeChoice::Light);
        assert_eq!(state.unit, 2);
        assert!(state.is_on("hide-amounts"));
        assert_eq!(state.lock_after, 1);
        assert!(state.password_each_send());
        assert_eq!(state.phrase_checked, None);
    }
}
