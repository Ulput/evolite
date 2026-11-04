//! 05 · History: every operation, filtered and searched, grouped by day.

use super::txt;
use crate::components::{
    Assets, ButtonKind, TxRow, button, button_enabled, dialog_with_header, field_label, kv_row,
    notice, page_header, paint_icon_box, pill, section_label, status_chip, tx_row_clickable,
};
use crate::context::Ctx;
use crate::csv_export;
use crate::text::{self};
use crate::theme::{Weight, mono, sans};
use chrono::{DateTime, Local};
use egui::{Align, Layout, Margin, Stroke, Ui};
use evolite_api::{Activity, ActivityKind, Chain, Duffs, SavedAddress, Source};

/// Which operations a filter on the History screen keeps.
fn matches_filter(filter: usize, act: &Activity) -> bool {
    match filter {
        1 => act.chain == Chain::Core,
        2 => act.chain != Chain::Core,
        3 => matches!(
            act.kind,
            ActivityKind::Shielded { .. } | ActivityKind::PoolReceived | ActivityKind::PoolSent
        ),
        4 => matches!(
            act.kind,
            ActivityKind::TopUp { .. }
                | ActivityKind::AppSignIn { .. }
                | ActivityKind::CreditTransfer { .. }
                | ActivityKind::Withdrawal { .. }
                | ActivityKind::ContactRequest { .. }
                | ActivityKind::ProfileSaved
                | ActivityKind::NameRegistered { .. }
        ),
        _ => true,
    }
}

/// Search (`q` lowercased) by what the row shows — the title, the amount — and what it stands
/// for: the whole address or name, and the txid (an operation's id is its txid's first 16 hex
/// digits, so a pasted txid finds it). Spaces between thousands may be typed as plain spaces.
fn matches_query(act: &Activity, q: &str, book: &[SavedAddress]) -> bool {
    if q.is_empty() {
        return true;
    }
    let counterparty = match &act.kind {
        ActivityKind::Received { from: Some(who) }
        | ActivityKind::Shielded { from: Some(who) }
        | ActivityKind::Sent { to: who }
        | ActivityKind::CreditTransfer { to: who }
        | ActivityKind::Withdrawal { to: who }
        | ActivityKind::ContactRequest { to: who }
        | ActivityKind::TopUp { identity: who } => who.as_str(),
        _ => "",
    };
    // The payment's Core address too: a payment in names no one, only our own address it came
    // to, and a payment to a contact shows the @name with the address beside it. And the whole
    // txid, as the card shows it.
    let shown = format!(
        "{} {} {} {} {}",
        text::activity_title_in(act, book),
        crate::format::number(act.amount, true),
        counterparty,
        act.address.as_deref().unwrap_or_default(),
        act.txid.as_deref().unwrap_or_default()
    )
    .replace('\u{a0}', " ")
    .to_lowercase();
    let txid = q.len() >= 8 && q.chars().all(|c| c.is_ascii_hexdigit()) && {
        let id = format!("{:016x}", act.id);
        id.starts_with(&q[..q.len().min(16)])
    };
    shown.contains(q) || txid
}

