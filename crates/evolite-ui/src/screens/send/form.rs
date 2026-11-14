//! The form of Send and its summary, drawn from what one frame knows ([`FormView`]).

use super::*;

/// What one frame of the form knows, worked out once before anything is drawn.
pub(super) struct FormView<'a> {
    pub(super) all: &'a [SourceView],
    pub(super) source: &'a SourceView,
    pub(super) core_as_of: Option<evolite_api::CoreAsOf>,
    pub(super) core_as_of_line: Option<String>,
    pub(super) ask_password: bool,
    /// A refusal has just come: its line under the buttons is brought into view, once.
    pub(super) reveal: bool,
    pub(super) address_problem: Option<String>,
    pub(super) amount: Option<Duffs>,
    pub(super) left: Option<Duffs>,
    pub(super) synced: bool,
    pub(super) problem: Option<AmountProblem>,
    pub(super) unavailable: Option<String>,
    /// From the pool to an @name, which it cannot pay (BUG-112): said once, under the recipient.
    pub(super) pool_to_name: bool,
    /// From Core to the wallet's own Platform address (FEAT-0015 stage 2, frame 14): a funding of
    /// the address, its plate and «Check and top up», not a payment.
    pub(super) fund_own: bool,
    pub(super) resolved_name: Option<ResolvedName>,
    pub(super) to_self: bool,
    pub(super) target_known: bool,
    pub(super) name_problem: Option<(String, bool)>,
    pub(super) checking: bool,
    pub(super) contact_fee: Duffs,
    /// The warning before "Add to contacts": what the network wants on the identity (BUG-107).
    pub(super) contact_reserve: Option<String>,
    pub(super) not_contact: bool,
    pub(super) recipient_label: String,
    pub(super) book: Vec<SavedAddress>,
    pub(super) address_ok: bool,
    pub(super) own: bool,
    pub(super) summary_to: String,
    pub(super) summary_note: Option<String>,
    pub(super) sent_to: String,
    /// The «Platform address» card (FEAT-0015 AC-D15): a source, or, empty, a button that
    /// fills the address from elsewhere; not known yet, neither.
    pub(super) address_card: AddressCard,
    /// The wallet's own Platform address, and the source that fills it while it is empty: the
    /// credits, or the pool without an identity (frame 14a).
    pub(super) own_platform_address: String,
    pub(super) fills_address: Source,
    /// From the Platform address to an @name that is not one of the wallet's identities (R-1а):
    /// said once under the recipient, nothing to pay, the reason under the buttons.
    pub(super) address_foreign_name: bool,
    /// The wallet's fees, for the plan of a payment from chosen coins (FEAT-0013).
    pub(super) fees: evolite_api::Fees,
    /// A payment from chosen coins (FEAT-0013): what it takes, or how much is short.
    pub(super) coin_plan: Option<Result<evolite_api::coin_plan::CoinPlan, Duffs>>,
    pub(super) coins_chosen: usize,
    /// «Fiat rate» on and a price from the core (FEAT-0008); fresh or not.
    pub(super) rate: Option<super::currency::Rate>,
    /// From «Mixed» (FEAT-0030): the core's plan for this amount and recipient, once it gave it.
    pub(super) mixed_quote: Option<Result<evolite_api::MixedQuote, CoreError>>,
    /// The core refuses its excess (AC-A12): the excess and the amount that burns nothing.
    pub(super) mixed_refused: Option<(Duffs, Duffs)>,
    /// «Mixed» has no fully mixed coin yet: its card is dimmed with this (S2).
    pub(super) mixed_hint: Option<String>,
    /// «Mixed» to a name or to Platform: its refusal is about the recipient, said under «To» (S2).
    pub(super) mixed_to_recipient: bool,
    /// BUG-142 AC-142-2: `unavailable` is the amount past what can be spent: the button's reason,
    /// in `danger` like «Not enough for the amount and the fee» (the designer).
    pub(super) unsignable_short: bool,
}

/// What the «Platform address» card of «From» is (FEAT-0015, the customer's change of
/// 2026-11-07).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AddressCard {
    /// It can pay: a source like the others.
    Source,
    /// Too little to spend: dimmed; a click fills it (frame 06, 14a).
    Empty,
    /// Its balance not checked yet: dimmed, a click does nothing (frame 10).
    NotKnown,
}

impl AddressCard {
    pub(super) fn of(w: &Wallet) -> Self {
        use evolite_api::address_plan::{AddressMove, MIN_INPUT};
        if !w.balances.platform_address_read {
            AddressCard::NotKnown
        } else if w
            .platform_inputs
            .iter()
            .all(|credits| *credits < MIN_INPUT + AddressMove::Transfer.fee(1))
        {
            AddressCard::Empty
        } else {
            AddressCard::Source
        }
    }

    /// The tooltip, on hover and on the keyboard's focus.
    fn hint(self) -> Option<&'static str> {
        match self {
            AddressCard::Source => None,
            AddressCard::Empty => Some(tr!(
                "There is no money on the Platform address to spend yet. Click the card to top up the address."
            )),
            AddressCard::NotKnown => Some(tr!(
                "The Platform address balance is not checked yet. You can spend from it after the check, usually a few seconds after the sync."
            )),
        }
    }
}

/// What the form asks of the core and the app once it is drawn.
#[derive(Default)]
pub(super) struct Asked {
    pub(super) commands: Vec<Command>,
    pub(super) open_contacts: bool,
    /// «Check and top up» (FEAT-0015 stage 2, frame 14): the funding's review for this amount.
    pub(super) fund: Option<Duffs>,
}

