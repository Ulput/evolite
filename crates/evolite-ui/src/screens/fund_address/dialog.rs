//! «Top up the Platform address» (frame 22) and «Check the top-up» (22b): one dialog, the form
//! counting what the address gets and what leaves Core, then the review with the password and
//! «Sign and top up» (BUG-42: the password on the review, not in the form). Long dialogs keep
//! their header and footer in the window (BUG-122).

use super::{FundAddressState, FundForm};
use crate::components::{
    Assets, ButtonKind, CardAmount, Part, amount_field, button, button_enabled, dialog_with_footer,
    divider, field_label, kv_row, notice,
};
use crate::screens::txt;
use crate::theme::{Weight, mono};
use egui::Ui;
use evolite_api::address_fund::{FundPlan, FundRefusal, fund_plan};
use evolite_api::{Command, Duffs, Spend, Wallet};

fn dash(d: Duffs) -> String {
    crate::format::amount(d)
}

/// What Core can pay with: its balance less what no payment can sign for yet (BUG-142).
fn spendable(wallet: &Wallet) -> Duffs {
    wallet
        .balances
        .core
        .saturating_sub(wallet.fees.core_unsignable)
        .max(0)
}

/// The plan for what is typed, as the core will make it (AC-T2).
pub(super) fn plan_of(form: &FundForm, wallet: &Wallet) -> Option<Result<FundPlan, FundRefusal>> {
    let amount = crate::format::parse_amount(&form.amount).filter(|d| *d > 0)?;
    Some(fund_plan(amount, false, spendable(wallet), &wallet.fees))
}

/// The address, shortened for a line of words: «tdash1qqmz…aznp».
fn short(address: &str) -> String {
    let chars: Vec<char> = address.chars().collect();
    if chars.len() <= 16 {
        return address.to_owned();
    }
    let head: String = chars[..10].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}…{tail}")
}