/// 05 · History: filter chips, search, operations grouped by day.
pub fn activity(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, filter: &mut usize, query: &mut String) {
    let p = a.p;
    let wallet = cx.wallet();
    ui.spacing_mut().item_spacing.y = 18.0;
    // The last export's answer: the file written (and how many lines), or why not.
    let saved_id = egui::Id::new("csv-export");
    let mut export = false;
    page_header(ui, a, tr!("History"), None, |ui, a| {
        let resp = button_enabled(
            ui,
            a,
            tr!("Export CSV"),
            ButtonKind::Secondary,
            None,
            !crate::is_demo(cx.snap),
        );
        export = resp.clicked();
        #[cfg(demo_core)]
        resp.on_disabled_hover_text(
            crate::i18n::t(tr!(
                "Nothing to export in the demo: the export takes the real history"
            ))
            .into_owned(),
        );
    });
    if export {
        let csv = csv_export::taxbit(&wallet.activity);
        let lines = csv.lines().count().saturating_sub(1);
        let day = Local::now().format("%Y-%m-%d").to_string();
        let saved = csv_export::downloads()
            .ok_or_else(|| tr!("the «Downloads» folder was not found").to_string())
            .and_then(|dir| csv_export::save(&dir, &csv, &day).map_err(|e| e.to_string()))
            .map(|path| (path, lines));
        ui.data_mut(|d| d.insert_temp(saved_id, saved));
    }
    let saved: Option<Result<(std::path::PathBuf, usize), String>> =
        ui.data(|d| d.get_temp(saved_id));
    match saved {
        Some(Ok((path, lines))) => {
            notice(
                ui,
                a,
                "confirmed",
                p.accent,
                &format!(
                    tr!("Saved {} in DashPay's (TaxBit) format: {}"),
                    text::plural(lines as u32, tr!("operation"), tr!("operations")),
                    path.display()
                ),
            );
            if button(ui, a, tr!("Show in folder"), ButtonKind::Ghost, None).clicked() {
                // Explorer and Finder select the file; Linux file managers open its folder.
                let mut command = if cfg!(windows) {
                    let mut c = std::process::Command::new("explorer");
                    c.arg(format!("/select,{}", path.display()));
                    c
                } else if cfg!(target_os = "macos") {
                    let mut c = std::process::Command::new("open");
                    c.arg("-R").arg(path);
                    c
                } else {
                    let mut c = std::process::Command::new("xdg-open");
                    c.arg(path.parent().unwrap_or(path.as_path()));
                    c
                };
                let _ = command.spawn();
            }
        }
        Some(Err(why)) => notice(
            ui,
            a,
            "warning",
            p.danger,
            &format!(tr!("The file was not saved: {why}."), why = why),
        ),
        None => {}
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        for (i, label) in [
            tr!("All"),
            "Core",
            "Platform",
            tr!("Private pool"),
            "Identity",
        ]
        .iter()
        .enumerate()
        {
            if pill(ui, &p, label, *filter == i).clicked() {
                *filter = i;
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            egui::Frame::new()
                .fill(p.raised)
                .stroke(Stroke::new(1.0, p.field_line))
                .corner_radius(10)
                .inner_margin(Margin::symmetric(14, 11))
                .show(ui, |ui| {
                    crate::rtl::text_edit(
                        ui,
                        egui::TextEdit::singleline(query)
                            .hint_text(txt(
                                tr!("Search by address, name, amount"),
                                14.0,
                                Weight::Regular,
                                p.muted,
                            ))
                            .font(sans(14.0, Weight::Regular))
                            .frame(egui::Frame::NONE)
                            .desired_width(272.0),
                    );
                });
        });
    });
    let q = query.trim().to_lowercase();
    let shown: Vec<_> = wallet
        .activity
        .iter()
        .filter(|act| matches_filter(*filter, act))
        .filter(|act| matches_query(act, &q, &wallet.address_book))
        .collect();
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            if shown.is_empty() {
                ui.add_space(40.0);
                // Nothing at all is not "nothing found".
                let empty = if wallet.activity.is_empty() {
                    tr!("No operations yet. Receive DASH on the «Receive» screen.")
                } else {
                    tr!("Nothing found")
                };
                ui.vertical_centered(|ui| ui.label(txt(empty, 14.0, Weight::Regular, p.muted)));
            }
            let mut day = String::new();
            for act in shown {
                let this_day = text::day(act.at, cx.now);
                if this_day != day {
                    day = this_day;
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.add_space(16.0);
                        section_label(ui, &p, &day);
                    });
                    ui.add_space(4.0);
                }
                let detail = format!(
                    "{} · {}",
                    text::chain(act.chain),
                    text::time_of_day(act.at, cx.now)
                );
                let row = tx_row_clickable(
                    ui,
                    a,
                    &TxRow {
                        icon: text::activity_icon(act),
                        title: &text::activity_title_in(act, &wallet.address_book),
                        detail: &detail,
                        proof: Some(act.proof),
                        amount: &text::signed_amount(act.amount),
                    },
                );
                if row.clicked() {
                    open_card(ui.ctx(), act.id);
                }
            }
        });
    card_window(ui, a, cx);
}

/// The open operation's card, if one is open: History and Overview's latest operations show
/// the same one.
pub fn card_window(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx) {
    let wallet = cx.wallet();
    let open = ui.data(|d| d.get_temp::<u64>(card_id()));
    if let Some(act) = open.and_then(|id| wallet.activity.iter().find(|a| a.id == id)) {
        let (closed, again) = card(ui, a, act, cx.now, &wallet.address_book);
        if closed {
            ui.data_mut(|d| d.remove::<u64>(card_id()));
        }
        // "Send again": the Send screen with the same recipient and amount.
        if let Some((to, amount)) = again {
            cx.pay_again(Source::Core, to, amount);
        }
    }
}

/// Where the open card's operation is kept between frames.
fn card_id() -> egui::Id {
    egui::Id::new("activity-card")
}

/// Opens the card of operation `id` (a row clicked; `--screen activity-card`).
pub fn open_card(ctx: &egui::Context, id: u64) {
    ctx.data_mut(|d| d.insert_temp(card_id(), id));
}