impl<'a> FormView<'a> {
    pub(super) fn of(
        cx: &mut Ctx,
        form: &mut SendForm,
        all: &'a [SourceView],
        typed: String,
        ask_password: bool,
        reveal: bool,
    ) -> Self {
        // Core not checked in this sync: its last checked balance and when, or «not known yet»
        // (BUG-77); a payment waits for the sync all the same.
        let core_as_of = cx.wallet().balances.core_as_of;
        let core_as_of_line = crate::text::core_as_of(core_as_of, cx.now);
        // A source asked for by its kind; one the list does not have stays where it was.
        if let Some(want) = form.want.take()
            && let Some(i) = all.iter().position(|s| s.source == want)
        {
            form.source = i;
        }
        // A source the list no longer has (on mainnet, «Platform address»): the first.
        if form.source >= all.len() {
            form.source = 0;
        }
        let source = &all[form.source];
        let fees = cx.wallet().fees.clone();
        // Chosen coins are this form's for a payment from Core to an address (R-9).
        if source.source != Source::Core || form.recipient.trim().starts_with('@') {
            form.coins.reset();
        }
        form.follow_source(source);
        form.follow_coins_max(&fees);
        // «Max» may have put a new amount there: the currency shows it too (FEAT-0008).
        let rate = super::currency::Rate::now(cx.now.timestamp());
        form.sync_currency(rate.as_ref());
        // An address for Core is checked as it is typed, so a typo shows before the password.
        // A Platform address is no Core payee (FEAT-0015, frame 15): not asked of the core.
        let platform_typed = typed.to_ascii_lowercase().starts_with("tdash1");
        let typed_address = source.source == Source::Core
            && !typed.is_empty()
            && !typed.starts_with('@')
            && !platform_typed;
        if typed_address && typed != form.previewed {
            cx.send(Command::PreviewPayment { to: typed.clone() });
            form.previewed = typed.clone();
            form.preview = None;
        }
        let address_problem = match &form.preview {
            // Nothing was being sent yet: the reason alone (the catalogs translate it with the full
            // stop after it, BUG-35).
            Some(Err(CoreError::NotAvailable(why))) if typed_address && form.previewed == typed => {
                Some(format!("{}.", why.trim_end_matches('.')))
            }
            Some(Err(e)) if typed_address && form.previewed == typed => Some(send_error(e)),
            _ => None,
        };
        let amount = crate::format::parse_amount(&form.amount).filter(|d| *d > 0);
        // From «Mixed» (FEAT-0030): the core plans each amount and recipient; a name or Platform
        // is refused before it is asked.
        let to_core = !typed.is_empty() && !typed.starts_with('@') && !platform_typed;
        if source.source == Source::Mixed
            && to_core
            && let Some(amount) = amount
            && let Some(ask) = form.mixed.ask(&typed, amount)
        {
            cx.send(ask);
        }
        let mixed_quote = (source.source == Source::Mixed)
            .then(|| form.mixed.quote_for(&typed, amount.unwrap_or(0)).cloned())
            .flatten();
        let mixed_refused = super::mixed::refused(mixed_quote.as_ref());
        let mixed_hint = super::mixed::card_hint(cx.wallet());
        let mixed_to_recipient =
            source.source == Source::Mixed && (typed.starts_with('@') || platform_typed);
        // What the payment takes from «Mixed»: the excess too, which goes to the miners.
        // (A refused plan's too: «Left» as the summary shows it, S3.)
        let excess = match &mixed_quote {
            Some(Ok(q)) => q.excess,
            _ => 0,
        };
        let left = amount
            .filter(|_| source.read)
            .map(|d| crate::format::left_after(source.duffs, d + excess, source.fee));
        // Not before the wallet knows all its coins: the core would refuse, and a balance
        // still syncing is no basis for a payment (nor for saying it is too small).
        let synced = crate::text::coins_known(cx.snap.sync.stage);
        let problem = if form.currency_not_an_amount(rate.as_ref()) {
            Some(AmountProblem::NotAnAmount)
        } else {
            form_problem(&form.amount, source, synced)
        };
        // A source that cannot pay at all says why, not "more than there is".
        let mixed_refusal = (source.source == Source::Mixed)
            .then(|| super::mixed::refusal(&typed, mixed_quote.as_ref(), source.duffs, amount))
            .flatten();
        // FEAT-0015 stage 2 (frame 14): from Core to the wallet's own Platform address is a
        // funding of the address — its plate and «Check and top up» — not «you can't» (BUG-143).
        // Bech32m in capitals is the same address (QA21 r1 P3 1).
        let fund_own = source.source == Source::Core
            && platform_typed
            && typed
                .trim()
                .eq_ignore_ascii_case(cx.wallet().platform_address.trim());
        let unavailable = match source.source {
            Source::Mixed => mixed_refusal.as_deref(),
            Source::Core if fund_own => None,
            Source::Credits if cx.wallet().identity.is_none() => Some(tr!(
                "Only an identity has credits, and the wallet has none yet: make one on the “Identity” screen."
            )),
            Source::Pool if cx.wallet().pool.address.is_none() => Some(tr!(
                "The private pool connects after syncing with Platform — usually a minute."
            )),
            // A private address in a DashPay profile is an open question. The wallet's own
            // Platform address is not «someone else's» (BUG-143): bech32m in capitals is the same
            // address, as in the notice from the Platform address (QA21 r1 P3 1).
            Source::Core
                if platform_typed
                    && typed
                        .trim()
                        .eq_ignore_ascii_case(cx.wallet().platform_address.trim()) =>
            {
                Some(tr!(
                    "This wallet's Platform address: you can't send to it from Core directly"
                ))
            }
            Source::Core if platform_typed => Some(tr!(
                "Someone else's Platform address: you can't send to it from Core directly"
            )),
            Source::Pool if typed.starts_with('@') => Some(tr!(
                "You can't send from the pool to an @name. Enter an evolite private address, a Platform address or a Core address, or choose another source."
            )),
            _ => None,
        }
        .map(str::to_owned);
        // Chosen coins that cannot make it: how much is short, or that the choice is stale.
        let coin_plan = (!form.coins.chosen.is_empty()).then(|| {
            super::coins::plan(
                &form.coins.chosen_coins(),
                amount.unwrap_or(0),
                form.max,
                &fees,
            )
        });
        let unavailable = unavailable.or_else(|| {
            if form.coins.gone(&form.coins.chosen.clone()).is_some() {
                Some(tr!("Check the selection.").to_owned())
            } else if let (Some(Err(short)), Some(_)) = (&coin_plan, amount) {
                Some(format!(
                    "{}. {}",
                    format!(tr!("{} short"), crate::format::amount(*short)),
                    tr!("Tick more coins or lower the amount.")
                ))
            } else {
                None
            }
        });
        let pool_to_name =
            source.source == Source::Pool && pool_route(&typed) == Some(PoolRoute::Name);
        let to = recipient(form);
        let resolved_name = match &to {
            Recipient::Name(n) => Some((*n).clone()),
            _ => None,
        };
        // From the Platform addresses (FEAT-0015): what the recipient makes of it and what it
        // may not be (R-3, R-1а), and a withdrawal that would not reach Core.
        let unavailable = unavailable.or_else(|| {
            (source.source == Source::PlatformAddress)
                .then(|| {
                    address_refusal(
                        cx.wallet(),
                        &typed,
                        resolved_name.as_ref(),
                        amount,
                        source.fee,
                    )
                })
                .flatten()
        });
        // BUG-142 AC-142-2: within the balance, but not within what can be signed for now.
        let mut unsignable_short = false;
        let unavailable = unavailable.or_else(|| {
            let unsignable = fees.core_unsignable;
            let amount = amount.filter(|_| source.source == Source::Core && unsignable > 0)?;
            let spendable = source.duffs - unsignable;
            (amount <= source.duffs && amount.saturating_add(source.fee) > spendable).then(|| {
                unsignable_short = true;
                format!(
                    tr!("Not enough to spend: {amount} of the balance can't be spent yet."),
                    amount = crate::format::balance(unsignable)
                )
            })
        });
        // Credits to the wallet's own identity would only cost the fee.
        let to_self = all[form.source].source == Source::Credits
            && resolved_name.as_ref().is_some_and(|n| {
                cx.wallet()
                    .identity
                    .as_ref()
                    .is_some_and(|i| i.id == n.identity_id)
            });
        let target_known = matches!(to, Recipient::Name(_) | Recipient::Address)
            && !to_self
            && address_problem.is_none();
        let name_problem = match &to {
            Recipient::NameProblem(e) => Some(name_problem(e)),
            _ => None,
        };
        let checking = matches!(to, Recipient::Checking);
        // Core pays a name only as a DashPay contact: to the addresses its answer to our request
        // gave us. Until then the form explains and offers the request instead of an amount.
        let contact_fee = cx.wallet().fees.contact_request;
        let contact_reserve = super::super::contacts::reserve_warning(cx.wallet());
        let not_contact = all[form.source].source == Source::Core
            && resolved_name
                .as_ref()
                .is_some_and(|n| n.contact != Some(ContactState::Established));
        let recipient_label = form.recipient.trim().to_string();
        // The address book: Core addresses the person named, to pick or to keep.
        let book = cx.wallet().address_book.clone();
        let address_ok =
            typed_address && form.previewed == typed && matches!(&form.preview, Some(Ok(_)));
        // The summary names the recipient only once it is one (BUG-39 №5), short (BUG-32).
        let known = match &to {
            Recipient::Name(_) => true,
            Recipient::Address => {
                looks_like_address(&typed) && (source.source != Source::Core || address_ok)
            }
            _ => false,
        };
        let summary_to = summary_recipient(&typed, known);
        // The note, last in the summary (AC-D1); as it was when the frame began.
        let summary_note = form.note_to_send();
        // Nothing to keep in the address book: it is this wallet's own (BUG-39 №9).
        let own = own_address(cx.wallet(), &typed);
        // The kind of the last payment signed, for its answer: "To" may be cleared or new by then.
        let sent_to = form.sent_to.trim().to_string();
        let address_card = AddressCard::of(cx.wallet());
        let own_platform_address = cx.wallet().platform_address.clone();
        let fills_address = if cx.wallet().identity.is_some() {
            Source::Credits
        } else {
            Source::Pool
        };
        let address_foreign_name = source.source == Source::PlatformAddress
            && resolved_name
                .as_ref()
                .is_some_and(|n| !cx.wallet().identities.iter().any(|i| i.id == n.identity_id));
        FormView {
            mixed_quote,
            mixed_refused,
            mixed_hint,
            mixed_to_recipient,
            unsignable_short,
            rate,
            fees,
            coin_plan,
            coins_chosen: form.coins.chosen.len(),
            address_card,
            own_platform_address,
            fills_address,
            address_foreign_name,
            all,
            source,
            core_as_of,
            core_as_of_line,
            ask_password,
            reveal,
            address_problem,
            amount,
            left,
            synced,
            problem,
            unavailable,
            pool_to_name,
            fund_own,
            resolved_name,
            to_self,
            target_known,
            name_problem,
            checking,
            contact_fee,
            contact_reserve,
            not_contact,
            recipient_label,
            book,
            address_ok,
            own,
            summary_to,
            summary_note,
            sent_to,
        }
    }
}

