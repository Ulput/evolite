//! FEAT-0020: an identity's keys. On Identity a small card sums them up with «Manage keys ›»
//! (X1); K1 is their own screen — «‹ {place}» back to where it was opened from, «Keys · @x»,
//! the table «All keys»: number, purpose in words, level, whose, state (the designer's frames
//! K1, Y1, round 2 and 3).

use super::*;
use crate::screens::txt;
use evolite_api::{IdentityKey, KeyLevel, KeyOwner, KeyPurpose};

/// DashPay's data contract, the same on every network: a key bound to it is DashPay's.
const DASHPAY: &str = "Bwr4WHCPz5rFVAD87RqTs3izo4zpzwsEdKPWUT1NS1C7";

/// What a key is for, in words, and the line under it (the designer's table).
pub(crate) fn key_words(key: &IdentityKey) -> (String, Option<String>) {
    // DashPay's only when that is known (AC-T19, as key_plan says): bound to it, or this
    // wallet's own; an unbound one of another wallet is «unbound».
    let bound = key.bound_to.as_deref() == Some(DASHPAY);
    let dashpay = evolite_api::key_plan::is_dashpay(key);
    let unbound = key.bound_to.is_none();
    let (title, sub): (String, Option<String>) = match (&key.purpose, key.level) {
        (KeyPurpose::Identity, _) | (_, KeyLevel::Master) => (
            tr!("Main key").into(),
            Some(tr!("changes the identity's keys").into()),
        ),
        (KeyPurpose::App(name), _) => (tr!("Signing in to an app").into(), Some(name.clone())),
        (KeyPurpose::Transfers, _) => (tr!("Credit transfers").into(), None),
        (KeyPurpose::Encryption, _) => {
            let title = match (key.decrypts, dashpay, unbound) {
                (false, true, _) => tr!("Encryption · DashPay"),
                (true, true, _) => tr!("Decryption · DashPay"),
                (false, false, true) => tr!("Encryption · unbound"),
                (true, false, true) => tr!("Decryption · unbound"),
                (false, false, false) => tr!("Encryption"),
                (true, false, false) => tr!("Decryption"),
            };
            let sub = bound.then(|| tr!("bound to the DashPay contract").to_owned());
            (title.into(), sub)
        }
        (KeyPurpose::Documents, KeyLevel::Critical) => {
            (tr!("Important actions · tokens").into(), None)
        }
        (KeyPurpose::Documents, KeyLevel::High) => {
            (tr!("Signing in to apps and sites").into(), None)
        }
        (KeyPurpose::Documents, _) => (tr!("Signing in, everyday actions").into(), None),
    };
    // An app's key by whose it is (the real core says so in `owner`), another wallet's with why
    // it matters.
    match &key.owner {
        KeyOwner::App(name) => (tr!("Signing in to an app").into(), Some(name.clone())),
        KeyOwner::Other => (title, Some(tr!("evolite cannot sign with it").into())),
        _ => (title, sub),
    }
}

/// Whose a key is, in words: «—» while the wallet is closed.
pub(crate) fn owner_words(owner: &KeyOwner) -> String {
    match owner {
        KeyOwner::This => tr!("this wallet's").into(),
        KeyOwner::App(name) => format!(tr!("app {}"), name),
        KeyOwner::Other => tr!("another wallet").into(),
        KeyOwner::Unknown => "—".into(),
    }
}

/// The table's order: the keys that act by number, then the disabled ones (the designer).
pub(crate) fn ordered(keys: &[IdentityKey]) -> Vec<&IdentityKey> {
    let mut out: Vec<&IdentityKey> = keys.iter().collect();
    out.sort_by_key(|k| (k.disabled_at.is_some(), k.id));
    out
}