/// The block explorer's page of a transaction, on the network of this run.
fn explorer(txid: &str) -> String {
    match evolite_core::network::current() {
        evolite_api::Network::Testnet => {
            format!("https://insight.testnet.networks.dash.org/insight/tx/{txid}")
        }
        evolite_api::Network::Mainnet => format!("https://insight.dash.org/insight/tx/{txid}"),
    }
}

/// Who an operation was with, as its card says it: the label, the value (the whole address
/// when the wallet knows it) and, for a contact paid at an address of its key, a line naming it.
fn counterparty(act: &Activity) -> Option<(&'static str, String, Option<String>)> {
    match &act.kind {
        ActivityKind::Received { from: Some(who) } => Some((tr!("Sender"), who.clone(), None)),
        ActivityKind::Sent { to } => match &act.address {
            // A contact (by name or short id) is paid at a new address of its key.
            Some(address) if to.starts_with('@') || to.contains('…') => Some((
                tr!("To"),
                address.clone(),
                Some(format!(
                    tr!("{to} · a new address from the contact's key"),
                    to = to
                )),
            )),
            Some(address) => Some((tr!("To"), address.clone(), None)),
            None if !to.is_empty() => Some((tr!("To"), to.clone(), None)),
            None => None,
        },
        ActivityKind::CreditTransfer { to }
        | ActivityKind::Withdrawal { to }
        | ActivityKind::ContactRequest { to } => Some((tr!("To"), to.clone(), None)),
        // An asset lock: the identity it registered or tops up, when the core knows it.
        ActivityKind::TopUp { identity } if !identity.is_empty() => {
            Some((tr!("To"), identity.clone(), None))
        }
        // The lock itself does not name it: one of this wallet's identities or its Platform
        // address, and nothing else.
        ActivityKind::TopUp { .. } => Some((
            tr!("To"),
            "Dash Platform".into(),
            Some(tr!("An identity or the Platform address of this wallet").into()),
        )),
        _ => None,
    }
}

/// A label over a value in mono that wraps (an address, a txid), with an optional muted line.
fn mono_block(
    ui: &mut Ui,
    p: &crate::theme::Palette,
    label: &str,
    value: &str,
    note: Option<&str>,
) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        field_label(ui, p, label);
        ui.add(
            egui::Label::new(
                egui::RichText::new(crate::i18n::t(value))
                    .font(mono(12.0, Weight::Regular))
                    .color(p.text)
                    .line_height(Some(18.0)),
            )
            .wrap(),
        );
        if let Some(note) = note {
            ui.label(txt(note, 12.0, Weight::Regular, p.muted));
        }
    });
}

