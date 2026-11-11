//! 04 · Receive: where to receive (Core address, Platform address or the DPNS name), a QR
//! code with an optional amount (BIP21 `dash:` link) and what happens next.

use super::txt;
use crate::components::{
    Assets, ButtonKind, TxRow, button_enabled, field_label, notice, option_card, page_header,
    tx_row_clickable,
};
use crate::context::Ctx;
use crate::format::DUFFS_PER_DASH;
use crate::text::short_address;
use crate::theme::{Weight, mono};
use egui::{Color32, CornerRadius, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use evolite_api::{Activity, ActivityKind, Chain, Duffs, Network, Wallet};

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
                "Waiting for the payment. With InstantSend it is confirmed in seconds and appears in “History”."
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
    /// «Request an amount» in the currency of «Fiat rate» (FEAT-0008, UC-2): the unit is
    /// kept for the session; the text typed in it.
    pub in_currency: bool,
    pub fiat: String,
}

impl ReceiveForm {
    /// The person left the screen (App calls it): coming back, "Request an amount" starts
    /// empty — what was typed (a mistake too) does not wait there (BUG-39 №10).
    pub fn leave(&mut self) {
        self.amount.clear();
        self.fiat.clear();
    }

    /// Whether «Request an amount» is on screen: a target with a `dash:` link (Core).
    pub fn requesting(&self, w: &Wallet, network: Network) -> bool {
        let all = targets(w, network);
        all.get(self.target.min(all.len().saturating_sub(1)))
            .is_some_and(|t| t.uri)
    }

    /// The amount asked for, in duffs: typed in the currency it is the DASH it buys at `rate`,
    /// rounded down (R-3); the link and the QR carry DASH only (AC-T10).
    fn requested(&self, rate: Option<&crate::fiat_lines::Rate>) -> Option<Duffs> {
        let typed = match rate.filter(|r| self.in_currency && r.fresh) {
            Some(rate) => rate.dash_text(&self.fiat)?,
            None => self.amount.clone(),
        };
        crate::format::parse_amount(&typed).filter(|d| *d > 0)
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

/// «Request an amount» with, while «Fiat rate» is on, the unit switch and by which rate
/// (frame 5); an old rate takes the currency away and says since when (UC-6).
fn request_field(
    ui: &mut Ui,
    p: &crate::theme::Palette,
    form: &mut ReceiveForm,
    rate: Option<&crate::fiat_lines::Rate>,
    amount: Option<Duffs>,
) {
    let typing = form.in_currency && rate.is_some_and(|r| r.fresh);
    let unit = rate.map(|r| crate::components::UnitChoice {
        currency: &r.code,
        in_currency: &mut form.in_currency,
        off: (!r.fresh).then_some(tr!("The rate is older than 5 minutes")),
    });
    let text = if typing {
        &mut form.fiat
    } else {
        &mut form.amount
    };
    let field = crate::components::amount_field_in(ui, p, "receive-amount", text, false, unit);
    // The amount keeps its meaning across the switch (behaviour 1).
    if field.switched
        && let Some(rate) = rate
    {
        if typing {
            form.amount = rate.dash_text(&form.fiat).unwrap_or_default();
        } else {
            form.fiat = rate.currency_text(&form.amount);
        }
    }
    let Some(rate) = rate else {
        return;
    };
    let muted = |ui: &mut Ui, line: &str| {
        ui.add(egui::Label::new(txt(line, 12.0, Weight::Regular, p.muted)).wrap());
    };
    if !rate.fresh {
        muted(
            ui,
            &format!(
                tr!(
                    "The rate has not been updated since {}: input in {} is unavailable until it updates"
                ),
                crate::fiat_lines::clock(rate.at),
                rate.code
            ),
        );
    } else if typing && let Some(amount) = amount {
        let dash = format!(
            "{} {}",
            crate::format::number(amount, false),
            crate::format::unit_name()
        );
        muted(
            ui,
            &format!(
                tr!("≈ {} by the rate {} · updated {}"),
                dash,
                rate.price_text(),
                crate::fiat_lines::clock(rate.at)
            ),
        );
        muted(ui, tr!("The QR code and the link get the amount in DASH"));
    }
}

pub fn receive(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, form: &mut ReceiveForm) {
    let p = a.p;
    let all = targets(cx.wallet(), cx.snap.network);
    form.target = form.target.min(all.len() - 1);
    ui.spacing_mut().item_spacing.y = 24.0;
    page_header(ui, a, tr!("Receive"), None, |_, _| {});
    // Its own scroll, as the other screens have (BUG-48): «Earlier addresses» goes on under
    // the bottom edge at 1024×768 (BUG-72).
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 24.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                for (i, t) in all.iter().enumerate() {
                    if option_card(ui, a, t.icon, &t.title, "", i == form.target).clicked() {
                        form.target = i;
                    }
                }
            });