/// X1: on Identity, the keys summed up — this wallet's, apps', another wallet's, disabled — and
/// «Manage keys ›». True when that was clicked.
pub(super) fn keys_summary(
    ui: &mut Ui,
    a: &mut Assets,
    id: &Identity,
    width: f32,
    cell: &mut RowCell,
) -> bool {
    let count = id.keys.len().to_string();
    let mut manage = false;
    list_card_in(ui, a, cell, width, (tr!("Keys"), &count), |ui, a| {
        let p = a.p;
        let active = || id.active_keys();
        let this = active().filter(|k| k.owner == KeyOwner::This).count();
        let mut apps: Vec<&str> = active()
            .filter_map(|k| match &k.owner {
                KeyOwner::App(name) => Some(name.as_str()),
                _ => None,
            })
            .collect();
        apps.dedup();
        let other = active().filter(|k| k.owner == KeyOwner::Other).count();
        let disabled = id.keys.len() - active().count();
        let mut line = |label: &str, value: String| {
            divider(ui, &p);
            kv_row(ui, &p, label, &value, style(UI));
        };
        if this > 0 {
            line(
                tr!("This wallet's"),
                format!(tr!("{} · active"), crate::format::count(this as u64)),
            );
        }
        if !apps.is_empty() {
            line(
                tr!("Apps"),
                format!(
                    "{} · {}",
                    crate::format::count(apps.len() as u64),
                    apps.join(", ")
                ),
            );
        }
        if other > 0 {
            line(
                tr!("Another wallet"),
                format!(
                    tr!("{} · evolite does not sign with them"),
                    crate::format::count(other as u64)
                ),
            );
        }
        if disabled > 0 {
            line(tr!("Disabled"), crate::format::count(disabled as u64));
        }
        divider(ui, &p);
        ui.add_space(4.0);
        if text_action(ui, a, "key", tr!("Manage keys ›")).clicked() {
            manage = true;
        }
    });
    manage
}

/// K1's own state. A shown private key lives here only while K1 is on screen and the wallet
/// open: [`KeysState::hide_unless_on`] and [`KeysState::forget`] drop it (and it wipes itself).
#[derive(Default)]
pub struct KeysState {
    /// The row whose «⋯» menu is open: its key and where its «⋯» is.
    menu: Option<(u32, Rect)>,
    /// K6 / K6b: a private key asked for or shown (R-10).
    pub(crate) reveal: Option<super::key_reveal::Reveal>,
    /// K3, K3b, K4, K5: a key change, from its dialog to the core's answer.
    pub(crate) change: Option<super::key_change::KeyChange>,
    /// The last change went through: said over the table until the next one.
    pub(crate) changed: bool,
    /// АРХ-4: the identity this visit asked Platform for again, and how that went — none while
    /// it is being read («checking…»), else when it was read (`Ok`) or why not.
    read_for: Option<String>,
    read: Option<Result<evolite_api::Timestamp, CoreError>>,
}

impl KeysState {
    /// The core's answer to `ReadIdentityKeys`.
    pub fn on_read(&mut self, result: Result<(), CoreError>) {
        if self.read_for.is_some() {
            self.read = Some(result.map(|()| chrono::Local::now().timestamp()));
        }
    }

    /// The line under the title: «checking…» until the identity is read again, then when.
    fn read_words(&self, now: chrono::DateTime<chrono::Local>) -> String {
        match &self.read {
            None => tr!("checking…").into(),
            Some(Ok(at)) => format!(tr!("checked {}"), text::time_of_day(*at, now)),
            Some(Err(_)) => tr!("not read again: the keys as last seen").into(),
        }
    }

    /// The core's answer to `RevealIdentityKey`: to the dialog that waits for it, if any.
    pub fn on_revealed(&mut self, result: Result<evolite_api::RevealedKey, CoreError>) {
        if let Some(r) = &mut self.reveal {
            r.on_revealed(result);
        }
    }

    /// The core's answer to `UpdateKeys`: done closes the dialog and says so; anything else
    /// stays in the dialog.
    pub fn on_changed(&mut self, result: Result<(), CoreError>) {
        if let Some(change) = &mut self.change {
            self.changed = result.is_ok() && change.waiting;
            // The core read the identity again to see the change.
            if self.changed {
                self.read = Some(Ok(chrono::Local::now().timestamp()));
            }
            change.on_changed(result);
        }
    }

    /// K7: K5 with «For DashPay» chosen (from K1 or from Contacts).
    pub fn open_enable_dashpay(&mut self) {
        self.open_change(super::key_change::KeyChange::adding(
            evolite_api::key_plan::KeyRequest::DashPay,
        ));
    }

