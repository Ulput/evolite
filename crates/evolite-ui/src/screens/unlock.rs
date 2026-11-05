//! Unlock: the wallet is on this computer, its keys are not open yet. One wallet: made of
//! the onboarding's parts (logo, heading, password field, primary button). Several (
//! Figma 28): "Which wallet to open?" — the wallets to choose from, the chosen one's password.

use super::delete_wallet::{DeleteForm, delete_dialog};
use super::txt;
use crate::components::{
    Assets, ButtonKind, FieldKind, button_enabled, field_label, paint_text, paint_wallet_dot,
    text_field,
};
use crate::context::{Ctx, WalletRequest};
use crate::icons::Logo;
use crate::theme::{Weight, sans};
use egui::{
    Align2, CornerRadius, CursorIcon, Key, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2,
};
use evolite_api::{Command, CoreError, Password, WalletEntry};
use zeroize::{Zeroize, Zeroizing};

#[derive(Default)]
pub struct UnlockForm {
    pub password: Zeroizing<String>,
    /// The password is with the core; the answer comes as `Done { what: "unlock" }` (or
    /// `"switch"` for another wallet chosen).
    pub waiting: bool,
    /// When the password went (`ui.input` time): "Opening… N s" counts while the core works
    /// (designer review №30).
    pub waiting_since: Option<f64>,
    pub error: Option<String>,
    /// "Forgot the password?": the deletion without the password; or, asked from Settings →
    /// Wallets for this wallet, with its password.
    pub forgot: Option<DeleteForm>,
    /// the wallet chosen in the list; none: the one the core has open.
    pub chosen: Option<String>,
}

impl UnlockForm {
    pub fn on_answer(&mut self, result: &Result<(), CoreError>) {
        self.waiting = false;
        self.waiting_since = None;
        self.error = match result {
            Ok(()) => None,
            Err(CoreError::WrongPassword) => Some(tr!("Wrong password").into()),
            Err(CoreError::NotAvailable(why)) => Some(why.clone()),
            Err(e @ CoreError::Failed(_)) => {
                Some(crate::text::failed(tr!("The sync did not start"), e))
            }
            Err(e) => Some(crate::text::failed(tr!("The wallet did not open"), e)),
        };
    }
}

