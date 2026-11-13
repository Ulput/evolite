//! The coins a person chooses for a Core payment (FEAT-0013, the customer's path 2): listed
//! from the wallet without the network, kept to the chosen set before the payment is built,
//! and checked against the signed transaction before it is sent.

use evolite_api::{Coin, CoinState, Duffs, Timestamp};
use key_wallet::Utxo;
use key_wallet::dashcore::{OutPoint, Txid};
use std::collections::HashSet;

/// The most inputs a payment may have: key-wallet's `MAX_STANDARD_TX_INPUTS` (R-12).
pub const MAX_COINS: usize = 500;

/// A coin's outpoint as the interface names it: `txid:vout`.
pub fn outpoint_text(outpoint: &OutPoint) -> String {
    format!("{}:{}", outpoint.txid, outpoint.vout)
}

/// Why the chosen coins cannot pay (refused before anything is built).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChoiceError {
    /// Nothing chosen.
    Empty,
    /// More than [`MAX_COINS`].
    TooMany(usize),
    /// A chosen coin is spent, gone, or no longer final (UC-4, R-10): its outpoint.
    Gone(String),
    /// The signed payment did not take what was chosen, or would pay less than asked: not sent
    /// (R-6).
    Changed,
    /// Coins of the mixing account not fully mixed together with ordinary ones (FEAT-0030 AC-A6):
    /// together they would show the link.
    MixedWithOrdinary,
}

/// What an error string of a payment refused for its coins starts with: the core says the rest
/// as it is (a reason in the interface's words), not as a failure.
pub const REFUSED: &str = "coins: ";

/// The words for a refusal, behind [`REFUSED`].
pub fn refusal(e: &ChoiceError) -> String {
    let words = match e {
        ChoiceError::Empty | ChoiceError::Changed => tr!("Check the selection.").to_owned(),
        ChoiceError::TooMany(_) => tr!("At most 500 coins in one payment: untick some.").to_owned(),
        ChoiceError::MixedWithOrdinary => {
            tr!("Coins that are not fully mixed and ordinary coins cannot be spent in one payment.")
                .to_owned()
        }
        ChoiceError::Gone(outpoint) => format!(
            tr!(
                "The coin {} is already spent, perhaps by a payment from another device. Untick it and check the selection before you go on."
            ),
            short_outpoint(outpoint)
        ),
    };
    format!("{REFUSED}{words}")
}

/// `ab12cd…:1`: a coin as a sentence names it.
pub fn short_outpoint(outpoint: &str) -> String {
    match outpoint.split_once(':') {
        Some((txid, vout)) if txid.len() > 10 => format!("{}…:{vout}", &txid[..8]),
        _ => outpoint.to_owned(),
    }
}

/// The payable coins (`payable`, final, of the paying accounts) that are in `chosen`, or why the
/// choice cannot be used: every chosen coin must still be among them — one that is not would be
/// dropped silently by the builder (the card's trap 2).
pub fn within(payable: Vec<Utxo>, chosen: &[String]) -> Result<Vec<Utxo>, ChoiceError> {
    if chosen.is_empty() {
        return Err(ChoiceError::Empty);
    }
    let wanted: HashSet<&str> = chosen.iter().map(String::as_str).collect();
    if wanted.len() > MAX_COINS {
        return Err(ChoiceError::TooMany(wanted.len()));
    }
    let coins: Vec<Utxo> = payable
        .into_iter()
        .filter(|u| wanted.contains(outpoint_text(&u.outpoint).as_str()))
        .collect();
    if coins.len() != wanted.len() {
        let found: HashSet<String> = coins.iter().map(|u| outpoint_text(&u.outpoint)).collect();
        let gone = chosen
            .iter()
            .find(|o| !found.contains(*o))
            .cloned()
            .unwrap_or_default();
        return Err(ChoiceError::Gone(gone));
    }
    Ok(coins)
}