    /// Opens a key change.
    pub(crate) fn open_change(&mut self, change: super::key_change::KeyChange) {
        self.menu = None;
        self.changed = false;
        self.change = Some(change);
    }

    /// A lock or another wallet: the menu, the dialogs, the password and the key go.
    pub fn forget(&mut self) {
        self.menu = None;
        self.reveal = None;
        self.change = None;
        self.changed = false;
        self.read_for = None;
        self.read = None;
    }

    /// Off K1, nothing of a key stays — not until the next time the screen is drawn. A change
    /// sent goes on in the core; its answer finds no dialog.
    pub fn hide_unless_on(&mut self, on: bool) {
        if !on
            && (self.menu.is_some()
                || self.reveal.is_some()
                || self.change.is_some()
                || self.read_for.is_some())
        {
            self.forget();
        }
    }

    /// `--screen key-disable`, `key-replace-main`, `key-replace-dashpay`, `key-add`: K3 (key 1),
    /// K4, K3b and K5 open over K1 (for the designer's frames, ar among them).
    #[cfg(feature = "dev-tools")]
    pub fn change_for_preset(&mut self, preset: &str) {
        use super::key_change::{ChangeKind, KeyChange};
        let kind = match preset {
            "key-disable" => ChangeKind::Disable(1),
            "key-replace-main" => ChangeKind::ReplaceMaster,
            "key-replace-dashpay" => ChangeKind::ReplaceDashPay,
            _ => ChangeKind::Add,
        };
        if preset == "key-enable-dashpay-foreign" {
            // K7 over another wallet's key (with `identity-keys-foreign`'s key 6).
            self.open_enable_dashpay();
            return;
        }
        self.open_change(KeyChange::new(kind));
    }

    /// `--screen identity-key-shown`: key 2's dialog, understood, the password typed and sent.
    #[cfg(feature = "dev-tools")]
    pub fn reveal_for_preset(&mut self) {
        let mut r = super::key_reveal::Reveal::of(2);
        r.understood = true;
        r.password = zeroize::Zeroizing::new("demo".into());
        r.ask = true;
        self.reveal = Some(r);
    }
}

/// Whether K1 offers a key's private half: this wallet's acting keys only (R-10) — never an
/// app's, another wallet's or a disabled one, so the core's refusal is not met.
pub(crate) fn may_show(key: &IdentityKey) -> bool {
    key.disabled_at.is_none() && key.owner == KeyOwner::This
}