/// The form's card: where from, to whom, how much, and the answer to the last payment.
pub(super) fn form_card(
    ui: &mut Ui,
    a: &mut Assets,
    form: &mut SendForm,
    v: &FormView,
    form_w: f32,
    asked: &mut Asked,
) {
    let p = a.p;
    card(&p, Margin::same(24)).show(ui, |ui| {
        ui.set_width(form_w - 50.0);
        ui.spacing_mut().item_spacing.y = 18.0;
        // «From» and, on its right, the app's one «Hide amounts» (FEAT-0007): the balances
        // of the sources and «Left» hide, the amount typed and the fee do not.
        ui.horizontal(|ui| {
            field_label(ui, &p, tr!("From"));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                crate::components::amount_toggle(ui, a, false);
            });
        });
        source_cards(ui, a, form, v);
        super::coins::coins_row(ui, a, form, v, asked);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            field_label(ui, &p, tr!("To"));
            let scanner = &mut form.scanner;
            if text_field(
                ui,
                &p,
                "send-recipient",
                &mut form.recipient,
                tr!("@name or Dash address"),
                FieldKind::Text,
                |ui| {
                    crate::components::qr_button(ui, a, "send-recipient", scanner);
                },
            )
            .changed()
            {
                form.result = None;
                // A `dash:` link pasted as text is taken as a QR's (FR-6).
                let typed = form.recipient.clone();
                form.take_link(&typed);
            }
            // The line is about the «To» it was said of: typing, a contact chosen or the
            // payment gone take it away.
            if form.recipient != form.qr_about {
                form.qr = None;
            }
            if let Some(note) = &form.qr {
                crate::components::qr_note(ui, a, note);
            }
        });
        recipient_state(ui, a, form, v, asked);
        if v.source.source == Source::Core
            && !v.fund_own
            && v.recipient_label.to_ascii_lowercase().starts_with("tdash1")
        {
            platform_from_core(ui, a, form, v);
        }
        if let (true, Some(name)) = (v.not_contact, &v.resolved_name) {
            contact_first(ui, a, form, v, name, asked);
            return;
        }
        amount_row(ui, a, form, v);
        // A funding of the own address carries no note, and its plate says how it is confirmed
        // (frame 14).
        if !v.fund_own {
            note_field(ui, a, form);
        }
        // A funding of the own address says its fees in its own line (frame 14).
        if !v.fund_own {
            kv_row(
                ui,
                &p,
                fee_label(v),
                &fee_value(v),
                mono(13.0, Weight::Regular),
            );
        }
        if !v.fund_own {
            route_notice(ui, a, v);
        }
        send_row(ui, a, form, v, asked);
        payment_answer(ui, a, form, v);
    });
}