/// 24 · Operation details: what and where, how much and its status, when, the fee and the block,
/// with whom, and the txid to copy or look up; "Send again" for a payment out. Whether it
/// closed, and the payment to make again when asked.
fn card(
    ui: &mut Ui,
    a: &mut Assets,
    act: &Activity,
    now: DateTime<Local>,
    book: &[SavedAddress],
) -> (bool, Option<(String, Duffs)>) {
    let repeat = again(act);
    let mut paid_again = None;
    let closed = dialog_with_header(
        ui,
        a,
        "activity-card",
        464.0,
        |ui, a| {
            let p = a.p;
            let (boxr, _) = ui.allocate_exact_size(egui::vec2(48.0, 48.0), egui::Sense::hover());
            paint_icon_box(ui, a, boxr, 12, text::activity_icon(act), 22.0, p.ink);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.add_space(4.0);
                ui.label(txt(
                    &text::activity_title_in(act, book),
                    20.0,
                    Weight::Bold,
                    p.text,
                ));
                ui.label(txt(
                    &format!("{} · {}", text::chain(act.chain), text::when(act.at, now)),
                    14.0,
                    Weight::Medium,
                    p.muted,
                ));
            });
        },
        |ui, a| {
            let p = a.p;
            ui.horizontal(|ui| {
                crate::components::amount_label(
                    ui,
                    &text::signed_amount(act.amount),
                    mono(28.0, Weight::Medium),
                    p.text,
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    status_chip(ui, a, act.proof);
                });
            });
            let value = mono(13.0, Weight::Medium);
            kv_row(
                ui,
                &p,
                tr!("When"),
                &text::date_time(act.at, now),
                value.clone(),
            );
            kv_row(ui, &p, tr!("Where"), text::chain(act.chain), value.clone());
            if let Some(fee) = act.fee {
                kv_row(
                    ui,
                    &p,
                    tr!("Fee"),
                    // Already inside the amount above: shown as a size, not a second debit
                    //.
                    &crate::format::amount(fee),
                    value.clone(),
                );
            }
            if let Some(block) = act.block {
                kv_row(
                    ui,
                    &p,
                    tr!("Block"),
                    &text::group_thousands(block as i64),
                    value,
                );
            }
            if let Some((label, who, note)) = counterparty(act) {
                // An address the person named says so.
                let note = note.or_else(|| {
                    text::book_label(act, book)
                        .map(|l| format!(tr!("«{l}» · from the address book"), l = l))
                });
                mono_block(ui, &p, label, &who, note.as_deref());
            }
            let mut close = false;
            if let Some(txid) = &act.txid {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    mono_block(ui, &p, "txid", txid, None);
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        // Two outlined buttons with a gap: a ghost one beside an outlined one read as
                        // one control.
                        ui.spacing_mut().item_spacing.x = 12.0;
                        if button(ui, a, tr!("Copy txid"), ButtonKind::Secondary, None).clicked() {
                            crate::clipboard::copy(ui.ctx(), txid.clone());
                        }
                        if button(
                            ui,
                            a,
                            tr!("Open in the explorer"),
                            ButtonKind::Secondary,
                            None,
                        )
                        .on_hover_text(crate::i18n::t(&explorer(txid)).into_owned())
                        .clicked()
                        {
                            // egui's open_url does nothing without eframe's `links`.
                            crate::links::open(&explorer(txid));
                        }
                    });
                });
                notice(
                    ui,
                    a,
                    "shield",
                    p.accent,
                    tr!(
                        "The explorer opens only when you click. The explorer's site learns this computer's address and which operation you look at, so evolite opens it only by the button. You can simply copy the txid."
                    ),
                );
            }
            ui.horizontal(|ui| {
                if let Some(repeat) = &repeat
                    && button(ui, a, tr!("Send again"), ButtonKind::Secondary, None)
                        .on_hover_text(
                            crate::i18n::t(tr!("The same amount to the same recipient — we show everything again before signing"),)
                        .into_owned())
                        .clicked()
                {
                    paid_again = Some(repeat.clone());
                    close = true;
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if button(ui, a, tr!("Close"), ButtonKind::Ghost, None).clicked() {
                        close = true;
                    }
                });
            });
            close
        },
    );
    (closed, paid_again)
}

/// What "Send again" pays: a Core payment out, to its @name or address, the amount
/// without the fee. Not for a contact without a name (its short id cannot be paid) or with
/// nothing known of the amount.
fn again(act: &Activity) -> Option<(String, Duffs)> {
    let ActivityKind::Sent { to } = &act.kind else {
        return None;
    };
    let recipient = if to.starts_with('@') {
        to.clone()
    } else {
        act.address.clone().unwrap_or_else(|| to.clone())
    };
    let amount = -act.amount - act.fee.unwrap_or(0);
    (act.chain == Chain::Core && !recipient.is_empty() && !recipient.contains('…') && amount > 0)
        .then_some((recipient, amount))
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolite_api::Proof;

    fn sent(to: &str, amount: i64) -> Activity {
        Activity {
            id: 0x536e_90c0_1af4_19f4,
            kind: ActivityKind::Sent { to: to.into() },
            chain: Chain::Core,
            at: 0,
            amount,
            proof: Proof::ChainLock,
            txid: None,
            fee: None,
            block: None,
            address: None,
        }
    }

    #[test]
    fn finds_by_address_name_amount_and_txid() {
        let act = sent("yTFDD75uEcLgEGN8QYPKKvVE69jqf6vMw2", -123_400_000_000);
        // The whole address, though the row shows it short.
        assert!(matches_query(&act, "vve69jqf6", &[]));
        assert!(matches_query(&act, "sent", &[]));
        // Thousands as the interface language writes them.
        assert!(matches_query(&act, "1,234", &[]));
        // A pasted txid, or its start.
        assert!(matches_query(
            &act,
            "536e90c01af419f42cd2b9430f318b26031c9532c86013a409d529b02fc802c4",
            &[]
        ));
        // By the name the person gave the address.
        let book = [SavedAddress {
            address: "yTFDD75uEcLgEGN8QYPKKvVE69jqf6vMw2".into(),
            label: "Exchange".into(),
        }];
        assert!(matches_query(&act, tr!("exchange"), &book));
        assert!(!matches_query(&act, tr!("exchange"), &[]));
        assert!(matches_query(&act, "536e90c0", &[]));
        assert!(!matches_query(&act, "deadbeef", &[]));
        assert!(!matches_query(&act, "@bob", &[]));
        assert!(matches_query(&sent("@bob", -1), "@bob", &[]));
    }

    /// An address finds the payments it is in: our own address of a payment in, the address
    /// behind a contact's @name, a payment out whose recipient has no name.
    #[test]
    fn finds_payments_by_their_core_address() {
        let ours = "yMXp7gU4uQ4mVQ4V1V3H6cPCXxWfNrsW3d";
        let theirs = "yTFDD75uEcLgEGN8QYPKKvVE69jqf6vMw2";
        // The screen lowercases what is typed.
        let q = |s: &str| s.to_lowercase();
        let mut received = sent("", 50_000_000);
        received.kind = ActivityKind::Received { from: None };
        received.address = Some(ours.into());
        assert!(matches_query(&received, &q(ours), &[]));
        assert!(
            matches_query(&received, &q("4uQ4mVQ4V1"), &[]),
            "a part of it"
        );
        assert!(!matches_query(&received, &q(theirs), &[]));
        let mut to_contact = sent("@bob", -1);
        to_contact.address = Some(theirs.into());
        assert!(matches_query(&to_contact, &q(theirs), &[]));
        let mut unnamed = sent("", -1);
        unnamed.address = Some(theirs.into());
        assert!(matches_query(&unnamed, &q(theirs), &[]));
        let mut with_txid = sent("", -1);
        with_txid.txid = Some("ab".repeat(32));
        assert!(matches_query(&with_txid, &"ab".repeat(32), &[]));
    }
}