/// The dialog while `state.form` is open. Sends `FundAddress` from the review.
pub fn fund_dialog(
    ui: &mut Ui,
    a: &mut Assets,
    state: &mut FundAddressState,
    wallet: &Wallet,
    now: chrono::DateTime<chrono::Local>,
    asked: &mut Vec<Command>,
) {
    let Some(form) = &mut state.form else {
        return;
    };
    crate::format::fill_suggested(&mut form.amount, &mut form.suggested);
    let review = form.review;
    let plan = plan_of(form, wallet);
    let core_amount = CardAmount::core(&wallet.balances);
    let mut send: Option<Duffs> = None;
    let mut back = false;
    let mut to_review = false;
    let title = if review {
        tr!("Check the top-up")
    } else {
        tr!("Top up the Platform address")
    };
    let close = dialog_with_footer(
        ui,
        a,
        "fund-address",
        "top-up",
        title,
        464.0,
        |ui, a, part| {
            let p = a.p;
            let value = mono(13.0, Weight::Medium);
            if part == Part::Footer {
                let mut close = false;
                if review {
                    // The password of the review (22b), always asked when the setting asks.
                    let ask = wallet.password_each_send;
                    let entered = state.password.field(ui, &p, "fund-address-password", ask);
                    let ready = matches!(plan, Some(Ok(_))) && state.password.ready(ask);
                    crate::components::button_row_spend(ui, |ui| {
                        if button(ui, a, tr!("Back"), ButtonKind::Ghost, None).clicked() {
                            back = true;
                        }
                        let sign = button_enabled(
                            ui,
                            a,
                            tr!("Sign and top up"),
                            ButtonKind::Primary,
                            None,
                            ready,
                        );
                        if (sign.clicked() || (entered && ready))
                            && let Some(Ok(plan)) = plan
                        {
                            send = Some(plan.arrives_at_least);
                        }
                    });
                } else {
                    crate::components::button_row_spend(ui, |ui| {
                        if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                            close = true;
                        }
                        let ready = matches!(plan, Some(Ok(_)));
                        if button_enabled(ui, a, tr!("Review"), ButtonKind::Primary, None, ready)
                            .clicked()
                        {
                            to_review = true;
                        }
                    });
                }
                return close;
            }
            if review {
                kv_row(
                    ui,
                    &p,
                    tr!("From"),
                    &format!("Core · {}", dash(wallet.balances.core)),
                    value.clone(),
                );
                field_label(ui, &p, tr!("To · this wallet's Platform address"));
                // Whole and solid, as every address on a review (FEAT-0035), in the monospace
                // font: Inter draws «x» between digits as «×» (the designer).
                ui.label(
                    egui::RichText::new(&wallet.platform_address)
                        .font(mono(13.0, Weight::Medium))
                        .color(p.text),
                );
            } else {
                ui.add(
                    crate::components::para(
                        &format!(
                            tr!("DASH from Core go to this wallet's Platform address: {}. From it you can pay to Platform addresses, top up identities and withdraw to Core."),
                            crate::i18n::own(&short(&wallet.platform_address))
                        ),
                        13.0,
                        Weight::Regular,
                        p.muted,
                    )
                    .line_height(20.0),
                );
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    field_label(ui, &p, tr!("How much to credit"));
                    let field = amount_field(ui, &p, "fund-address-amount", &mut form.amount, true);
                    if field.all {
                        let most = match fund_plan(0, true, spendable(wallet), &wallet.fees) {
                            Ok(plan) => plan.arrives_at_least,
                            Err(_) => 0,
                        };
                        form.amount = crate::format::input_text(most.max(0));
                    }
                });
                let (line, color) = match plan {
                    Some(Err(FundRefusal::NotEnough { .. })) => {
                        (tr!("More than there is on Core").to_owned(), p.danger)
                    }
                    Some(Err(FundRefusal::BelowLeast { least })) => (
                        format!(tr!("At least {} to the address"), dash(least)),
                        p.danger,
                    ),
                    _ => (
                        format!(
                            tr!("At least {} to the address"),
                            dash(wallet.fees.address_fund_least())
                        ),
                        p.muted,
                    ),
                };
                ui.label(txt(&line, 12.0, Weight::Regular, color));
                crate::components::kv_row_eye_in(
                    ui,
                    a,
                    tr!("From the Core wallet"),
                    &crate::text::core_amount(&wallet.balances, core_amount, now),
                    value.clone(),
                    core_amount.color(&p),
                );
            }
            let plan_now = plan.and_then(Result::ok);
            if review && let Some(plan) = plan_now {
                kv_row(
                    ui,
                    &p,
                    tr!("Arrives at the address, at least"),
                    &dash(plan.arrives_at_least),
                    value.clone(),
                );
            }
            kv_row(
                ui,
                &p,
                tr!("Processing on Platform"),
                &format!("≈ {}", dash(wallet.fees.address_fund_on_top())),
                value.clone(),
            );
            ui.label(txt(
                tr!("Whatever is not needed stays on the address."),
                12.0,
                Weight::Regular,
                p.muted,
            ));
            let core_fee = plan_now.map_or(wallet.fees.core_payment, |plan| plan.core_fee);
            kv_row(
                ui,
                &p,
                tr!("Core fee"),
                &format!("≈ {}", dash(core_fee)),
                value.clone(),
            );
            divider(ui, &p);
            kv_row(
                ui,
                &p,
                tr!("Total from Core"),
                &plan_now.map_or_else(|| "—".to_owned(), |plan| format!("≈ {}", dash(plan.total))),
                value,
            );
            if review {
                ui.label(txt(
                    tr!("The DASH are locked in Core for Platform. Once signed, this cannot be undone; if the connection drops, evolite continues from the same place."),
                    12.0,
                    Weight::Regular,
                    p.muted,
                ));
                notice(
                    ui,
                    a,
                    "eye",
                    p.muted,
                    tr!("The amount on the Platform address will be visible to everyone."),
                );
            } else {
                notice(
                    ui,
                    a,
                    "eye",
                    p.muted,
                    tr!(
                        "The amount on the Platform address will be visible to everyone. Confirmation: InstantSend in seconds, otherwise ChainLock up to ~2.5 min."
                    ),
                );
            }
            false
        },
    );
    if let Some(amount) = send {
        let ask = wallet.password_each_send;
        asked.push(state.password.command(Spend::FundAddress { amount }, ask));
        state.running = Some(amount);
        state.steps_open = true;
        state.result = None;
        state.form = None;
    } else if back {
        if let Some(form) = &mut state.form {
            form.review = false;
        }
        state.password.forget();
    } else if to_review {
        if let Some(form) = &mut state.form {
            form.review = true;
        }
    } else if close {
        state.form = None;
        state.password.forget();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_address_is_shortened_in_words() {
        assert_eq!(
            short("tdash1qqmz8fh2wl5c9u4hyrq3m4n0t7k2vx6s4u6aznp"),
            "tdash1qqmz…aznp"
        );
        assert_eq!(short("tdash1short"), "tdash1short");
    }
}