/// The sources to pay from, as cards in one row.
fn source_cards(ui: &mut Ui, a: &mut Assets, form: &mut SendForm, v: &FormView) {
    let p = a.p;
    let all = v.all;
    let core_as_of = v.core_as_of;
    let core_as_of_line = &v.core_as_of_line;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            // Equal columns in one row (the designer's summary №25), a long Core balance
            // too (BUG-67); every card whole inside the panel, its words with the padding
            // (BUG-81): as many columns as hold the widest words, without icons if need be.
            let details: Vec<String> = all
                .iter()
                .map(|s| source_line(s, core_as_of, core_as_of_line.as_deref()))
                .collect();
            let widths: Vec<CardWidths> = all
                .iter()
                .zip(&details)
                .map(|(s, d)| CardWidths {
                    words: option_card_width(ui, s.title, d, false),
                    with_icon: option_card_width(ui, s.title, d, true),
                })
                .collect();
            let columns = from_layout(ui.available_width(), &widths);
            for (i, ((s, detail), column)) in all.iter().zip(&details).zip(columns).enumerate() {
                let selected = i == form.source;
                // FEAT-0015: an address with nothing to spend is no source; dimmed, it fills
                // the address from the credits (or the pool) instead (frames 06, 14a, 10).
                // FEAT-0030: «Mixed» with no fully mixed coin yet, the same way (S2).
                let hint = match s.source {
                    Source::PlatformAddress => v.address_card.hint().map(str::to_owned),
                    Source::Mixed => v.mixed_hint.clone(),
                    _ => None,
                };
                if let Some(hint) = hint.as_deref() {
                    let resp = ui
                        .scope(|ui| {
                            ui.multiply_opacity(0.45);
                            option_card_in(ui, a, s.icon, s.title, detail, selected, column)
                        })
                        .inner;
                    let words = crate::i18n::t(hint).into_owned();
                    if resp.has_focus() {
                        resp.show_tooltip_text(words.clone());
                    }
                    let resp = resp.on_hover_text(words);
                    if resp.clicked()
                        && s.source == Source::PlatformAddress
                        && v.address_card == AddressCard::Empty
                    {
                        form.pick(v.fills_address);
                        form.recipient = v.own_platform_address.clone();
                    }
                    continue;
                }
                // The last known amount, not read in this run, is dimmed (BUG-113).
                let card = if source_dim(s) {
                    option_card_dim_in
                } else {
                    option_card_in
                };
                if card(ui, a, s.icon, s.title, detail, selected, column).clicked() {
                    form.source = i;
                    form.result = None;
                }
            }
        });
        if let Some(caption) = sources_caption(core_as_of, core_as_of_line.as_deref(), all) {
            ui.add(egui::Label::new(txt(&caption, 12.0, Weight::Regular, p.muted)).wrap());
        }
        // «Mixed»: why it gives no change and pays only Core, where the source is chosen (the
        // designer, S1): a plain line, not a banner at the bottom of the form.
        if let Some(s) = all.get(form.source).filter(|s| s.source == Source::Mixed) {
            ui.add(
                crate::components::para(s.note, 12.0, Weight::Regular, p.muted).line_height(18.0),
            );
        }
    });
}

/// What the form knows about whom it pays: the name found, or why not; the address book.
fn recipient_state(
    ui: &mut Ui,
    a: &mut Assets,
    form: &mut SendForm,
    v: &FormView,
    asked: &mut Asked,
) {
    recipient_lines(ui, a, form, v, asked);
    // From «Mixed» to a name or to Platform: the reason is about the recipient (S2).
    if v.mixed_to_recipient
        && let Some(why) = v.unavailable.as_deref()
    {
        let p = a.p;
        ui.add(crate::components::para(
            why,
            12.0,
            Weight::Regular,
            p.danger,
        ));
    }
    // Said once, right under the recipient, not under the amount (BUG-112).
    if v.pool_to_name
        && let Some(why) = v.unavailable.as_deref()
    {
        crate::components::attention_notice(ui, a, why);
    }
}

/// The recipient's card, or why there is none yet, and the address book.
fn recipient_lines(
    ui: &mut Ui,
    a: &mut Assets,
    form: &mut SendForm,
    v: &FormView,
    asked: &mut Asked,
) {
    let p = a.p;
    let all = v.all;
    let (to_self, checking, own, address_ok) = (v.to_self, v.checking, v.own, v.address_ok);
    let (resolved_name, name_problem) = (&v.resolved_name, &v.name_problem);
    let (address_problem, book) = (&v.address_problem, &v.book);
    if let Some(name) = &resolved_name {
        // `resolved` says a name is no recipient when given the Platform address (R-1а); the
        // wallet's own identity there is topped up, as from the credits.
        let from = match all[form.source].source {
            Source::PlatformAddress if !v.address_foreign_name => Source::Credits,
            other => other,
        };
        resolved(ui, a, name, from, &mut asked.commands);
        if v.address_foreign_name {
            crate::components::attention_notice(
                ui,
                a,
                tr!(
                    "From the Platform address you can only top up your own identity. To pay this person, enter their Platform address."
                ),
            );
        }
        if to_self {
            crate::components::attention_line(
                ui,
                a,
                tr!("This is your identity: the credits would stay yours, only the fee would go."),
            );
        }
    } else if checking {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs(1));
        ui.add(
            egui::Label::new(txt(
                crate::text::name_waiting(form.asked_at),
                12.0,
                Weight::Regular,
                p.muted,
            ))
            .wrap(),
        );
    } else if let Some((why, typo)) = &name_problem {
        let color = if *typo { p.danger } else { p.muted };
        ui.label(txt(why, 12.0, Weight::Regular, color));
    } else if let Some(why) = &address_problem {
        ui.add(egui::Label::new(txt(why, 12.0, Weight::Regular, p.danger)).wrap());
    }
    if all[form.source].source == Source::Core {
        if own && address_ok {
            ui.add(
                        egui::Label::new(txt(
                            tr!("This is an address of this wallet: the money stays yours, only the fee goes."),
                            12.0,
                            Weight::Regular,
                            p.muted,
                        ))
                        .wrap(),
                    );
        }
        address_book(ui, a, form, book, address_ok && !own, &mut asked.commands);
    }
}

