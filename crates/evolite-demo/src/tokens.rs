//! The demo's tokens (FEAT-0019 AC-T7): the example identity's tokens in every state a send is
//! refused for before the password (paused, frozen, not transferable, not known yet, zero), with
//! and without history, found by the search and added by hand; the example network's tokens to
//! add by id or keyword; and a staged "Find my tokens". Names are the issuers' words, as on
//! Platform: they are not translated. Nothing is asked of a network.

use evolite_api::tokens::{
    SearchState, TokenAmount, TokenInfo, TokenOrigin, TokenRow, TokenSearch, Tokens,
};
use evolite_api::{CoreError, Timestamp};

/// Contracts on the example network, as the first whole pass counted them.
pub const CONTRACTS: u32 = 4175;

/// How long the staged "Find my tokens" takes, in seconds.
pub const PASS_SECS: f32 = 4.0;

/// A token of the example network: what its contract says, its keywords, and the example
/// identity's balance of it (`None`: not read yet).
struct Listed {
    info: TokenInfo,
    keywords: &'static [&'static str],
    balance: Option<TokenAmount>,
}

fn info(
    token_id: &str,
    contract_id: &str,
    position: u16,
    name: &str,
    decimals: u8,
    issuer_name: Option<&str>,
) -> TokenInfo {
    TokenInfo {
        token_id: token_id.into(),
        contract_id: contract_id.into(),
        position,
        names: vec![("en".into(), name.into())],
        decimals,
        issuer: "5TKGz2Jm9wDqYp4rHcV8sNbE1xUa7LfQ3kRt6WhXoP2d".into(),
        issuer_name: issuer_name.map(Into::into),
        transferable: true,
        keeps_history: false,
    }
}

const COFFEE_CONTRACT: &str = "Ab3xK9mPq2Rt7VwYz4Nc8Ld1Fg6Hj5Ks3Bv9Cx2Mn7Qe";

fn network() -> Vec<Listed> {
    vec![
        Listed {
            info: TokenInfo {
                keeps_history: true,
                ..info(
                    "7Hq2Wd9kLm4Np8Rs3Tv6Xy1Zb5Cf2Gh7Jk9Mn4Pq8Rs",
                    COFFEE_CONTRACT,
                    0,
                    "Evo Coffee",
                    2,
                    Some("evo-lab"),
                )
            },
            keywords: &["coffee", "evo"],
            balance: Some(12_550),
        },
        Listed {
            info: info(
                "3Fk8Lp2Qr6Sv9Tw4Xz7Yb1Cd5Eg8Hj2Km6Np9Qs3Rt",
                COFFEE_CONTRACT,
                1,
                "Coffee Club Stamp",
                0,
                Some("evo-lab"),
            ),
            keywords: &["coffee", "evo"],
            balance: Some(0),
        },
        Listed {
            info: info(
                "9Mn3Pq7Rs2Tv6Wx1Yz5Ab8Cd4Ef9Gh3Jk7Lm2Np6Qr",
                "Bc4yL2nQr8St3Uw9Xa6Zc1De7Fh4Gj8Kl2Mp5Nq9Rs",
                0,
                "Workshop Hours",
                0,
                None,
            ),
            keywords: &["workshop"],
            balance: Some(40),
        },
        Listed {
            info: info(
                "4Gh8Jk2Lm6Np1Qr5St9Uv3Wx7Yz2Ab6Cd1Ef5Gh9Jk",
                "Cd5zM3oRs9Tu4Vx1Yb7Ad2Eg8Gi5Hk9Lm3Nq6Or1St",
                0,
                "Testnet Gold",
                8,
                Some("goldsmith"),
            ),
            keywords: &["gold"],
            balance: Some(1_000_000_000),
        },
        Listed {
            info: info(
                "6Jk1Lm5Np9Qr3St7Uv2Wx6Yz1Ab5Cd9Ef3Gh7Jk2Lm",
                "De6aN4pSt1Uv5Wy2Zc8Be3Fh9Hj6Il1Mn4Or7Ps2Tu",
                0,
                "Meetup Ticket",
                0,
                Some("dash-meetup"),
            ),
            keywords: &["meetup", "ticket"],
            balance: Some(3),
        },
        Listed {
            info: TokenInfo {
                transferable: false,
                ..info(
                    "8Lm2Np6Qr1St5Uv9Wx3Yz7Ab2Cd6Ef1Gh5Jk9Lm3Np",
                    "De6aN4pSt1Uv5Wy2Zc8Be3Fh9Hj6Il1Mn4Or7Ps2Tu",
                    1,
                    "Speaker Badge",
                    0,
                    Some("dash-meetup"),
                )
            },
            keywords: &["meetup"],
            balance: Some(1),
        },
        Listed {
            info: info(
                "2Np7Qr1St6Uv1Wx5Yz9Ab4Cd8Ef3Gh7Jk1Lm6Np2Qr",
                "Ef7bO5qTu2Vw6Xz3Ad9Cf4Gi1Ik7Jm2No5Ps8Qt3Uv",
                0,
                "Garden Points",
                2,
                None,
            ),
            keywords: &["garden"],
            balance: None,
        },
        Listed {
            info: info(
                "5Qr9St3Uv7Wx2Yz6Ab1Cd5Ef9Gh4Jk8Lm3Np7Qr2St",
                "Fg8cP6rUv3Wx7Ya4Be1Dg5Hj2Jl8Kn3Op6Qt9Ru4Vw",
                0,
                "Evo Shares",
                4,
                Some("evo-lab"),
            ),
            keywords: &["shares", "evo"],
            balance: Some(0),
        },
    ]
}