/// K1: «‹ {place}», «Keys · @x», «All keys» and what the table means.
pub fn identity_keys(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, state: &mut KeysState) {
    let p = a.p;
    let wallet = cx.wallet().clone();
    let Some(id) = wallet.identity.as_ref() else {
        // No identity to show: back where it came from.
        cx.go_back();
        return;
    };
    let me = identity_named(id);
    // АРХ-4: each visit reads the identity from Platform again; until then «checking…».
    if state.read_for.as_deref() != Some(id.id.as_str()) {
        state.read_for = Some(id.id.clone());
        state.read = None;
        cx.send(Command::ReadIdentityKeys {
            identity: id.id.clone(),
        });
    }
    ui.spacing_mut().item_spacing.y = 24.0;
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        if crate::components::back_link(ui, a, super::super::place_name(cx.back)).clicked() {
            cx.go_back();
        }
        page_header(ui, a, &format!(tr!("Keys · {}"), me), None, |ui, a| {
            use super::key_change::{ChangeKind, KeyChange};
            // The designer's header: «+ Add a key» and «Replace the main key», off where the
            // main key is not this wallet's (nothing could sign the change).
            let ours = id
                .active_keys()
                .any(|k| k.level == KeyLevel::Master && k.owner == KeyOwner::This);
            if crate::components::button_enabled(
                ui,
                a,
                tr!("+ Add a key"),
                ButtonKind::Primary,
                None,
                ours,
            )
            .clicked()
            {
                state.open_change(KeyChange::new(ChangeKind::Add));
            }
            if crate::components::button_enabled(
                ui,
                a,
                tr!("Replace the main key"),
                ButtonKind::Secondary,
                None,
                ours,
            )
            .clicked()
            {
                state.open_change(KeyChange::new(ChangeKind::ReplaceMaster));
            }
        });
        ui.label(txt(
            &state.read_words(cx.now),
            12.0,
            Weight::Regular,
            p.muted,
        ));
    });
    // K7: an identity made elsewhere without encryption keys gets DashPay's pair here (AC-D8).
    if needs_dashpay(id) && enable_dashpay_card(ui, a, id, &me, &cx.wallet().fees) {
        state.open_enable_dashpay();
    }
    if state.changed {
        notice(
            ui,
            a,
            "check",
            p.accent,
            tr!("The keys changed: Platform has the new set."),
        );
    }
    card(&p, Margin::same(20)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.label(txt(tr!("All keys"), 16.0, Weight::SemiBold, p.text));
        ui.add_space(12.0);
        if let Some(more) = key_table(ui, a, &id.keys, cx.now) {
            state.menu = match state.menu {
                Some((open, _)) if open == more.0 => None,
                _ => Some(more),
            };
        }
    });
    if let Some((key_id, anchor)) = state.menu {
        use super::key_change::{ChangeKind, KeyChange};
        let items = id
            .keys
            .iter()
            .find(|k| k.id == key_id)
            .map(menu_items)
            .unwrap_or_default();
        if let Some(chosen) = row_menu(ui, a, anchor, &items) {
            state.menu = None;
            match chosen {
                Some(MenuItem::Show) => {
                    state.reveal = Some(super::key_reveal::Reveal::of(key_id));
                }
                Some(MenuItem::Disable) => {
                    state.open_change(KeyChange::new(ChangeKind::Disable(key_id)));
                }
                Some(MenuItem::ReplaceMaster) => {
                    state.open_change(KeyChange::new(ChangeKind::ReplaceMaster));
                }
                Some(MenuItem::ReplaceDashPay) => {
                    state.open_change(KeyChange::new(ChangeKind::ReplaceDashPay));
                }
                None => {}
            }
        }
    }
    super::key_reveal::reveal_dialog(ui, a, cx, &mut state.reveal, id, &me);
    // BUG-184: a change the snapshot shows done after its send is said as done.
    if super::key_change::change_dialog(ui, a, cx, &mut state.change, id, &me) {
        state.changed = true;
    }
    if id.keys.iter().any(|k| k.owner == KeyOwner::Unknown) {
        ui.label(txt(
            tr!("Whose a key is shows when the wallet is open."),
            12.0,
            Weight::Regular,
            p.muted,
        ));
    }
    notice(
        ui,
        a,
        "info",
        p.muted,
        tr!(
            "A key is never deleted — only disabled, for good. evolite derives new keys from this wallet's phrase; another key cannot be entered."
        ),
    );
}

/// The identity in a sentence (K1, K7, Settings): its @name or short ID, as one person's words
/// ([`crate::i18n::own`]) — kept whole and left to right inside Arabic, «@» before the name
/// (the designer, AC-D9; as BUG-118, BUG-119).
pub fn identity_named(id: &Identity) -> String {
    let named = id
        .name()
        .map_or_else(|| short_id(&id.id), |n| format!("@{n}"));
    crate::i18n::own(&named)
}

/// Whether `id` has no keys DashPay could use (K7): made in a wallet that gave it none.
pub fn needs_dashpay(id: &Identity) -> bool {
    !evolite_api::key_plan::has(&id.keys, evolite_api::key_plan::KeyRequest::DashPay)
}

