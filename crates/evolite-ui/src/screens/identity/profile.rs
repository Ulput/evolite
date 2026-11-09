//! Editing the profile: the form and its dialog.

use super::*;

/// 23 · Edit profile: the picture's link, the name and "Bio", what writing it costs.
pub(super) fn edit_profile(
    ui: &mut Ui,
    a: &mut Assets,
    state: &mut IdentityState,
    id: &Identity,
    fee: i64,
    ask: bool,
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
                    "Anyone who knows your @name sees the name, “About” and avatar. The picture stays at its link: evolite has no servers of its own."
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
            if let Some(Err(e)) = &form.result
                && !form.password.is_wrong()
            {
                notice(
                    ui,
                    a,
                    "warning",
                    p.danger,
                    &super::super::contacts::contact_error(e),
                );
            }
            let entered = form.password.field(ui, &p, "profile-password", ask);
            let ready = url_ok && !form.waiting && form.password.ready(ask);
            let mut close = false;
            let changed = form.changes(id.profile.as_ref());
            button_row(ui, |ui| {
                // Why «Save» is off: a write costs credits, the same profile again buys nothing.
                if !changed && !form.waiting {
                    ui.label(txt(
                        tr!("Nothing has changed yet"),
                        12.0,
                        Weight::Regular,
                        p.muted,
                    ));
                }
                if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                    close = true;
                }
                let label = if form.waiting {
                    tr!("Saving…")
                } else {
                    tr!("Save")
                };
                if button_enabled(ui, a, label, ButtonKind::Primary, None, ready && changed)
                    .clicked()
                    || (entered && ready && changed)
                {
                    save = true;
                }
            });
            close
        },
    );
    if save {
        let url = form.avatar_url.trim();
        let op = Spend::SaveProfile {
            display: form.display.trim().to_owned(),
            about: form.about.trim().to_owned(),
            avatar_url: (!url.is_empty()).then(|| url.to_owned()),
        };
        asked.push(form.password.command(op, ask));
        form.waiting = true;
        form.result = None;
    } else if close && !form.waiting {
        state.editing = None;
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
    /// "Password for every payment": writing the profile costs credits.
    pub password: SpendPassword,
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

    /// Whether anything differs from what the profile says now (spaces at the ends aside):
    /// writing the same again would only cost credits (design review №16).
    pub fn changes(&self, profile: Option<&Profile>) -> bool {
        let now = Self::from(profile);
        [
            (&self.display, &now.display),
            (&self.about, &now.about),
            (&self.avatar_url, &now.avatar_url),
        ]
        .iter()
        .any(|(typed, saved)| typed.trim() != saved.trim())
    }
}

/// DashPay's limits on a profile (DIP-15).
pub(super) const DISPLAY_MAX: usize = 25;

pub(super) const ABOUT_MAX: usize = 140;

/// Cuts `s` to `max` characters (not bytes).
pub(super) fn keep_chars(s: &mut String, max: usize) {
    if let Some((cut, _)) = s.char_indices().nth(max) {
        s.truncate(cut);
    }
}