/// Core pays a name only as a contact: the explanation and the request, instead of an amount.
fn contact_first(
    ui: &mut Ui,
    a: &mut Assets,
    form: &mut SendForm,
    v: &FormView,
    name: &ResolvedName,
    asked: &mut Asked,
) {
    let p = a.p;
    let contact_fee = v.contact_fee;
    let who = name
        .display
        .clone()
        .unwrap_or_else(|| format!("@{}", name.name));
    let (text, action) = match name.contact {
        Some(ContactState::Outgoing) => (
            format!(
                tr!(
                    "{who} already has your contact request. When {who} answers, you can pay from here. Credits can go to @{} right away: choose “Credits”."
                ),
                name.name,
                who = who
            ),
            None,
        ),
        Some(ContactState::Incoming) => (
            format!(
                tr!("{who} sent you a contact request: accept it, and you can pay from here."),
                who = who
            ),
            Some(tr!("Open contacts").to_string()),
        ),
        _ => (
            format!(
                tr!(
                    "A payment from Core to an @name goes to addresses from the key the person sends in answer to a contact request. Send {who} a request — once they answer, you can pay from here. Credits can go to @{} right away: choose “Credits”."
                ),
                name.name,
                who = who
            ),
            Some(format!(
                tr!("Add to contacts · ≈ {}"),
                crate::format::amount(contact_fee)
            )),
        ),
    };
    notice(ui, a, "contacts", p.accent, &text);
    if name.contact.is_none()
        && let Some(warning) = &v.contact_reserve
    {
        super::super::contacts::reserve_line(ui, &p, warning);
    }
    if let Some(Err(e)) = &form.contact_result
        && !form.contact_password.is_wrong()
    {
        crate::components::error_notice(
            ui,
            a,
            &super::super::contacts::contact_error(e),
            crate::text::error_details(e).as_deref(),
        );
    }
    // The request costs credits: "password for every payment" asks here too.
    let ask = action.is_some() && name.contact != Some(ContactState::Incoming) && v.ask_password;
    let entered = form
        .contact_password
        .field(ui, &p, "send-contact-password", ask);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        if let Some(action) = action {
            let label = if form.contact_waiting {
                tr!("Sending…").to_string()
            } else {
                action
            };
            let ready = !form.contact_waiting && form.contact_password.ready(ask);
            if button_enabled(ui, a, &label, ButtonKind::Primary, None, ready).clicked()
                || (entered && ready)
            {
                if name.contact == Some(ContactState::Incoming) {
                    asked.open_contacts = true;
                } else {
                    asked.commands.push(form.contact_password.command(
                        Spend::AddContact {
                            name: format!("@{}", name.name),
                        },
                        ask,
                    ));
                    form.contact_waiting = true;
                    form.contact_result = None;
                }
            }
        }
        if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
            form.replace(SendForm::empty());
        }
    });
}

/// The amount, and what is wrong with it.
fn amount_row(ui: &mut Ui, a: &mut Assets, form: &mut SendForm, v: &FormView) {
    let p = a.p;
    let source = v.source;
    let (unavailable, problem) = (v.unavailable.as_deref(), v.problem);
    // FEAT-0015 stage 2 (frame 14): what a payment to the wallet's own Platform address is.
    if v.fund_own {
        crate::components::notice_parts(
            ui,
            a,
            "platform-address",
            p.accent,
            &[(
                tr!("Top-up of your Platform address from Core"),
                tr!(
                    "Asset lock: DASH are locked on Core and credited to the address · InstantSend in seconds, otherwise ChainLock up to ~2.5 min"
                ),
            )],
        );
    }
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        field_label(ui, &p, tr!("Amount"));
        let rate = v.rate.as_ref();
        let typing_currency = form.typing_currency(rate);
        let unit = rate.map(|r| crate::components::UnitChoice {
            currency: &r.code,
            in_currency: &mut form.in_currency,
            off: (!r.fresh).then_some(tr!("The rate is older than 5 minutes")),
        });
        // Typed in the currency, the field holds that text; the amount in DASH comes from it.
        let text = if typing_currency {
            &mut form.fiat
        } else {
            &mut form.amount
        };
        let field = crate::components::amount_field_in(ui, &p, "send-amount", text, true, unit);
        let (resp, all) = (field.response, field.all);
        if field.switched {
            form.switched_unit();
        } else if resp.changed()
            && typing_currency
            && let Some(rate) = rate
        {
            form.typed_currency(rate);
        }
        if resp.changed() && !all {
            form.max = false;
        }
        if all {
            form.max = true;
            form.follow_source(source);
            form.follow_coins_max(&v.fees);
        }
        if resp.changed() || all {
            form.result = None;
        }
        // R-1а and BUG-112 are said under the recipient, once. From «Mixed», an excess too large
        // to burn has its own block with the amount that burns nothing (AC-A12, S3).
        if let Some(refused) = v.mixed_refused {
            super::mixed::refusal_block(ui, a, form, refused);
        } else if let Some(why) = unavailable
            .filter(|_| !v.address_foreign_name && !v.pool_to_name && !v.mixed_to_recipient)
        {
            if v.unsignable_short {
                // It wraps at 800 and in Arabic: in reading order (BUG-119).
                ui.add(crate::components::para(
                    why,
                    12.0,
                    Weight::Regular,
                    p.danger,
                ));
            } else {
                ui.add(egui::Label::new(txt(why, 12.0, Weight::Regular, p.muted)).wrap());
            }
        } else if let Some(problem) = problem {
            ui.add(
                egui::Label::new(txt(
                    &amount_problem_text(problem, source.fee, source.withdrawal),
                    12.0,
                    Weight::Regular,
                    p.danger,
                ))
                .wrap(),
            );
        }
        // BUG-142 AC-142-1: why «Max» is less than the balance, from Core only, while nothing else
        // is said here.
        if source.source == Source::Core
            && v.fees.core_unsignable > 0
            && unavailable.is_none()
            && problem.is_none()
        {
            ui.add(crate::components::para(
                &unsignable_hint(v.fees.core_unsignable),
                12.0,
                Weight::Regular,
                p.muted,
            ));
        }
        // By which rate (frames 1–3), «Fiat rate» on only (R-1).
        if let Some(line) = super::currency::field_line(form, rate, v.amount) {
            ui.add(egui::Label::new(txt(&line, 12.0, Weight::Regular, p.muted)).wrap());
        }
        // FEAT-0015 stage 2 (frame 14): the funding's fees in one line, as its review counts them.
        if v.fund_own
            && let Some(amount) = v.amount
        {
            let spendable = (v.source.duffs - v.fees.core_unsignable).max(0);
            let fees = match evolite_api::address_fund::fund_plan(amount, false, spendable, &v.fees)
            {
                Ok(plan) => plan.on_top + plan.core_fee,
                Err(_) => v.fees.address_fund_on_top() + v.fees.core_payment,
            };
            crate::components::kv_row(
                ui,
                &p,
                tr!("Core fee + processing on Platform"),
                &format!("≈ {}", crate::format::amount(fees)),
                crate::theme::mono(13.0, Weight::Medium),
            );
        }
    });
}

/// BUG-142 AC-142-1: a part of the Core balance no payment can sign for yet, in words. Two pieces,
/// each translated: the second, when it clears, is easy to drop (the analyst).
fn unsignable_hint(unsignable: Duffs) -> String {
    let lang = crate::i18n::current();
    let first = format!(
        tr!(
            "{amount} of the balance can't be spent yet: the wallet is still taking in these coins, usually money from a contact."
        ),
        amount = crate::format::balance(unsignable)
    );
    let when = tr!("This normally clears after the next sync.");
    // In reading order, each in the language: `para` lays the sentence out (BUG-119).
    format!(
        "{} {}",
        crate::i18n::t_in(lang, &first),
        crate::i18n::t_in(lang, when)
    )
}