/// K7, on K1 and on Contacts: «DashPay contacts for @x are not enabled yet», why, and «Enable
/// DashPay…» with its fee, roughly. True when that was clicked.
pub fn enable_dashpay_card(
    ui: &mut Ui,
    a: &mut Assets,
    id: &Identity,
    me: &str,
    fees: &evolite_api::Fees,
) -> bool {
    let p = a.p;
    let fee = evolite_api::key_plan::fee(&[evolite_api::key_plan::KeyRequest::DashPay], fees);
    let mut clicked = false;
    card(&p, Margin::same(20)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 8.0;
        let title = format!(tr!("DashPay contacts for {} are not enabled yet"), me);
        ui.add(crate::components::para(&title, 14.0, Weight::SemiBold, p.text));
        // AC-T19: another's encryption key in DashPay's way — said, no contacts promised.
        let why = match evolite_api::key_plan::foreign_encryption(&id.keys).first() {
            Some(first) => format!(
                tr!(
                    "DashPay takes the encryption key with the lowest number — here key {} of another wallet or an app, so contacts do not work in evolite. Enabling DashPay adds this wallet's pair and disables that key."
                ),
                first
            ),
            None => tr!(
                "This identity, made in another wallet, lacks encryption keys: without them a contact request cannot be sent or read. evolite adds a pair of keys from this wallet's phrase — a Platform write paid in the identity's credits."
            )
            .into(),
        };
        ui.add(crate::components::para(&why, 13.0, Weight::Regular, p.muted).line_height(18.0));
        ui.horizontal(|ui| {
            clicked = crate::components::button(
                ui,
                a,
                tr!("Enable DashPay…"),
                ButtonKind::Secondary,
                None,
            )
            .clicked();
            ui.label(txt(
                &format!(tr!("≈ {} in credits"), crate::format::amount(fee)),
                12.0,
                Weight::Regular,
                p.muted,
            ));
        });
    });
    clicked
}

/// The columns' left edges as parts of the row's width: №, purpose, level, whose, state.
const COLUMNS: [f32; 5] = [0.0, 0.06, 0.5, 0.63, 0.82];

/// The table; the key whose «⋯» was clicked, with where its «⋯» is.
fn key_table(
    ui: &mut Ui,
    a: &mut Assets,
    keys: &[IdentityKey],
    now: chrono::DateTime<chrono::Local>,
) -> Option<(u32, Rect)> {
    let p = a.p;
    let mut clicked = None;
    let picks = evolite_api::key_plan::dashpay_picks(keys);
    let width = ui.available_width();
    let x = |i: usize, left: f32| left + COLUMNS[i] * width;
    // The heads.
    let (head, _) = ui.allocate_exact_size(vec2(width, 28.0), Sense::hover());
    for (i, name) in [
        tr!("No."),
        tr!("Purpose"),
        tr!("Level"),
        tr!("Whose"),
        tr!("State"),
    ]
    .into_iter()
    .enumerate()
    {
        crate::components::paint_text(
            ui,
            pos2(x(i, head.left()), head.center().y),
            Align2::LEFT_CENTER,
            name,
            style(SMALL),
            p.muted,
        );
    }
    for key in ordered(keys) {
        divider(ui, &p);
        let (title, sub) = key_words(key);
        // AC-T19: a fact, not a guess — the keys DashPay takes, by number, whoever's they are.
        let sub = if picks.contains(&key.id) {
            Some(tr!("DashPay uses this key").to_owned())
        } else {
            sub
        };
        let height = if sub.is_some() { 52.0 } else { 40.0 };
        let (row, resp) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
        let ink = if key.disabled_at.is_some() {
            p.muted
        } else {
            p.text
        };
        let cy = row.center().y;
        let text_at = |ui: &Ui, i: usize, y: f32, s: &str, font, color| {
            crate::components::paint_text(
                ui,
                pos2(x(i, row.left()), y),
                Align2::LEFT_CENTER,
                s,
                font,
                color,
            );
        };
        text_at(ui, 0, cy, &key.id.to_string(), style(UI.mono()), ink);
        match &sub {
            Some(sub) => {
                text_at(
                    ui,
                    1,
                    cy - 9.0,
                    &title,
                    style(UI.weight(Weight::SemiBold)),
                    ink,
                );
                text_at(ui, 1, cy + 10.0, sub, style(SMALL), p.muted);
            }
            None => text_at(ui, 1, cy, &title, style(UI.weight(Weight::SemiBold)), ink),
        }
        text_at(
            ui,
            2,
            cy,
            text::key_level(key.level),
            style(SMALL.mono()),
            p.muted,
        );
        let owner_ink = match key.owner {
            KeyOwner::App(_) | KeyOwner::Other if key.disabled_at.is_none() => p.text,
            _ => p.muted,
        };
        text_at(ui, 3, cy, &owner_words(&key.owner), style(SMALL), owner_ink);
        match key.disabled_at {
            None => text_at(ui, 4, cy, tr!("active"), style(SMALL), p.muted),
            Some(at) => {
                let words = format!(tr!("disabled {}"), text::date_short(at, now));
                let shown = crate::i18n::t(&words).into_owned();
                let font = style(CAPTION.weight(Weight::Medium));
                let galley = ui.painter().layout_no_wrap(shown, font, p.muted);
                let rect = Rect::from_min_size(
                    pos2(x(4, row.left()), cy - galley.size().y / 2.0 - 3.0),
                    galley.size() + vec2(16.0, 6.0),
                );
                ui.painter().rect_stroke(
                    rect,
                    CornerRadius::same(10),
                    egui::Stroke::new(1.0, p.muted),
                    egui::StrokeKind::Inside,
                );
                ui.painter()
                    .galley(rect.min + vec2(8.0, 3.0), galley, p.muted);
            }
        }
        // Read whole by a screen reader: number, purpose, level, whose, state.
        let state = match key.disabled_at {
            None => crate::i18n::spoken(tr!("active")),
            Some(at) => {
                crate::i18n::spoken(&format!(tr!("disabled {}"), text::date_short(at, now)))
            }
        };
        let spoken = format!(
            "{} · {} · {} · {} · {}",
            key.id,
            crate::i18n::spoken(&title),
            text::key_level(key.level),
            crate::i18n::spoken(&owner_words(&key.owner)),
            state
        );
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &spoken));
        // «⋯»: what can be done with this key (the designer's menu), where anything can.
        if !menu_items(key).is_empty() {
            let dots = Rect::from_center_size(pos2(row.right() - 16.0, cy), vec2(32.0, 32.0));
            let more = ui.interact(dots, ui.id().with(("key-more", key.id)), Sense::click());
            crate::components::named(
                &more,
                egui::WidgetType::Button,
                true,
                &format!(tr!("Key {} — actions"), key.id),
            );
            if more.hovered() {
                ui.painter()
                    .rect_filled(dots, CornerRadius::same(8), p.raised);
            }
            for dx in [-6.0, 0.0, 6.0] {
                ui.painter()
                    .circle_filled(pos2(dots.center().x + dx, cy), 1.8, p.text);
            }
            if more.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                clicked = Some((key.id, dots));
            }
        }
    }
    clicked
}