/// [`within`] over two kinds of coin (FEAT-0030 AC-A6): the ordinary payable ones and
/// `unfinished`, the mixing account's final coins that have not been through all the rounds. The
/// chosen coins must all be of one kind; `true` with them when they are the unfinished ones.
pub fn within_apart(
    ordinary: Vec<Utxo>,
    unfinished: Vec<Utxo>,
    chosen: &[String],
) -> Result<(Vec<Utxo>, bool), ChoiceError> {
    let of_mixing: HashSet<String> = unfinished
        .iter()
        .map(|u| outpoint_text(&u.outpoint))
        .collect();
    let n = chosen.iter().filter(|o| of_mixing.contains(*o)).count();
    match n {
        0 => within(ordinary, chosen).map(|c| (c, false)),
        n if n == chosen.len() => within(unfinished, chosen).map(|c| (c, true)),
        _ => Err(ChoiceError::MixedWithOrdinary),
    }
}

/// Whether a signed payment's inputs are what the person chose (R-6, АРХ-2): each one of the
/// set, and with `all` («Max»: every chosen coin) the whole set.
pub fn inputs_kept<'a>(
    inputs: impl IntoIterator<Item = &'a OutPoint>,
    chosen: &[String],
    all: bool,
) -> bool {
    let chosen: HashSet<&str> = chosen.iter().map(String::as_str).collect();
    let mut used = HashSet::new();
    for input in inputs {
        let text = outpoint_text(input);
        if !chosen.contains(text.as_str()) {
            return false;
        }
        used.insert(text);
    }
    !all || used.len() == chosen.len()
}