/// How the money travels: a withdrawal arrives later than a payment.
fn route_notice(ui: &mut Ui, a: &mut Assets, v: &FormView) {
    let p = a.p;
    // «Mixed» says it under «From» (FEAT-0030 S1).
    if v.source.source == Source::Mixed {
        return;
    }
    if v.source.source == Source::PlatformAddress {
        return address_notice(ui, a, v);
    }
    // Credits or the pool to a Core address are a withdrawal: they arrive later, not in seconds;
    // from the pool, the line says what the network sees, which depends on the recipient (BUG-112).
    if let Some((icon, text)) = route_line(v.source, &v.recipient_label) {
        notice(ui, a, icon, p.accent, text);
    }
}

/// The fee's label: from the Platform addresses, Platform's or a withdrawal's (FEAT-0015).
fn fee_label(v: &FormView) -> &'static str {
    use evolite_api::address_plan::{AddressMove, move_for};
    match (v.source.source, move_for(&v.recipient_label)) {
        (Source::PlatformAddress, AddressMove::Withdraw) => tr!("Withdrawal fee"),
        (Source::PlatformAddress, _) => tr!("Platform fee"),
        _ => tr!("Network fee"),
    }
}

/// The fee: from the credits and the Platform addresses an estimate, «≈», as Platform meters
/// them (BUG-108).
fn fee_value(v: &FormView) -> String {
    // Nothing to pay for a payment that cannot be made (R-1а, BUG-112).
    if v.address_foreign_name || v.pool_to_name {
        return "—".into();
    }
    let fee = crate::format::amount(v.source.fee);
    if matches!(v.source.source, Source::Credits | Source::PlatformAddress) {
        format!("≈ {fee}")
    } else {
        fee
    }
}

/// Why a payment from the Platform addresses cannot be made as typed (FEAT-0015), before the
/// core is asked: too little on the addresses, a Core address not this wallet's (R-3), a name
/// not one of its identities (R-1а), a withdrawal that would not reach Core (UC-4).
pub(super) fn address_refusal(
    w: &Wallet,
    typed: &str,
    resolved: Option<&ResolvedName>,
    amount: Option<Duffs>,
    fee: Duffs,
) -> Option<String> {
    use evolite_api::address_plan::{AddressMove, MIN_INPUT, move_for};
    let kind = move_for(typed);
    if w.platform_inputs
        .iter()
        .all(|credits| *credits < MIN_INPUT + kind.fee(1))
    {
        return Some(
            tr!("The Platform address holds less than the minimum to spend. Top it up.").into(),
        );
    }
    if typed.trim().is_empty() {
        return None;
    }
    // To one of the wallet's own funded addresses: what that address holds does not pay (BUG-141,
    // QA pass 23). More than the others carry is «more than there is» — said here, not shown as
    // a fee (r1, should 1). Only when leaving the recipient out is what makes it too much.
    if kind == AddressMove::Transfer
        && let Some(amount) = amount.filter(|a| *a > 0)
    {
        use evolite_api::address_plan::{AddressRefusal, CREDITS_PER_DUFF, address_plan, plan_to};
        let credits = (amount as u64).saturating_mul(CREDITS_PER_DUFF);
        let (inputs, names) = (&w.platform_inputs, &w.platform_input_addresses);
        if matches!(
            plan_to(kind, credits, false, inputs, names, typed),
            Err(AddressRefusal::NotEnough { .. })
        ) && address_plan(kind, credits, false, inputs).is_ok()
        {
            return Some(tr!("More than there is in this balance").into());
        }
    }
    match kind {
        AddressMove::Withdraw if !own_address(w, typed) => Some(format!(
            "{}. {}.",
            crate::i18n::t(tr!(
                "From a Platform address to Core: only to this wallet's addresses"
            )),
            crate::i18n::t(tr!(
                "To pay this Core address, choose Core as the source. Or send to the recipient's Platform address"
            ))
        )),
        AddressMove::Withdraw if amount.is_some_and(|a| a <= fee) => Some(format!(
            tr!("The amount must be above ≈ {}"),
            crate::format::amount(fee)
        )),
        AddressMove::TopUp => resolved
            .filter(|n| !w.identities.iter().any(|i| i.id == n.identity_id))
            .map(|_| {
                tr!("From the Platform address you can only top up your own identity. To pay this person, enter their Platform address.")
                    .into()
            }),
        _ => None,
    }
}

/// From the Platform addresses: what the payment is, by the recipient (UC-1), and a withdrawal
/// that costs much of its amount (UC-4, analyst-47: above 10 %).
fn address_notice(ui: &mut Ui, a: &mut Assets, v: &FormView) {
    use evolite_api::address_plan::{AddressMove, move_for};
    let p = a.p;
    let to = v.recipient_label.trim();
    notice(ui, a, v.source.note_icon, p.accent, v.source.note);
    if to.is_empty() || v.unavailable.is_some() {
        return;
    }
    match move_for(to) {
        AddressMove::Transfer => {
            // The wallet's own address is not someone else's (QA pass 21). bech32m may be typed
            // in capitals too, and is never valid in mixed case (QA21 r1 P3 1).
            let whose = if to.eq_ignore_ascii_case(v.own_platform_address.trim()) {
                crate::i18n::t(tr!("this wallet's address"))
            } else {
                crate::i18n::t(tr!(
                    "Someone else's address · the money arrives at once, no queue"
                ))
            };
            notice(
                ui,
                a,
                "platform-address",
                p.accent,
                &format!(
                    "{} · {whose}",
                    crate::i18n::t(tr!("Transfer to a Platform address"))
                ),
            )
        }
        AddressMove::TopUp => notice(
            ui,
            a,
            "top-up",
            p.accent,
            &format!(
                tr!("Top up the credits of {} · your identity"),
                crate::i18n::own(to)
            ),
        ),
        AddressMove::Withdraw => {
            let fee = v.source.fee;
            match v.amount {
                Some(amount) if fee.saturating_mul(10) > amount => notice(
                    ui,
                    a,
                    "warning",
                    p.danger,
                    &format!(
                        tr!(
                            "A withdrawal to Core costs ≈ {}: that is {}% of the amount. It is cheaper to move it to an identity or the private pool first."
                        ),
                        crate::format::amount(fee),
                        (fee.saturating_mul(100) / amount.max(1)).max(1)
                    ),
                ),
                _ => notice(
                    ui,
                    a,
                    "withdraw",
                    p.accent,
                    &format!(
                        "{} · {}",
                        crate::i18n::t(tr!("Withdrawal to Core · this wallet's address")),
                        crate::i18n::t(tr!(
                            "Platform withdraws through a queue: the DASH usually reach Core within an hour"
                        ))
                    ),
                ),
            }
        }
    }
}