#[cfg(test)]
mod card_tests {
    use super::*;
    use evolite_api::Proof;

    fn sent(to: &str, address: Option<&str>) -> Activity {
        Activity {
            id: 1,
            kind: ActivityKind::Sent { to: to.into() },
            chain: Chain::Core,
            at: 0,
            amount: -1,
            proof: Proof::ChainLock,
            txid: None,
            fee: None,
            block: None,
            address: address.map(Into::into),
        }
    }

    #[test]
    fn says_whom_with_the_address_and_the_contact() {
        // A contact: its new address, and its name under it.
        assert_eq!(
            counterparty(&sent("@bob", Some("yLcBz"))),
            Some((
                tr!("To"),
                "yLcBz".to_string(),
                Some("@bob · a new address from the contact's key".to_string())
            ))
        );
        // An address alone, or a name with no address known.
        assert_eq!(
            counterparty(&sent("yLcBz", Some("yLcBz"))),
            Some((tr!("To"), "yLcBz".to_string(), None))
        );
        assert_eq!(
            counterparty(&sent("@bob", None)),
            Some((tr!("To"), "@bob".to_string(), None))
        );
        assert_eq!(counterparty(&sent("", None)), None);
    }
}

#[cfg(test)]
mod again_tests {
    use super::*;
    use evolite_api::Proof;

    fn out(to: &str, address: Option<&str>, amount: Duffs, fee: Option<Duffs>) -> Activity {
        Activity {
            id: 1,
            kind: ActivityKind::Sent { to: to.into() },
            chain: Chain::Core,
            at: 0,
            amount,
            proof: Proof::ChainLock,
            txid: None,
            fee,
            block: None,
            address: address.map(Into::into),
        }
    }

    #[test]
    fn pays_the_same_recipient_the_same_amount_again() {
        // To an address: the address, the amount without the fee.
        assert_eq!(
            again(&out("yXk3", Some("yXk3"), -100_000_226, Some(226))),
            Some(("yXk3".to_string(), 100_000_000))
        );
        // To a contact: by its name, so it goes to the next address of its key.
        assert_eq!(
            again(&out("@bob", Some("yLc"), -10_226, Some(226))),
            Some(("@bob".to_string(), 10_000))
        );
        // A contact without a name, or nothing that went out: nothing to repeat.
        assert_eq!(again(&out("6G7V…Qck", None, -10_000, None)), None);
        let mut received = out("yX", None, 5, None);
        received.kind = ActivityKind::Received { from: None };
        assert_eq!(again(&received), None);
    }

    #[test]
    fn offers_saved_addresses_while_typing() {
        let book = [
            SavedAddress {
                address: "yAAA".into(),
                label: "Rent".into(),
            },
            SavedAddress {
                address: "yBBB".into(),
                label: "Exchange".into(),
            },
        ];
        // `book_matches` lives with the Send screen.
        assert_eq!(
            crate::text::book_label(&out("yBBB", None, -1, None), &book),
            Some("Exchange")
        );
        assert_eq!(
            crate::text::activity_title_in(&out("yBBB", Some("yBBB"), -1, None), &book),
            "Sent · Exchange"
        );
    }
}