fn row(listed: &Listed, origin: TokenOrigin) -> TokenRow {
    let known = listed.balance.is_some();
    TokenRow {
        info: listed.info.clone(),
        origin,
        balance: listed.balance,
        paused: known.then_some(false),
        frozen: known.then_some(false),
        claim: false,
    }
}

/// The example identity's tokens: one in each state, and the search's last pass three hours ago.
pub fn demo_tokens(t: Timestamp) -> Tokens {
    let net = network();
    let by = |name: &str| {
        net.iter()
            .find(|l| evolite_api::tokens::name_in(&l.info, "en") == name)
            .expect("an example token")
    };
    let rows = vec![
        // Found by the search, with history and something to claim.
        TokenRow {
            claim: true,
            ..row(by("Evo Coffee"), TokenOrigin::Found)
        },
        // Added by hand, no history, whole units.
        row(by("Workshop Hours"), TokenOrigin::ByHand),
        // The issuer paused it.
        TokenRow {
            paused: Some(true),
            ..row(by("Testnet Gold"), TokenOrigin::Found)
        },
        // The issuer froze this balance.
        TokenRow {
            frozen: Some(true),
            ..row(by("Meetup Ticket"), TokenOrigin::Found)
        },
        // Not transferable.
        row(by("Speaker Badge"), TokenOrigin::Found),
        // Added a moment ago: nothing read yet (AC-T5, never shown as zero).
        row(by("Garden Points"), TokenOrigin::ByHand),
        // Added by hand, nothing on it.
        row(by("Evo Shares"), TokenOrigin::ByHand),
    ];
    Tokens {
        rows,
        search: TokenSearch {
            state: SearchState::Idle,
            checked: CONTRACTS,
            total: Some(CONTRACTS),
            left_secs: None,
            done: Some((t - 3 * 60 * 60, CONTRACTS)),
            failed: false,
        },
        hidden: Vec::new(),
    }
}

/// "Add token" by id: the token, or every token of the contract with that id.
pub fn find(id: &str) -> Vec<TokenInfo> {
    let id = id.trim();
    network()
        .into_iter()
        .filter(|l| l.info.token_id == id || l.info.contract_id == id)
        .map(|l| l.info)
        .collect()
}

/// "Add token" by a word: an exact match, lower case, 3 to 50 characters; another word finds
/// nothing, as on Platform.
pub fn search(keyword: &str) -> Vec<TokenInfo> {
    let len = keyword.chars().count();
    if !(3..=50).contains(&len) || keyword.to_lowercase() != keyword {
        return Vec::new();
    }
    network()
        .into_iter()
        .filter(|l| l.keywords.contains(&keyword))
        .map(|l| l.info)
        .collect()
}

/// Follow `token_id` by hand (`on`) or take it out of the list.
pub fn track(tokens: &mut Tokens, token_id: &str, on: bool) -> Result<(), CoreError> {
    if !on {
        // A found one is hidden, as the real core does: it can be brought back.
        if tokens
            .rows
            .iter()
            .any(|r| r.info.token_id == token_id && r.origin == TokenOrigin::Found)
        {
            tokens.hidden.push(token_id.to_owned());
        }
        tokens.rows.retain(|r| r.info.token_id != token_id);
        return Ok(());
    }
    // «Return to the list»: the example's own row again, as it was.
    if tokens.hidden.iter().any(|h| h == token_id) {
        tokens.hidden.retain(|h| h != token_id);
        if let Some(back) = demo_tokens(0)
            .rows
            .into_iter()
            .find(|r| r.info.token_id == token_id)
        {
            tokens.rows.push(back);
        }
        return Ok(());
    }
    if tokens.rows.iter().any(|r| r.info.token_id == token_id) {
        return Ok(());
    }
    let listed = network()
        .into_iter()
        .find(|l| l.info.token_id == token_id)
        .ok_or_else(|| CoreError::NotAvailable(crate::OFFLINE.into()))?;
    tokens.rows.push(row(&listed, TokenOrigin::ByHand));
    Ok(())
}