/// Core to a Platform address (FEAT-0015 frame 15, AC-D14): not possible directly; the way
/// there from the credits or from the wallet's own Platform address, the recipient kept.
fn platform_from_core(ui: &mut Ui, a: &mut Assets, form: &mut SendForm, v: &FormView) {
    let p = a.p;
    crate::components::notice(
        ui,
        a,
        "platform-address",
        p.accent,
        tr!("Send from credits or from your Platform address: the money arrives at once."),
    );
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        if button(ui, a, tr!("Send from credits ›"), ButtonKind::Ghost, None).clicked() {
            form.pick(Source::Credits);
        }
        if v.address_card == AddressCard::Source
            && v.all.iter().any(|s| s.source == Source::PlatformAddress)
            && button(
                ui,
                a,
                tr!("Send from the Platform address ›"),
                ButtonKind::Ghost,
                None,
            )
            .clicked()
        {
            form.pick(Source::PlatformAddress);
        }
    });
}

/// "Check and send", and what is still missing for it.
fn send_row(ui: &mut Ui, a: &mut Assets, form: &mut SendForm, v: &FormView, asked: &mut Asked) {
    let p = a.p;
    let source = v.source;
    let (amount, problem, unavailable) = (v.amount, v.problem, v.unavailable.as_deref());
    let (target_known, synced, checking) = (v.target_known, v.synced, v.checking);
    // A balance not read yet waits like the sync (BUG-113): the pool is read after it.
    let synced = synced && source.read;
    // From «Mixed» the core's plan for this amount comes first (FEAT-0030): its excess is shown
    // before the password, and one too large is refused.
    let mixed_waiting = source.source == Source::Mixed
        && unavailable.is_none()
        && amount.is_some()
        && v.mixed_quote.is_none();
    let mixed_ok =
        source.source != Source::Mixed || super::mixed::usable(v.mixed_quote.as_ref()).is_some();
    let ready = amount.is_some()
        && problem.is_none()
        && unavailable.is_none()
        && target_known
        && synced
        && mixed_ok;
    // From the pool to a name the button is outlined and says what to change (BUG-112).
    let kind = if v.pool_to_name {
        ButtonKind::Secondary
    } else {
        ButtonKind::Primary
    };
    if form.waiting > 0 {
        ui.add(
                    egui::Label::new(txt(
                        tr!("The payment awaits the network's answer: it is in the history as “Sending”, and the answer will come. The form is free for the next one."),
                        12.0,
                        Weight::Regular,
                        p.muted,
                    ))
                    .wrap(),
                );
    }
    // A grey button says nothing by itself: what is still missing — under the buttons,
    // wrapped, so a long reason does not widen the form past the window (BUG-36).
    let missing = if ready {
        None
    } else if v.pool_to_name {
        Some(tr!("Change the source or the recipient"))
    } else if !synced {
        Some(crate::text::WAIT_FOR_SYNC)
    } else if form.recipient.trim().is_empty() {
        Some(tr!("Enter whom to pay"))
    } else if form.amount.trim().is_empty() {
        Some(tr!("Enter the amount"))
    } else if checking {
        Some(tr!("Checking the recipient…"))
    } else if mixed_waiting {
        Some(tr!("Counting the mixed coins…"))
    } else if v.address_foreign_name {
        Some(tr!("Enter your own identity or a Platform address"))
    } else {
        None
    };
    // FEAT-0015 stage 2 (frame 14): to the wallet's own Platform address, the funding's review
    // (22b) with this amount, over this screen; the core checks it all again before the lock.
    if v.fund_own {
        let ready = amount.is_some() && problem.is_none() && synced;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            if button_enabled(ui, a, tr!("Check and top up"), kind, None, ready).clicked() {
                asked.fund = amount;
            }
            if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
                form.replace(SendForm::empty());
            }
        });
        if let Some(missing) = missing {
            ui.add(egui::Label::new(txt(missing, 12.0, Weight::Regular, p.muted)).wrap());
        }
        return;
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let label = tr!("Check and send");
        if button_enabled(ui, a, label, kind, None, ready).clicked() {
            form.confirming = true;
            form.result = None;
            // Chosen coins are read again for the last look: one spent meanwhile is said
            // there and the payment waits (R-10).
            if !form.coins.chosen.is_empty() {
                asked.commands.push(form.coins.ask_again());
            }
            // From Core the last look shows where the money goes: the address, or the
            // contact's next address from its key, as the core will pay it.
            form.preview = None;
            form.previewed = form.recipient.trim().to_string();
            if source.source == Source::Core {
                asked.commands.push(Command::PreviewPayment {
                    to: form.previewed.clone(),
                });
            }
        }
        if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
            form.replace(SendForm::empty());
        }
    });
    if let Some(missing) = missing {
        ui.add(egui::Label::new(txt(missing, 12.0, Weight::Regular, p.muted)).wrap());
    }
}

/// The network's answer to the last payment.
fn payment_answer(ui: &mut Ui, a: &mut Assets, form: &SendForm, v: &FormView) {
    let p = a.p;
    let source = v.source;
    let sent_to = &v.sent_to;
    let reveal = v.reveal;
    let answer = ui.scope(|ui| match &form.result {
                // No clear answer from the network: the payment may have gone.
                Some(Ok(sent)) if !sent.relayed => notice(
                    ui,
                    a,
                    "pending",
                    p.muted,
                    &format!(
                        tr!("The payment was handed to the nodes, but the network has not confirmed it yet. Do not send it again: it may go through. In the history it is “Sending”.{}"),
                        if sent.txid.is_empty() {
                            String::new()
                        } else {
                            format!(" {}", short_address(&sent.txid))
                        }
                    ),
                ),
                // Credits: Platform proves the transition, there is no Core transaction.
                Some(Ok(sent)) if sent.txid.is_empty() && source.source == Source::Pool => {
                    let withdrawal = !sent_to.starts_with("tdash1");
                    notice(
                        ui,
                        a,
                        "confirmed",
                        p.accent,
                        if withdrawal {
                            tr!("Platform accepted the withdrawal from the pool. The money arrives at the Core address in a few blocks, when the masternodes pay it out.")
                        } else {
                            tr!("Platform accepted the transfer from the pool.")
                        },
                    )
                }
                Some(Ok(sent)) if sent.txid.is_empty() => {
                    let withdrawal = !sent_to.starts_with('@')
                        && !sent_to.starts_with("tdash1");
                    notice(
                        ui,
                        a,
                        "confirmed",
                        p.accent,
                        if withdrawal {
                            tr!("Platform accepted the credit withdrawal. The money arrives at the Core address in a few blocks, when the masternodes pay it out.")
                        } else {
                            tr!("Credits sent: Platform confirmed the transfer with a proof.")
                        },
                    )
                }
                Some(Ok(sent)) => notice(
                    ui,
                    a,
                    "confirmed",
                    p.accent,
                    &format!(
                        tr!("Payment sent, the network accepted it. Fee {} · {}"),
                        crate::format::amount(sent.fee),
                        short_address(&sent.txid)
                    ),
                ),
                Some(Err(e @ CoreError::NotAvailable(_))) => {
                    notice(ui, a, "info", p.muted, &send_error(e))
                }
                Some(Err(e)) => crate::components::error_notice(ui, a, &send_error(e), crate::text::error_details(e).as_deref()),
                None => {}
            });
    if reveal {
        crate::diagnostics::ask_scroll(ui.ctx());
        answer.response.scroll_to_me(Some(Align::Center));
    }
}

