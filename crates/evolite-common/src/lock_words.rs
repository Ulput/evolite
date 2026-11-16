//! An asset lock refused for its coins (SPV Б2, item 3), in words: the same in the form, before
//! «Check», and from the core, before the password.

use evolite_api::lock_coins::LockCoins;

/// Б2: more inputs than the network takes.
pub const TOO_MANY_COINS: &str = tr!(
    "This amount needs more than {} coins — the network will not take such a transaction. First combine the small coins: send “Max” from Core to your own address and try again"
);

/// Item 3: the final coins fall short; the others are not confirmed yet.
pub const NOT_FINAL_YET: &str =
    tr!("Some of the coins are not confirmed by the network yet — wait for a block");

/// The refusal [`LockCoins`] says, if any: `None` when the lock can be made, and when the
/// wallet is short in all (each form says that in its own words).
pub fn lock_refusal(coins: LockCoins) -> Option<String> {
    match coins {
        LockCoins::TooMany { allowed } => Some(TOO_MANY_COINS.replace("{}", &allowed.to_string())),
        LockCoins::NotFinal => Some(NOT_FINAL_YET.to_owned()),
        LockCoins::Fits(_) | LockCoins::Short => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_refusal_says_its_own() {
        let words = lock_refusal(LockCoins::TooMany { allowed: 90 }).unwrap();
        assert!(
            words.starts_with("This amount needs more than 90 coins"),
            "{words}"
        );
        assert_eq!(
            lock_refusal(LockCoins::NotFinal).as_deref(),
            Some(NOT_FINAL_YET)
        );
        assert_eq!(lock_refusal(LockCoins::Fits(3)), None);
        assert_eq!(lock_refusal(LockCoins::Short), None);
    }
}
