//! 04 · Receive: where to receive (Core address, Platform address or the DPNS name), a QR
//! code with an optional amount (BIP21 `dash:` link) and what happens next.

use super::txt;
use crate::components::{
    Assets, ButtonKind, TxRow, amount_field, button, field_label, list_card, notice, option_card,
    page_header, tx_row_clickable,
};
use crate::context::Ctx;
use crate::format::DUFFS_PER_DASH;
use crate::text::short_address;
use crate::theme::{Weight, mono};
use egui::{Color32, CornerRadius, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use evolite_api::{Activity, ActivityKind, Chain, Network, Wallet};

struct Target {
    icon: &'static str,
    title: String,
    label: String,
    value: String,
    /// Whether the QR code and the copy button carry a `dash:` link with the amount: Core only,
    /// BIP21 has no form for a Platform address.
    uri: bool,
    /// The DPNS name, copied as a name; otherwise an address.
    name: bool,
    note_icon: &'static str,
    note: &'static str,
    footer: &'static str,
}

/// Where the wallet can receive: fresh addresses from the core and the identity's name.
fn targets(w: &Wallet, network: Network) -> Vec<Target> {
    let net = match network {
        Network::Testnet => "testnet",
        Network::Mainnet => "mainnet",
    };
    let mut out = vec![
        Target {
            icon: "core",
            title: "Core".into(),
            label: format!(tr!("Your Core address · {net}"), net = net),
            value: w.core_address.clone(),
            uri: true,
            name: false,
            note_icon: "pending",
            note: tr!(
                "Waiting for the payment. With InstantSend it is confirmed in seconds and appears in «History»."
            ),
            footer: tr!(
                "After each payment evolite shows a new address — it is more private. Old addresses keep working."
            ),
        },
        Target {
            icon: "platform-address",
            title: tr!("Platform address").into(),
            label: format!(tr!("Your Platform address · {net}"), net = net),
            value: w.platform_address.clone(),
            uri: false,
            name: false,
            note_icon: "pending",
            note: tr!(
                "Waiting for the transfer. A payment on Platform is confirmed by a proof in a couple of seconds."
            ),
            footer: tr!(
                "A Platform address takes DASH without an identity. They can be moved to credits or to Core later."
            ),
        },
    ];
    if let Some(name) = w.identity.as_ref().and_then(|i| i.name()) {
        out.push(Target {
            icon: "username",
            title: format!(tr!("My name @{name}"), name = name),
            label: tr!("Your name in Dash").into(),
            value: format!("@{name}"),
            uri: false,
            name: true,
            note_icon: "confirmed",
            note: tr!("Wallets that support DPNS find your address by name and check that it is yours."),
            footer: tr!("A name is easier to say than an address. Payments by name arrive at your Core address."),
        });
    }
    out
}

#[derive(Default)]
pub struct ReceiveForm {
    pub target: usize,
    pub amount: String,
}

impl ReceiveForm {
    /// The person left the screen (App calls it): coming back, "Request an amount" starts
    /// empty — what was typed (a mistake too) does not wait there (BUG-39 №10).
    pub fn leave(&mut self) {
        self.amount.clear();
    }
}

/// The control that copies `what` on "Receive": it says "Copied" for a moment after a click.
fn copy_id(what: &str) -> egui::Id {
    egui::Id::new("receive-copy").with(what)
}

/// BIP21: `dash:<address>?amount=0.25`, the amount with a dot and without trailing zeros.
pub fn payment_uri(address: &str, duffs: Option<i64>) -> String {
    match duffs {
        Some(d) if d > 0 => {
            let mut fraction = format!("{:08}", d % DUFFS_PER_DASH);
            while fraction.ends_with('0') {
                fraction.pop();
            }
            let whole = d / DUFFS_PER_DASH;
            if fraction.is_empty() {
                format!("dash:{address}?amount={whole}")
            } else {
                format!("dash:{address}?amount={whole}.{fraction}")
            }
        }
        _ => format!("dash:{address}"),
    }
}

pub fn receive(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, form: &mut ReceiveForm) {
    let p = a.p;
    let all = targets(cx.wallet(), cx.snap.network);
    form.target = form.target.min(all.len() - 1);
    ui.spacing_mut().item_spacing.y = 24.0;
    page_header(ui, a, tr!("Receive"), None, |_, _| {});
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        for (i, t) in all.iter().enumerate() {
            if option_card(ui, a, t.icon, &t.title, "", i == form.target).clicked() {
                form.target = i;
            }
        }
    });

    let t = &all[form.target];
    let amount = crate::format::parse_amount(&form.amount).filter(|d| *d > 0);
    let uri = payment_uri(&t.value, amount);
    let qr_payload = if t.uri {
        uri.as_str()
    } else {
        t.value.as_str()
    };

    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 32.0;
        qr_card(ui, &p, qr_payload);
        ui.vertical(|ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 16.0;
            ui.label(txt(&t.label, 13.0, Weight::Medium, p.muted));
            ui.add(
                egui::Label::new(
                    egui::RichText::new(crate::i18n::t(&t.value))
                        .font(mono(20.0, Weight::Medium))
                        .color(p.text)
                        .line_height(Some(28.0)),
                )
                .wrap(),
            );
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let copy = if t.name {
                    tr!("Copy name")
                } else {
                    tr!("Copy address")
                };
                // "Copied" for a moment on the button that copied (what it copies is its id:
                // another target or amount is a fresh button).
                let ctx = ui.ctx().clone();
                let label = |name: &'static str, what: &str| {
                    if crate::clipboard::just_copied(&ctx, copy_id(what)) {
                        tr!("Copied")
                    } else {
                        name
                    }
                };
                if button(ui, a, label(copy, &t.value), ButtonKind::Secondary, None).clicked() {
                    crate::clipboard::copy_answered(ui.ctx(), t.value.clone(), copy_id(&t.value));
                }
                if t.uri
                    && button(
                        ui,
                        a,
                        label(tr!("dash: link"), &uri),
                        ButtonKind::Secondary,
                        None,
                    )
                    .on_hover_text(crate::i18n::t(&uri).into_owned())
                    .clicked()
                {
                    crate::clipboard::copy_answered(ui.ctx(), uri.clone(), copy_id(&uri));
                }
            });
            if t.uri {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    field_label(ui, &p, tr!("Request an amount (optional)"));
                    amount_field(ui, &p, "receive-amount", &mut form.amount, false);
                    if !form.amount.trim().is_empty() && amount.is_none() {
                        ui.label(txt(
                            tr!("Enter an amount, e.g. 0.25"),
                            12.0,
                            Weight::Regular,
                            p.danger,
                        ));
                    }
                });
            }
            notice(ui, a, t.note_icon, p.muted, t.note);
            ui.add(
                egui::Label::new(
                    txt(t.footer, 12.0, Weight::Regular, p.muted).line_height(Some(18.0)),
                )
                .wrap(),
            );
        });
    });

    // "Old addresses keep working": here they are, with what each received.
    let past = if t.uri {
        past_addresses(&cx.wallet().activity, &t.value)
    } else {
        Vec::new()
    };
    if !past.is_empty() {
        let now = cx.now;
        let width = ui.available_width();
        list_card(
            ui,
            a,
            width,
            tr!("Earlier addresses"),
            tr!("still receive payments"),
            |ui, a| {
                ui.add_space(-2.0);
                for old in past.iter().take(PAST_SHOWN) {
                    let amount = crate::format::amount(old.received);
                    let shown = if crate::format::display().hide {
                        crate::format::mask(&amount)
                    } else {
                        amount
                    };
                    // A click copies it: "Copied" stands in the line under it for a moment.
                    let detail = if crate::clipboard::just_copied(ui.ctx(), copy_id(&old.address)) {
                        tr!("Copied").to_string()
                    } else {
                        format!(
                            "{} · {}",
                            crate::text::plural(old.payments, tr!("payment"), tr!("payments")),
                            crate::text::when(old.last, now)
                        )
                    };
                    let row = tx_row_clickable(
                        ui,
                        a,
                        &TxRow {
                            icon: "receive",
                            title: &short_address(&old.address),
                            detail: &detail,
                            proof: None,
                            amount: &shown,
                        },
                    );
                    if row
                        .on_hover_text(
                            crate::i18n::t(tr!("Click to copy the address")).into_owned(),
                        )
                        .clicked()
                    {
                        crate::clipboard::copy_answered(
                            ui.ctx(),
                            old.address.clone(),
                            copy_id(&old.address),
                        );
                    }
                }
            },
        );
    }
}