/// The summary beside or under the form.
pub(super) fn summary_card(ui: &mut Ui, p: &crate::theme::Palette, v: &FormView, width: f32) {
    let source = v.source;
    let (amount, left, not_contact) = (v.amount, v.left, v.not_contact);
    let (summary_to, summary_note) = (&v.summary_to, &v.summary_note);
    // FEAT-0015 stage 2 (frame 14): to the wallet's own Platform address, the funding's plan.
    let fund = v
        .fund_own
        .then(|| {
            let spendable = (source.duffs - v.fees.core_unsignable).max(0);
            v.amount.and_then(|amount| {
                evolite_api::address_fund::fund_plan(amount, false, spendable, &v.fees).ok()
            })
        })
        .flatten();
    let own_to = v.fund_own.then(|| {
        format!(
            tr!("own {}"),
            crate::i18n::own(&crate::text::short_address(&v.own_platform_address))
        )
    });
    let summary_to = own_to.as_ref().unwrap_or(summary_to);
    card(p, Margin::same(24)).show(ui, |ui| {
        ui.set_width(width);
            ui.spacing_mut().item_spacing.y = 14.0;
            ui.label(txt(tr!("Summary"), 16.0, Weight::SemiBold, p.text));
            let value = mono(13.0, Weight::Medium);
            let dash = |d: Option<i64>| d.map_or("—".into(), crate::format::amount);
            // From chosen coins: «Core · of 3 coins», never a promise that all go (AC-D1).
            let mixed = super::mixed::usable(v.mixed_quote.as_ref());
            let from = match &v.coin_plan {
                Some(_) => format!(
                    tr!("Core · of {} coins"),
                    v.coins_chosen
                ),
                // «Mixed · 2 coins» once the core planned it (FEAT-0030 S1).
                None if source.source == Source::Mixed => super::mixed::summary_from(mixed),
                None => source.title.to_owned(),
            };
            kv_row(ui, p, tr!("From"), &from, sans(13.0, Weight::Medium));
            kv_row(
                ui,
                p,
                tr!("To"),
                summary_to,
                sans(13.0, Weight::Medium),
            );
            // Nothing is paid until the name is a contact, nor from the pool to a name (BUG-112),
            // nor from the Platform address to a foreign name (R-1а).
            let (amount, fee, left) = if v.pool_to_name {
                (amount, None, None)
            } else if not_contact || v.address_foreign_name {
                (None, None, source.read.then_some(source.duffs))
            } else {
                (amount, Some(source.fee), left.filter(|l| *l >= 0))
            };
            kv_row(ui, p, tr!("Amount"), &dash(amount), value.clone());
            if let Some(plan) = fund {
                kv_row(
                    ui,
                    p,
                    tr!("On the address, at least"),
                    &crate::format::amount(plan.arrives_at_least),
                    value.clone(),
                );
            }
            // «≈ 25,00 USD» under it, smaller (R-2, frame 1).
            if let Some(line) =
                amount.and_then(|d| super::currency::summary_line(v.rate.as_ref(), d))
            {
                small_right(ui, p, &line);
            }
            // Platform meters credits and address payments: an estimate there, as in the last look
            // (BUG-108, the QA pass of 2026-11-10).
            let about = matches!(source.source, Source::Credits | Source::PlatformAddress);
            let (fee, about, left) = match fund {
                Some(plan) => (
                    Some(plan.on_top + plan.core_fee),
                    true,
                    Some(source.duffs - plan.total),
                ),
                None => (fee, about, left),
            };
            let fee_text = match fee {
                Some(fee) if about => format!("≈ {}", crate::format::amount(fee)),
                _ => dash(fee),
            };
            // From chosen coins the fee and the change are estimates: how many coins the wallet
            // takes is known once the payment is built (F2).
            match &v.coin_plan {
                Some(Ok(plan)) => {
                    kv_row(
                        ui,
                        p,
                        tr!("Fee"),
                        &format!("≈ {}", crate::format::amount(plan.fee)),
                        value.clone(),
                    );
                    if plan.change > 0 {
                        kv_row(
                            ui,
                            p,
                            tr!("Change back"),
                            &format!("≈ {}", crate::format::amount(plan.change)),
                            value.clone(),
                        );
                    }
                }
                _ => kv_row(ui, p, tr!("Fee"), &fee_text, value.clone()),
            }
            // The excess the coins give to the miners, over dust (FEAT-0030 S1), and as it is when
            // too large (S3).
            let excess = match v.mixed_refused {
                Some((excess, _)) => Some(excess),
                None => mixed.and_then(super::mixed::shown_excess),
            };
            if let Some(excess) = excess.filter(|_| source.source == Source::Mixed) {
                kv_row(
                    ui,
                    p,
                    tr!("Excess · to miners"),
                    &crate::format::amount(excess),
                    value.clone(),
                );
            }
            kv_row(
                ui,
                p,
                tr!("Left"),
                &left.map_or("—".into(), crate::format::balance),
                value,
            );
            if let Some(note) = &summary_note {
                summary_note_row(ui, p, note);
            }
            divider(ui, p);
            ui.add(
                egui::Label::new(
                    txt(
                        tr!("A payment cannot be undone. Before signing, evolite shows the recipient's address once more."),
                        12.0,
                        Weight::Regular,
                        p.muted,
                    )
                    .line_height(Some(18.0)),
                )
                .wrap(),
            );
    });
}

/// A small muted line on the right, under a row's value: an amount in the currency.
pub(super) fn small_right(ui: &mut Ui, p: &crate::theme::Palette, line: &str) {
    ui.with_layout(Layout::top_down(Align::Max), |ui| {
        ui.label(txt(line, 12.0, Weight::Regular, p.muted));
    });
}
