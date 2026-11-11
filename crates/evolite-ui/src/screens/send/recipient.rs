//! Who the money goes to: what was typed, the address book, a resolved name.

use super::*;

pub(super) enum Recipient<'a> {
    Empty,
    /// Waiting for the core to resolve the name.
    Checking,
    Name(&'a ResolvedName),
    /// No identity for the name, or no answer yet: why.
    NameProblem(&'a CoreError),
    Address,
}

pub(super) fn recipient<'a>(form: &'a SendForm) -> Recipient<'a> {
    let input = form.recipient.trim();
    if input.is_empty() {
        return Recipient::Empty;
    }
    if !input.starts_with('@') {
        return Recipient::Address;
    }
    match &form.resolution {
        _ if form.asked != input => Recipient::Checking,
        None => Recipient::Checking,
        Some(Ok(name)) => Recipient::Name(name),
        Some(Err(e)) => Recipient::NameProblem(e),
    }
}

/// Whether `to` is written like a Core address (Base58, 25–36 characters) or a Platform one
/// (`tdash1…`/`dash1…`): only the core says it really is one, this keeps a stray `q` out of the
/// summary.
pub(super) fn looks_like_address(to: &str) -> bool {
    let to = to.trim();
    let lower = to.to_ascii_lowercase();
    if lower.starts_with("tdash1") || lower.starts_with("dash1") {
        return to.len() >= 40 && to.chars().all(|c| c.is_ascii_alphanumeric());
    }
    (25..=36).contains(&to.chars().count())
        && to
            .chars()
            .all(|c| c.is_ascii_alphanumeric() && !matches!(c, '0' | 'O' | 'I' | 'l'))
}

/// "To" in the summary: "—" until the recipient is one (`known`: a resolved name, an address
/// the core took), then the name, or the address as `yNBw…Reb4` — never a whole line running
/// over the label (BUG-32).
pub(super) fn summary_recipient(typed: &str, known: bool) -> String {
    const NAME_SHOWN: usize = 24;
    let typed = typed.trim();
    if !known || typed.is_empty() {
        return "—".into();
    }
    if typed.starts_with('@') {
        if typed.chars().count() <= NAME_SHOWN {
            return typed.into();
        }
        let head: String = typed.chars().take(NAME_SHOWN - 1).collect();
        return format!("{head}…");
    }
    short_address(typed)
}

/// Whether `to` is an address of this wallet: the one to receive on now, its Platform address,
/// or one that received before.
pub(super) fn own_address(w: &Wallet, to: &str) -> bool {
    own_of(&[&w.core_address, &w.platform_address], &w.activity, to)
}

/// [`own_address`] from the receive addresses `now` and the history: an address that received,
/// from someone else or from this wallet itself (a payment to its own address, BUG-68 — the
/// receive address has moved on since, as «Earlier addresses» of Receive count it).
pub(super) fn own_of(now: &[&str], activity: &[Activity], to: &str) -> bool {
    let to = to.trim();
    !to.is_empty()
        && (now.contains(&to)
            || activity.iter().any(|act| {
                matches!(
                    act.kind,
                    ActivityKind::Received { .. }
                        | ActivityKind::SelfTransfer { .. }
                        | ActivityKind::PlatformReceived
                ) && act.address.as_deref() == Some(to)
            }))
}

/// The address-book entries to offer for what is typed in "To": all of them while it is
/// empty, those whose name or address holds it while it is being typed, none once it is a whole
/// saved address or a name (`@`).
pub(super) fn book_matches<'a>(book: &'a [SavedAddress], typed: &str) -> Vec<&'a SavedAddress> {
    let typed = typed.trim().to_lowercase();
    if typed.starts_with('@') || book.iter().any(|e| e.address.to_lowercase() == typed) {
        return Vec::new();
    }
    book.iter()
        .filter(|e| {
            typed.is_empty()
                || e.label.to_lowercase().contains(&typed)
                || e.address.to_lowercase().contains(&typed)
        })
        .collect()
}

/// A muted text that can be clicked.
fn link(ui: &mut Ui, p: &crate::theme::Palette, text: &str) -> egui::Response {
    let resp = ui
        .add(egui::Label::new(txt(text, 12.0, Weight::Medium, p.muted)).sense(egui::Sense::click()))
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    // A Tab stop: named, or a screen reader says nothing there.
    crate::components::named(&resp, egui::WidgetType::Button, true, text);
    resp
}