            let t = &all[form.target];
            let rate = crate::fiat_lines::Rate::now(cx.now.timestamp());
            let amount = form.requested(rate.as_ref());
            let uri = payment_uri(&t.value, amount);
            // No address yet (the keys are being read): no QR and nothing to copy — a QR without
            // its address says nothing of what it holds (BUG-74 №1).
            let ready = !t.value.trim().is_empty();
            let qr_payload = if t.uri {
                uri.as_str()
            } else {
                t.value.as_str()
            };

            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 32.0;
                if ready {
                    qr_card(ui, &p, qr_payload);
                } else {
                    qr_waiting(ui, &p);
                }
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 16.0;
                    ui.label(txt(&t.label, 13.0, Weight::Medium, p.muted));
                    if ready {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(crate::i18n::t(&t.value))
                                    .font(mono(20.0, Weight::Medium))
                                    .color(p.text)
                                    .line_height(Some(28.0)),
                            )
                            .wrap(),
                        );
                    } else {
                        ui.add(
                            egui::Label::new(
                                txt(
                                    tr!(
                                        "Preparing the address from the wallet keys — no network needed, usually a second."
                                    ),
                                    14.0,
                                    Weight::Regular,
                                    p.muted,
                                )
                                .line_height(Some(20.0)),
                            )
                            .wrap(),
                        );
                    }
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
                        if button_enabled(
                            ui,
                            a,
                            label(copy, &t.value),
                            ButtonKind::Secondary,
                            None,
                            ready,
                        )
                        .clicked()
                        {
                            crate::clipboard::copy_answered(
                                ui.ctx(),
                                t.value.clone(),
                                copy_id(&t.value),
                            );
                        }
                        if t.uri
                            && button_enabled(
                                ui,
                                a,
                                label(tr!("dash: link"), &uri),
                                ButtonKind::Secondary,
                                None,
                                ready,
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
                            request_field(ui, &p, form, rate.as_ref(), amount);
                            let typed = if form.in_currency && rate.as_ref().is_some_and(|r| r.fresh) {
                                &form.fiat
                            } else {
                                &form.amount
                            };
                            if !typed.trim().is_empty() && amount.is_none() {
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
                crate::components::list_card_with(
                    ui,
                    a,
                    width,
                    (tr!("Earlier addresses"), tr!("still receive payments")),
                    // The app's one «Hide amounts», as an icon (FEAT-0007).
                    |ui, a| {
                        crate::components::amount_toggle(ui, a, true);
                    },
                    |ui, a| {
                        ui.add_space(-2.0);
                        for old in past.iter().take(PAST_SHOWN) {
                            let shown = crate::format::balance(old.received);
                            // A click copies it: "Copied" stands in the line under it for a moment.
                            let detail =
                                if crate::clipboard::just_copied(ui.ctx(), copy_id(&old.address)) {
                                    tr!("Copied").to_string()
                                } else {
                                    format!(
                                        "{} · {}",
                                        crate::text::plural(
                                            old.payments,
                                            tr!("payment"),
                                            tr!("payments")
                                        ),
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
                                    alert: None,
                                    quiet: false,
 core_chip: false,
                                    note: None,
                                    fiat: None,
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
        });
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
/// A transfer between our own addresses counts for the address it came to (BUG-33).
pub(crate) fn past_addresses(history: &[Activity], current: &str) -> Vec<PastAddress> {
    let mut out: Vec<PastAddress> = Vec::new();
    for act in history {
        let (Chain::Core, Some(address)) = (act.chain, &act.address) else {
            continue;
        };
        let amount = match act.kind {
            ActivityKind::Received { .. } => act.amount,
            ActivityKind::SelfTransfer { moved } => moved,
            _ => continue,
        };
        if address == current || amount <= 0 {
            continue;
        }
        match out.iter_mut().find(|p| &p.address == address) {
            Some(p) => {
                p.received += amount;
                p.payments += 1;
                p.last = p.last.max(act.at);
            }
            None => out.push(PastAddress {
                address: address.clone(),
                received: amount,
                payments: 1,
                last: act.at,
            }),
        }
    }
    out.sort_by_key(|p| std::cmp::Reverse(p.last));
    out
}

/// Where the QR card stands while there is no address yet: the same size, empty (BUG-74 №1).
pub(crate) fn qr_waiting(ui: &mut Ui, p: &crate::theme::Palette) {
    let side = 261.0 + 40.0;
    let (rect, _) = ui.allocate_exact_size(vec2(side, side), Sense::hover());
    ui.painter().rect(
        rect,
        CornerRadius::same(16),
        p.surface,
        Stroke::new(1.0, p.line),
        StrokeKind::Inside,
    );
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

    /// FEAT-0008 AC-T10: 25 USD at 30.78 ask for 0.81221572 DASH; the link carries DASH only.
    #[test]
    fn a_request_in_the_currency_links_the_dash_it_buys() {
        crate::format::set_display(crate::format::Display {
            style: crate::format::NumberStyle::CommaDot,
            ..Default::default()
        });
        let rate = crate::fiat_lines::Rate {
            price: 3_078_000_000,
            code: "USD".into(),
            at: 0,
            fresh: true,
        };
        let form = super::ReceiveForm {
            in_currency: true,
            fiat: "25".into(),
            ..Default::default()
        };
        assert_eq!(
            payment_uri("yX", form.requested(Some(&rate))),
            "dash:yX?amount=0.81221572"
        );
        // An old rate: the currency is not used; the DASH field (empty) is.
        let old = crate::fiat_lines::Rate {
            fresh: false,
            ..rate
        };
        assert_eq!(payment_uri("yX", form.requested(Some(&old))), "dash:yX");
        crate::format::set_display(Default::default());
    }

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
            layers: evolite_api::layer::CORE,
            flow: evolite_api::Flow::Out,
            key: String::new(),
            detail: None,
            note: None,
            rate: None,
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

    /// BUG-33: 0.001 sent to our own address A — A is an earlier address with that payment.
    #[test]
    fn an_address_paid_from_the_wallet_itself_is_an_earlier_address() {
        let history = [Activity {
            id: 1,
            kind: ActivityKind::SelfTransfer { moved: 100_000 },
            chain: Chain::Core,
            at: 80,
            amount: -226,
            proof: Proof::InstantLock,
            txid: None,
            fee: Some(226),
            block: None,
            address: Some("yA".into()),
            layers: evolite_api::layer::CORE,
            flow: evolite_api::Flow::Out,
            key: String::new(),
            detail: None,
            note: None,
            rate: None,
        }];
        assert_eq!(
            past_addresses(&history, "yNow"),
            [PastAddress {
                address: "yA".into(),
                received: 100_000,
                payments: 1,
                last: 80
            }]
        );
        // A shielded transfer to our own private address names no Core address.
        let pool = Activity {
            kind: ActivityKind::SelfTransfer { moved: 0 },
            address: None,
            layers: evolite_api::layer::CORE,
            flow: evolite_api::Flow::Out,
            ..history[0].clone()
        };
        assert!(past_addresses(&[pool], "yNow").is_empty());
    }
}