/// What a row's «⋯» offers (the designer's menu table).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MenuItem {
    Show,
    Disable,
    ReplaceMaster,
    ReplaceDashPay,
}

impl MenuItem {
    fn words(self, p: &crate::theme::Palette) -> (&'static str, &'static str, egui::Color32) {
        match self {
            MenuItem::Show => ("key", tr!("Show the private key"), p.text),
            MenuItem::Disable => ("trash", tr!("Disable key…"), p.danger),
            MenuItem::ReplaceMaster => ("key", tr!("Replace the main key…"), p.text),
            MenuItem::ReplaceDashPay => ("key", tr!("Replace the DashPay pair…"), p.text),
        }
    }
}

/// What can be done with `key` (the designer's table): this wallet's acting keys are shown;
/// the main key only replaced (and only when it is this wallet's), DashPay's pair only replaced
/// whole, any other acting key disabled. A disabled key offers nothing.
pub(crate) fn menu_items(key: &IdentityKey) -> Vec<MenuItem> {
    if key.disabled_at.is_some() {
        return Vec::new();
    }
    let mut items = Vec::new();
    if may_show(key) {
        items.push(MenuItem::Show);
    }
    // Only a pair known to be DashPay's is replaced as a pair (AC-T19); any other key goes alone.
    let dashpay = evolite_api::key_plan::is_dashpay(key);
    if key.level == KeyLevel::Master {
        if key.owner == KeyOwner::This {
            items.push(MenuItem::ReplaceMaster);
        }
    } else if dashpay {
        items.push(MenuItem::ReplaceDashPay);
    } else {
        items.push(MenuItem::Disable);
    }
    items
}