/// The coins as «Coins» lists them: every coin of the paying accounts, each once.
///
/// - `account_of`: for each coin, the contact whose DashPay account holds it, if any.
/// - `verified`: whether a txid's InstantLock was checked in this run (BUG-111).
/// - `at`: when the wallet saw a txid, if it knows.
pub fn listed<'a>(
    coins: impl IntoIterator<Item = (&'a Utxo, Option<String>)>,
    verified: impl Fn(&Txid) -> bool,
    at: impl Fn(&Txid) -> Option<Timestamp>,
) -> Vec<Coin> {
    let mut seen = HashSet::new();
    let mut out: Vec<Coin> = coins
        .into_iter()
        .filter(|(u, _)| seen.insert(u.outpoint))
        .map(|(u, from_contact)| Coin {
            outpoint: outpoint_text(&u.outpoint),
            value: u.value() as Duffs,
            address: u.address.to_string(),
            from_contact,
            state: if u.is_locked {
                CoinState::Reserved
            } else if crate::convert::coin_is_final(u, &verified) {
                CoinState::Final
            } else {
                CoinState::Unconfirmed
            },
            at: at(&u.outpoint.txid),
            mixing: None,
        })
        .collect();
    // Largest first, as the dialog opens (DDW's default).
    out.sort_by(|a, b| {
        b.value
            .cmp(&a.value)
            .then_with(|| a.outpoint.cmp(&b.outpoint))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use key_wallet::dashcore::hashes::Hash;
    use key_wallet::dashcore::{Address, TxOut};

    fn coin(n: u8, value: u64, confirmed: bool) -> Utxo {
        let outpoint = OutPoint::new(Txid::from_byte_array([n; 32]), u32::from(n));
        let address = Address::new(
            key_wallet::Network::Testnet,
            key_wallet::dashcore::address::Payload::PubkeyHash(
                key_wallet::dashcore::PubkeyHash::from_byte_array([n; 20]),
            ),
        );
        let txout = TxOut {
            value,
            script_pubkey: Default::default(),
        };
        let mut u = Utxo::new(
            outpoint,
            txout,
            address,
            if confirmed { 100 } else { 0 },
            false,
        );
        u.is_confirmed = confirmed;
        u
    }

    fn texts(coins: &[Utxo]) -> Vec<String> {
        coins.iter().map(|u| outpoint_text(&u.outpoint)).collect()
    }

    #[test]
    fn the_payment_takes_only_the_chosen_coins() {
        let all = vec![coin(1, 10, true), coin(2, 20, true), coin(3, 30, true)];
        let chosen = texts(&all[..2]);
        let kept = within(all.clone(), &chosen).unwrap();
        assert_eq!(texts(&kept), chosen);
    }

    #[test]
    fn a_chosen_coin_no_longer_payable_refuses_before_building() {
        // Spent, or not final any more: not among the payable coins.
        let payable = vec![coin(1, 10, true)];
        let chosen = texts(&[coin(1, 10, true), coin(2, 20, true)]);
        assert_eq!(
            within(payable, &chosen),
            Err(ChoiceError::Gone(chosen[1].clone()))
        );
        assert!(refusal(&ChoiceError::Gone(chosen[1].clone())).starts_with(REFUSED));
        assert_eq!(short_outpoint(&chosen[1]), "02020202…:2");
        assert_eq!(within(vec![coin(1, 1, true)], &[]), Err(ChoiceError::Empty));
    }

    #[test]
    fn more_than_five_hundred_coins_are_refused_before_building() {
        let chosen: Vec<String> = (0..=MAX_COINS).map(|n| format!("{n:064x}:0")).collect();
        assert_eq!(within(Vec::new(), &chosen), Err(ChoiceError::TooMany(501)));
    }

    #[test]
    fn signed_inputs_outside_the_set_or_short_of_it_for_max_are_caught() {
        let all = [coin(1, 10, true), coin(2, 20, true), coin(3, 30, true)];
        let chosen = texts(&all[..2]);
        let op = |u: &Utxo| u.outpoint;
        // Path 2: some of the set is fine; all of it too.
        assert!(inputs_kept([op(&all[0])].iter(), &chosen, false));
        assert!(inputs_kept(
            [op(&all[0]), op(&all[1])].iter(),
            &chosen,
            true
        ));
        // «Max»: one left out.
        assert!(!inputs_kept([op(&all[0])].iter(), &chosen, true));
        // A coin not chosen (swapped in by the builder): never.
        assert!(!inputs_kept(
            [op(&all[0]), op(&all[2])].iter(),
            &chosen,
            false
        ));
    }

    #[test]
    fn the_list_says_each_coins_state_largest_first_each_once() {
        let mut locked = coin(4, 40, true);
        locked.is_locked = true;
        let mut instant = coin(5, 5, false);
        instant.is_instantlocked = true;
        let final_one = coin(1, 10, true);
        let mempool = coin(2, 20, false);
        let list = listed(
            [
                (&final_one, None),
                (&mempool, Some("@alice".to_owned())),
                (&locked, None),
                (&instant, None),
                (&final_one, None),
            ],
            // No InstantLock was checked in this run: a locked mark alone is not final.
            |_| false,
            |txid| (txid == &final_one.outpoint.txid).then_some(1_700_000_000),
        );
        let states: Vec<(Duffs, CoinState)> = list.iter().map(|c| (c.value, c.state)).collect();
        assert_eq!(
            states,
            vec![
                (40, CoinState::Reserved),
                (20, CoinState::Unconfirmed),
                (10, CoinState::Final),
                (5, CoinState::Unconfirmed),
            ]
        );
        assert_eq!(list[1].from_contact.as_deref(), Some("@alice"));
        assert_eq!(list[2].at, Some(1_700_000_000));
        assert_eq!(list[2].outpoint, outpoint_text(&final_one.outpoint));
    }

    /// FEAT-0030 AC-A6: not fully mixed coins go only with others like them; a choice of both
    /// kinds is refused before anything is built.
    #[test]
    fn unfinished_mixed_coins_go_apart_from_ordinary_ones() {
        let ordinary = vec![coin(1, 10, true), coin(2, 20, true)];
        let unfinished = vec![coin(3, 100_001, true), coin(4, 100_001, true)];
        let (coins, mixing) =
            within_apart(ordinary.clone(), unfinished.clone(), &texts(&ordinary)).unwrap();
        assert_eq!((texts(&coins), mixing), (texts(&ordinary), false));
        let (coins, mixing) = within_apart(
            ordinary.clone(),
            unfinished.clone(),
            &texts(&unfinished[..1]),
        )
        .unwrap();
        assert_eq!((texts(&coins), mixing), (texts(&unfinished[..1]), true));
        let both = vec![texts(&ordinary)[0].clone(), texts(&unfinished)[0].clone()];
        assert_eq!(
            within_apart(ordinary, unfinished, &both),
            Err(ChoiceError::MixedWithOrdinary)
        );
    }
}