pub fn unlock(
    ui: &mut Ui,
    a: &mut Assets,
    cx: &mut Ctx,
    form: &mut UnlockForm,
    settings: &mut super::settings::SettingsState,
) {
    let p = a.p;
    let snap = cx.snap;
    let several = snap.wallets.len() > 1;
    let column = if several { 400.0 } else { 360.0 };
    let height = if several {
        390.0 + 64.0 * snap.wallets.len() as f32
    } else {
        440.0
    };
    // The wallet the password is for: the one chosen, else the one the core has open.
    let chosen = form
        .chosen
        .as_ref()
        .and_then(|id| snap.wallets.iter().find(|w| &w.id == id))
        .or_else(|| snap.wallets.iter().find(|w| w.open));
    let chosen_id = chosen.map(|w| w.id.clone());
    ui.add_space(((ui.available_height() - height) / 2.0).max(24.0));
    let content = ui.vertical_centered(|ui| {
        ui.spacing_mut().item_spacing.y = 24.0;
        ui.add(a.icons.logo(Logo::Stacked, &p, 96.0));
        if several {
            ui.label(txt(
                tr!("Which wallet to open?"),
                20.0,
                Weight::Medium,
                p.text,
            ));
            ui.add_space(-8.0);
            ui.allocate_ui(vec2(column, 0.0), |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                for w in &snap.wallets {
                    let on = chosen_id.as_deref() == Some(w.id.as_str());
                    if wallet_choice(ui, a, w, on, cx.now, column).clicked() && !on {
                        form.chosen = Some(w.id.clone());
                        form.password.zeroize();
                        form.error = None;
                    }
                }
            });
        } else {
            ui.label(txt(tr!("Welcome back"), 24.0, Weight::Bold, p.text));
        }
        // The label on the left, over the field (Figma 28).
        ui.allocate_ui_with_layout(
            vec2(column, 0.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                match chosen.filter(|_| several) {
                    Some(w) => field_label(
                        ui,
                        &p,
                        &format!(tr!("Password of “{}”"), crate::i18n::own(&w.name)),
                    ),
                    None => field_label(ui, &p, tr!("Password")),
                }
                let field = text_field(
                    ui,
                    &p,
                    "unlock-password",
                    &mut form.password,
                    "",
                    FieldKind::Password,
                    |_| {},
                );
                if field.changed() {
                    form.error = None;
                }
                // The password is what this screen is for: typing starts at once, as when
                // switching wallets.
                if !form.waiting
                    && form.forgot.is_none()
                    && !field.has_focus()
                    && form.password.is_empty()
                {
                    field.request_focus();
                }
                let entered = field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                if let Some(error) = &form.error {
                    ui.label(txt(error, 12.0, Weight::Regular, p.danger));
                }
                ui.add_space(6.0);
                let ready = !form.password.is_empty() && !form.waiting;
                let label = match (form.waiting, several) {
                    (true, _) => opening_label(ui, form.waiting_since),
                    (false, true) => tr!("Open").to_owned(),
                    (false, false) => tr!("Open the wallet").to_owned(),
                };
                let clicked =
                    button_enabled(ui, a, &label, ButtonKind::Primary, Some(column), ready)
                        .clicked();
                if ready && (clicked || entered) {
                    // The password goes to the core and is wiped here at once.
                    let password = Password::new(form.password.to_string());
                    let other = chosen.filter(|w| !w.open);
                    cx.send(match other {
                        // Another wallet: the core checks its password before it locks this one.
                        Some(w) => Command::SwitchWallet {
                            id: w.id.clone(),
                            password: Some(password),
                        },
                        None => Command::Unlock { password },
                    });
                    form.password.zeroize();
                    form.waiting = true;
                    form.waiting_since = Some(ui.input(|i| i.time));
                }
            },
        );
        if !several {
            ui.label(txt(
                tr!("The password opens the phrase encrypted on this computer."),
                13.0,
                Weight::Regular,
                p.muted,
            ));
        }
        // Centred under the button (a text action lays out from the left).
        let link = |text: &str, color| {
            egui::Button::new(txt(text, 13.0, Weight::Medium, color)).frame(false)
        };
        if several {
            ui.add_space(-12.0);
            ui.horizontal(|ui| {
                let w = crate::components::text_width(
                    ui,
                    tr!("Forgot the password? · Add wallet"),
                    sans(13.0, Weight::Medium),
                ) + 24.0;
                ui.add_space(((ui.available_width() - w) / 2.0).max(0.0));
                ui.spacing_mut().item_spacing.x = 8.0;
                if ui
                    .add(link(tr!("Forgot the password?"), p.muted))
                    .on_hover_cursor(CursorIcon::PointingHand)
                    .clicked()
                {
                    // The deletion is the open wallet's: another one chosen opens first.
                    match chosen.filter(|w| !w.open) {
                        Some(w) => cx.wallets(WalletRequest::ForgotOther(w.id.clone())),
                        None => form.forgot = Some(DeleteForm::forgot()),
                    }
                }
                ui.label(txt("·", 13.0, Weight::Medium, p.muted));
                if ui
                    .add(link(tr!("Add wallet"), p.text))
                    .on_hover_cursor(CursorIcon::PointingHand)
                    .clicked()
                {
                    cx.wallets(WalletRequest::Add(None));
                }
            });
        } else if ui
            .add(link(tr!("Forgot the password? · Another wallet"), p.text))
            .on_hover_cursor(CursorIcon::PointingHand)
            .clicked()
        {
            form.forgot = Some(DeleteForm::forgot());
        }
        // Figma 28's foot, as on the welcome screen: the network, the language and the theme,
        // which can be changed before the password (BUG-53) — with one wallet too.
        ui.add_space(24.0);
        super::settings::language_and_theme(ui, a, cx.snap.network, settings);
    });
    super::welcome::dash_evolution_logo(ui, a, content.response.rect.bottom());
    // The wallet is locked: what it holds is not shown, nor what may be unfinished.
    if let Some(forgot) = &mut form.forgot
        && delete_dialog(ui, a, cx, forgot, None, false)
    {
        form.forgot = None;
    }
}