/// A row's «⋯» menu under `anchor` with `items`: Some(Some(item)) when one is chosen,
/// Some(None) when it closes otherwise (a click elsewhere, Esc — taken, so K1 stays), None
/// while it is open.
fn row_menu(
    ui: &mut Ui,
    a: &mut Assets,
    anchor: Rect,
    items: &[MenuItem],
) -> Option<Option<MenuItem>> {
    let p = a.p;
    let mut chosen = None;
    let area = egui::Area::new(egui::Id::new("key-more-menu"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos2(anchor.right() - 250.0, anchor.bottom() + 6.0))
        .show(ui.ctx(), |ui| {
            egui::Frame::new()
                .fill(p.surface)
                .stroke(egui::Stroke::new(1.0, p.line))
                .corner_radius(CornerRadius::same(12))
                .inner_margin(Margin::same(6))
                .show(ui, |ui| {
                    ui.set_width(238.0);
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for item in items {
                        let (icon, label, color) = item.words(&p);
                        let (row, resp) = ui.allocate_exact_size(vec2(238.0, 34.0), Sense::click());
                        crate::components::named(&resp, egui::WidgetType::Button, true, label);
                        if resp.hovered() {
                            ui.painter()
                                .rect_filled(row, CornerRadius::same(8), p.raised);
                        }
                        a.icons.icon(icon, color, &p, 16.0).paint_at(
                            ui,
                            Rect::from_min_size(
                                pos2(row.left() + 12.0, row.center().y - 8.0),
                                vec2(16.0, 16.0),
                            ),
                        );
                        crate::components::paint_text(
                            ui,
                            pos2(row.left() + 36.0, row.center().y),
                            Align2::LEFT_CENTER,
                            label,
                            style(UI.weight(Weight::Medium)),
                            color,
                        );
                        if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                            chosen = Some(*item);
                        }
                    }
                });
        });
    if chosen.is_some() {
        return Some(chosen);
    }
    let outside = ui.input(|i| {
        i.pointer.any_click()
            && i.pointer
                .interact_pos()
                .is_some_and(|at| !area.response.rect.contains(at) && !anchor.contains(at))
    });
    let esc = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    (outside || esc).then_some(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolite_api::KeyKind;

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

    /// FEAT-0020 K1 (the designer's table): each key in words, an app's and another wallet's
    /// with the line that matters; DashPay's bound keys named so.
    #[test]
    fn keys_are_named_by_what_they_are_for() {
        let master = key(0, KeyLevel::Master, KeyPurpose::Identity, KeyOwner::This);
        assert_eq!(
            key_words(&master),
            (
                "Main key".into(),
                Some("changes the identity's keys".into())
            )
        );
        let tokens = key(2, KeyLevel::Critical, KeyPurpose::Documents, KeyOwner::This);
        assert_eq!(key_words(&tokens).0, "Important actions · tokens");
        let mut dashpay = key(4, KeyLevel::Medium, KeyPurpose::Encryption, KeyOwner::This);
        dashpay.bound_to = Some(DASHPAY.into());
        assert_eq!(
            key_words(&dashpay),
            (
                "Encryption · DashPay".into(),
                Some("bound to the DashPay contract".into())
            )
        );
        let app = key(
            6,
            KeyLevel::High,
            KeyPurpose::Documents,
            KeyOwner::App("Yappr".into()),
        );
        assert_eq!(
            key_words(&app),
            ("Signing in to an app".into(), Some("Yappr".into()))
        );
        let other = key(7, KeyLevel::High, KeyPurpose::Documents, KeyOwner::Other);
        assert_eq!(
            key_words(&other),
            (
                "Signing in to apps and sites".into(),
                Some("evolite cannot sign with it".into())
            )
        );
        assert_eq!(owner_words(&KeyOwner::Unknown), "—");
    }

    /// K1: the keys that act by number, the disabled ones last.
    #[test]
    fn disabled_keys_come_last() {
        let mut gone = key(1, KeyLevel::High, KeyPurpose::Documents, KeyOwner::This);
        gone.disabled_at = Some(1);
        let keys = [
            key(3, KeyLevel::Critical, KeyPurpose::Transfers, KeyOwner::This),
            gone,
            key(0, KeyLevel::Master, KeyPurpose::Identity, KeyOwner::This),
        ];
        let ids: Vec<u32> = ordered(&keys).iter().map(|k| k.id).collect();
        assert_eq!(ids, [0, 3, 1]);
    }
}