/// The staged "Find my tokens" `secs` after it started; `true` once the pass is over.
pub fn step(search: &mut TokenSearch, secs: f32, now: Timestamp) -> bool {
    let fraction = (secs / PASS_SECS).clamp(0.0, 1.0);
    if fraction >= 1.0 {
        *search = TokenSearch {
            state: SearchState::Idle,
            checked: CONTRACTS,
            total: Some(CONTRACTS),
            left_secs: None,
            done: Some((now, CONTRACTS)),
            failed: false,
        };
        return true;
    }
    search.state = SearchState::Running;
    search.total = Some(CONTRACTS);
    search.checked = (CONTRACTS as f32 * fraction) as u32;
    search.left_secs = Some((PASS_SECS - secs).ceil().max(1.0) as u32);
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolite_api::tokens::{Sender, TokenRefusal, send_refusal};

    const SENDER: Sender = Sender {
        has_key: true,
        credits: Some(1_000_000),
        fee: 1_000,
    };

    #[test]
    fn the_example_has_a_token_in_each_state_a_send_is_refused_for() {
        let tokens = demo_tokens(1_000_000);
        let refusals: Vec<_> = tokens
            .rows
            .iter()
            .map(|r| send_refusal(r, Some(1), SENDER))
            .collect();
        for want in [
            None,
            Some(TokenRefusal::Paused),
            Some(TokenRefusal::Frozen),
            Some(TokenRefusal::NotTransferable),
            Some(TokenRefusal::NotKnown),
            Some(TokenRefusal::MoreThanBalance(0)),
        ] {
            assert!(refusals.contains(&want), "{want:?} in {refusals:?}");
        }
        let shown = |f: fn(&TokenRow) -> bool| tokens.rows.iter().any(f);
        assert!(shown(|r| r.info.keeps_history) && shown(|r| !r.info.keeps_history));
        assert!(
            shown(|r| r.origin == TokenOrigin::Found) && shown(|r| r.origin == TokenOrigin::ByHand)
        );
        assert!(shown(|r| r.claim));
        assert!(
            tokens
                .rows
                .iter()
                .all(|r| r.balance.is_some() || (r.paused.is_none() && r.frozen.is_none())),
            "not read yet is not known at all, never zero"
        );
    }

    #[test]
    fn found_by_id_by_contract_and_by_exact_keyword() {
        let coffee = "7Hq2Wd9kLm4Np8Rs3Tv6Xy1Zb5Cf2Gh7Jk9Mn4Pq8Rs";
        assert_eq!(find(coffee).len(), 1);
        assert_eq!(
            find(COFFEE_CONTRACT).len(),
            2,
            "every token of the contract"
        );
        assert!(find("nothing").is_empty());
        assert_eq!(search("coffee").len(), 2);
        assert!(search("Coffee").is_empty(), "lower case only");
        assert!(search("cof").is_empty(), "an exact match");
        assert!(search("ev").is_empty(), "3 characters at least");
    }

    #[test]
    fn tracked_by_hand_once_and_taken_out() {
        let mut tokens = demo_tokens(0);
        let stamp = "3Fk8Lp2Qr6Sv9Tw4Xz7Yb1Cd5Eg8Hj2Km6Np9Qs3Rt";
        let before = tokens.rows.len();
        track(&mut tokens, stamp, true).unwrap();
        track(&mut tokens, stamp, true).unwrap();
        assert_eq!(tokens.rows.len(), before + 1);
        let added = tokens.rows.last().unwrap();
        assert_eq!(
            (added.origin, added.balance),
            (TokenOrigin::ByHand, Some(0))
        );
        track(&mut tokens, stamp, false).unwrap();
        assert_eq!(tokens.rows.len(), before);
        assert!(track(&mut tokens, "not on the example network", true).is_err());
    }

    #[test]
    fn find_my_tokens_runs_and_ends_with_a_whole_pass() {
        let mut search = demo_tokens(0).search;
        assert!(!step(&mut search, 1.0, 50));
        assert_eq!(search.state, SearchState::Running);
        assert!(search.checked > 0 && search.checked < CONTRACTS);
        assert!(search.left_secs.is_some());
        assert!(step(&mut search, PASS_SECS, 60));
        assert_eq!(search.state, SearchState::Idle);
        assert_eq!(search.done, Some((60, CONTRACTS)));
    }
}