/// One wallet to choose (Figma 28): its circle, name and last time in, ticked when chosen.
fn wallet_choice(
    ui: &mut Ui,
    a: &mut Assets,
    w: &WalletEntry,
    on: bool,
    now: chrono::DateTime<chrono::Local>,
    width: f32,
) -> egui::Response {
    let p = a.p;
    let (row, resp) = ui.allocate_exact_size(vec2(width, 56.0), Sense::click());
    crate::components::named_choice(&resp, egui::WidgetType::RadioButton, on, &shown_name(w));
    ui.painter().rect(
        row,
        CornerRadius::same(12),
        if on || resp.hovered() {
            p.raised
        } else {
            p.surface
        },
        Stroke::new(1.0, if on { p.brand } else { p.line }),
        StrokeKind::Inside,
    );
    let dot = Rect::from_min_size(
        pos2(row.left() + 12.0, row.center().y - 16.0),
        vec2(32.0, 32.0),
    );
    paint_wallet_dot(ui, dot, w);
    let x = dot.right() + 12.0;
    paint_text(
        ui,
        pos2(x, row.top() + 11.0),
        Align2::LEFT_TOP,
        &shown_name(w),
        sans(14.0, Weight::SemiBold),
        p.text,
    );
    let line = match w.last_opened {
        Some(at) => format!(tr!("last opened {}"), crate::text::when(at, now)),
        None => String::new(),
    };
    paint_text(
        ui,
        pos2(x, row.top() + 31.0),
        Align2::LEFT_TOP,
        &line,
        sans(12.0, Weight::Regular),
        p.muted,
    );
    if on {
        a.icons.icon("check", p.accent, &p, 16.0).paint_at(
            ui,
            Rect::from_center_size(pos2(row.right() - 24.0, row.center().y), vec2(16.0, 16.0)),
        );
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// A wallet's name as its owner wrote it: never taken for a word of the interface, even when it
/// is one ("Main", "Savings"), and kept in its own direction inside right-to-left text.
fn shown_name(w: &WalletEntry) -> String {
    crate::i18n::own(&w.name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Lang;

    #[test]
    fn a_wallet_name_that_is_also_a_word_of_the_interface_stays_as_written() {
        let main = WalletEntry {
            id: "w1".into(),
            name: "Main".into(),
            color: 0,
            origin: evolite_api::WalletOrigin::Created,
            created_at: 0,
            last_opened: None,
            open: true,
        };
        // "Main" is a key of the catalogs: the plain name would be translated.
        assert_ne!(crate::i18n::t_in(Lang::Ru, "Main"), "Main");
        let shown = shown_name(&main);
        assert_eq!(crate::i18n::t_in(Lang::Ru, &shown), shown.as_str());
        assert!(shown.contains("Main"));
    }
}

/// "Opening…" with the seconds since the password went, so a slow start shows it is alive;
/// the first second is plain "Opening…".
fn opening_label(ui: &Ui, since: Option<f64>) -> String {
    let secs = since.map_or(0.0, |t| ui.input(|i| i.time) - t).max(0.0) as u64;
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(500));
    opening_text(secs)
}

fn opening_text(secs: u64) -> String {
    if secs < 1 {
        tr!("Opening…").to_owned()
    } else {
        format!(tr!("Opening… {} s"), secs)
    }
}

#[cfg(test)]
mod opening_tests {
    use super::*;

    #[test]
    fn opening_counts_the_seconds_after_the_first() {
        assert_eq!(opening_text(0), "Opening…");
        assert_eq!(opening_text(12), "Opening… 12 s");
    }

    #[test]
    fn an_answer_stops_the_count() {
        let mut form = UnlockForm {
            waiting: true,
            waiting_since: Some(3.0),
            ..Default::default()
        };
        form.on_answer(&Err(CoreError::WrongPassword));
        assert!(!form.waiting && form.waiting_since.is_none());
    }
}