/// Under "To", paying from Core: saved addresses to pick; for a saved one its name, to rename
/// or remove; for a good address not saved yet, "Save to the address book" and its name.
pub(super) fn address_book(
    ui: &mut Ui,
    a: &mut Assets,
    form: &mut SendForm,
    book: &[SavedAddress],
    address_ok: bool,
    asked: &mut Vec<Command>,
) {
    let p = a.p;
    let typed = form.recipient.trim().to_string();
    let saved = book.iter().find(|e| e.address == typed);
    let offer = book_matches(book, &typed);
    if !offer.is_empty() {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            field_label(ui, &p, tr!("Address book"));
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                for e in offer {
                    let label = format!("{} · {}", e.label, short_address(&e.address));
                    if pill(ui, &p, &label, false)
                        .on_hover_text(crate::i18n::t(&e.address).into_owned())
                        .clicked()
                    {
                        form.recipient = e.address.clone();
                        form.result = None;
                        form.naming = None;
                    }
                }
            });
        });
    }
    if form.naming.is_none() {
        if let Some(e) = saved {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                ui.label(txt(
                    &format!(
                        tr!("“{}” · from the address book"),
                        crate::i18n::own(&e.label)
                    ),
                    12.0,
                    Weight::Medium,
                    p.accent,
                ));
                if link(ui, &p, tr!("Rename")).clicked() {
                    form.naming = Some(e.label.clone());
                    form.book_error = None;
                }
                if link(ui, &p, tr!("Remove from the book")).clicked() {
                    asked.push(Command::ForgetAddress {
                        address: e.address.clone(),
                    });
                }
            });
        } else if address_ok && link(ui, &p, tr!("Save to the address book")).clicked() {
            form.naming = Some(String::new());
            form.book_error = None;
        }
    }
    let mut cancel = false;
    if let Some(name) = &mut form.naming {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let field_w = (ui.available_width() - 230.0).max(160.0);
            ui.allocate_ui(vec2(field_w, 44.0), |ui| {
                text_field(
                    ui,
                    &p,
                    "send-book-name",
                    name,
                    tr!("A label: exchange, rent, Masha…"),
                    FieldKind::Text,
                    |_| {},
                );
            });
            let ready = !name.trim().is_empty();
            if button_enabled(ui, a, tr!("Save"), ButtonKind::Secondary, None, ready).clicked() {
                asked.push(Command::SaveAddress {
                    address: typed.clone(),
                    label: name.trim().to_owned(),
                });
            }
            if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                cancel = true;
            }
        });
    }
    if cancel {
        form.naming = None;
        form.book_error = None;
    }
    if let Some(e) = &form.book_error {
        ui.label(txt(
            &crate::text::failed(tr!("Not saved"), e),
            12.0,
            Weight::Regular,
            p.danger,
        ));
    }
}

/// The name resolved through DPNS: its avatar and profile name, where the money goes, and
/// "Name verified" — or, paying from Core someone not yet a contact, "Not a contact".
pub(super) fn resolved(
    ui: &mut Ui,
    a: &mut Assets,
    c: &ResolvedName,
    from: Source,
    asked: &mut Vec<Command>,
) {
    let p = a.p;
    let contact = c.contact == Some(ContactState::Established);
    // FEAT-0015 R-1а: from the Platform address a name that is not the wallet's own is no
    // recipient; its card says only whose identity it is, in a plain frame.
    let foreign = from == Source::PlatformAddress;
    let (goes, mark, icon, border) = match (from, contact) {
        _ if foreign => ("", tr!("Name verified"), "confirmed", p.line),
        (Source::Core, true) => (
            tr!("the payment goes to a new address from the contact's key"),
            tr!("Name verified"),
            "confirmed",
            p.accent,
        ),
        (Source::Core, false) => (
            tr!("not in your contacts"),
            tr!("Not a contact"),
            "pending",
            p.line,
        ),
        // The pool cannot pay a name (BUG-112): the card says who it is, nothing about credits.
        (Source::Pool, _) => ("", tr!("Name verified"), "confirmed", p.line),
        _ => (
            tr!("the owner's identity gets the credits"),
            tr!("Name verified"),
            "confirmed",
            p.accent,
        ),
    };
    egui::Frame::new()
        .fill(p.bg)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(12)
        .inner_margin(Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                let (face, _) = ui.allocate_exact_size(vec2(36.0, 36.0), egui::Sense::hover());
                let label = c.display.clone().unwrap_or_else(|| c.name.clone());
                let picture = c.profile.as_ref().and_then(|pr| pr.avatar.as_deref());
                crate::components::avatar(
                    ui,
                    a,
                    face,
                    &c.identity_id,
                    &label,
                    picture,
                    &mut |cmd| asked.push(cmd),
                );
                // The mark first, at the right: the two lines take what is left and end in "…"
                // there, whole on hover, instead of running under "Name verified".
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.label(txt(mark, 12.0, Weight::SemiBold, p.accent));
                    ui.add(a.icons.icon(icon, p.accent, &p, 18.0));
                    ui.add_space(12.0);
                    ui.with_layout(Layout::top_down(Align::Min), |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.add(
                            egui::Label::new(txt(
                                &match &c.display {
                                    Some(display) => format!("@{} · {display}", c.name),
                                    None => format!("@{}", c.name),
                                },
                                14.0,
                                Weight::SemiBold,
                                p.text,
                            ))
                            .truncate(),
                        );
                        ui.add(
                            egui::Label::new(txt(
                                &if goes.is_empty() {
                                    format!("identity {}", short_address(&c.identity_id))
                                } else {
                                    format!("identity {} · {goes}", short_address(&c.identity_id))
                                },
                                12.0,
                                Weight::Regular,
                                p.muted,
                            ))
                            .truncate(),
                        );
                    });
                });
            });
        });
}