/// How many earlier addresses "Receive" lists: the latest ones.
const PAST_SHOWN: usize = 5;

/// An address of ours that received before, from the history.
#[derive(Debug, PartialEq)]
pub(crate) struct PastAddress {
    pub address: String,
    pub received: i64,
    pub payments: u32,
    pub last: evolite_api::Timestamp,
}

/// Our Core addresses that received payments, newest first, without `current` (shown above).
pub(crate) fn past_addresses(history: &[Activity], current: &str) -> Vec<PastAddress> {
    let mut out: Vec<PastAddress> = Vec::new();
    for act in history {
        let (ActivityKind::Received { .. }, Chain::Core, Some(address)) =
            (&act.kind, act.chain, &act.address)
        else {
            continue;
        };
        if address == current || act.amount <= 0 {
            continue;
        }
        match out.iter_mut().find(|p| &p.address == address) {
            Some(p) => {
                p.received += act.amount;
                p.payments += 1;
                p.last = p.last.max(act.at);
            }
            None => out.push(PastAddress {
                address: address.clone(),
                received: act.amount,
                payments: 1,
                last: act.at,
            }),
        }
    }
    out.sort_by_key(|p| std::cmp::Reverse(p.last));
    out
}

/// QR card: always dark modules on white with a quiet zone, in both themes, so any camera
/// reads it. 261 px of code inside 20 px of white padding, radius 16.
pub(crate) fn qr_card(ui: &mut Ui, p: &crate::theme::Palette, payload: &str) {
    let side = 261.0;
    let (rect, _) = ui.allocate_exact_size(vec2(side + 40.0, side + 40.0), Sense::hover());
    // Right to left the card moves, the code is not mirrored: a mirrored QR may not scan.
    crate::rtl::keep(ui.ctx(), rect);
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(16),
        Color32::WHITE,
        Stroke::new(1.0, p.line),
        StrokeKind::Inside,
    );
    let Ok(code) = qrcode::QrCode::with_error_correction_level(payload, qrcode::EcLevel::M) else {
        return;
    };
    let n = code.width();
    let colors = code.to_colors();
    // Whole physical pixels per module keep the edges sharp; the rest is centred.
    let quiet = 2;
    let ppp = ui.ctx().pixels_per_point();
    let module = ((side * ppp / (n + 2 * quiet) as f32).floor()).max(1.0) / ppp;
    let drawn = module * (n + 2 * quiet) as f32;
    let origin = rect.center() - vec2(drawn, drawn) / 2.0 + vec2(module, module) * quiet as f32;
    let ink = Color32::from_rgb(0x0B, 0x0D, 0x12);
    for (i, c) in colors.iter().enumerate() {
        if *c == qrcode::Color::Dark {
            let (x, y) = ((i % n) as f32, (i / n) as f32);
            painter.rect_filled(
                Rect::from_min_size(
                    pos2(origin.x + x * module, origin.y + y * module),
                    vec2(module, module),
                ),
                0,
                ink,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PastAddress, past_addresses, payment_uri};
    use evolite_api::{Activity, ActivityKind, Chain, Proof};

    #[test]
    fn builds_bip21_links() {
        assert_eq!(payment_uri("yX", None), "dash:yX");
        assert_eq!(payment_uri("yX", Some(25_000_000)), "dash:yX?amount=0.25");
        assert_eq!(payment_uri("yX", Some(300_000_000)), "dash:yX?amount=3");
        assert_eq!(payment_uri("yX", Some(1)), "dash:yX?amount=0.00000001");
    }

    #[test]
    fn earlier_addresses_add_up_what_they_received() {
        let act = |id, kind, chain, at, amount, address: Option<&str>| Activity {
            id,
            kind,
            chain,
            at,
            amount,
            proof: Proof::ChainLock,
            txid: None,
            fee: None,
            block: None,
            address: address.map(Into::into),
        };
        let received = || ActivityKind::Received { from: None };
        let history = [
            act(1, received(), Chain::Core, 10, 100, Some("yA")),
            act(2, received(), Chain::Core, 30, 50, Some("yA")),
            act(3, received(), Chain::Core, 20, 7, Some("yB")),
            // The address shown above is not repeated below it.
            act(4, received(), Chain::Core, 40, 9, Some("yNow")),
            // A payment out names the payee's address, not ours.
            act(
                5,
                ActivityKind::Sent { to: "yC".into() },
                Chain::Core,
                50,
                -5,
                Some("yC"),
            ),
            // Platform has no Core address; an unknown one is skipped.
            act(6, received(), Chain::Platform, 60, 3, Some("yD")),
            act(7, received(), Chain::Core, 70, 3, None),
        ];
        assert_eq!(
            past_addresses(&history, "yNow"),
            [
                PastAddress {
                    address: "yA".into(),
                    received: 150,
                    payments: 2,
                    last: 30
                },
                PastAddress {
                    address: "yB".into(),
                    received: 7,
                    payments: 1,
                    last: 20
                },
            ]
        );
    }
}
