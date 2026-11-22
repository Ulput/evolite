//! FEAT-0055: the Collection shows the NFTs of every identity of the wallet together; with
//! several identities a filter on top — the one identity picker (BUG-265) with «All» first —
//! shows one identity's only; with one identity there is no filter. On the hard preset of 10
//! long-named identities (TEAM.md §8d).

use super::*;
use crate::screens::collection::groups;
use evolite_api::NftsStatus;

fn nfts(kit: &Kit) -> Vec<evolite_api::Nft> {
    kit.state()
        .snapshot
        .as_ref()
        .and_then(|s| s.wallet.as_ref())
        .map(|w| w.nfts.clone())
        .unwrap_or_default()
}

/// The tiles' names for the NFTs at `shown` (as the grid names them: «Djembe · ×3» for a group).
fn labels(nfts: &[evolite_api::Nft], shown: &[usize]) -> Vec<String> {
    groups(nfts, shown)
        .iter()
        .map(|g| {
            let name = &nfts[g[0]].name;
            if g.len() > 1 {
                format!("{name} · ×{}", g.len())
            } else {
                name.clone()
            }
        })
        .collect()
}

fn tile(kit: &Kit, label: &str) -> bool {
    kit.query_by(|n| n.role() == Role::Button && n.label().as_deref() == Some(label))
        .is_some()
}

/// The tiles on screen are exactly those of the NFTs `owned` (`None`: every NFT).
fn shows_only(kit: &Kit, owned: Option<&str>) -> bool {
    let nfts = nfts(kit);
    let all: Vec<usize> = (0..nfts.len()).collect();
    let mine: Vec<usize> = all
        .iter()
        .copied()
        .filter(|&i| owned.is_none_or(|o| nfts[i].owner_id == o))
        .collect();
    let wanted = labels(&nfts, &mine);
    let mut others = labels(&nfts, &all);
    for i in &all {
        others.extend(labels(&nfts, &[*i]));
    }
    !wanted.is_empty()
        && wanted.iter().all(|l| tile(kit, l))
        && others
            .iter()
            .filter(|l| !wanted.contains(l))
            .all(|l| !tile(kit, l))
}

const SECOND: &str = "@werner555-identity-number-1-with-a-long-name";

/// «All» is on at first and shows every identity's NFTs; an identity's chip shows its own only;
/// «All» again shows them all.
#[test]
fn all_shows_every_identitys_nfts_and_a_chip_only_its_own() {
    let (_turn, mut kit) = demo_app("collection-identities-long", "en");
    wait_for(&mut kit, "every NFT", |kit| shows_only(kit, None));
    assert!(kit.state().collection.only.is_none());
    kit.get_by_label(SECOND).click();
    kit.run_steps(2);
    let id = nfts(&kit)
        .iter()
        .find(|n| n.owner == SECOND)
        .map(|n| n.owner_id.clone())
        .expect("the second identity holds an NFT");
    assert_eq!(kit.state().collection.only.as_deref(), Some(id.as_str()));
    wait_for(&mut kit, "the second identity's only", |kit| {
        shows_only(kit, Some(&id))
    });
    kit.get_by_label("All").click();
    kit.run_steps(2);
    wait_for(&mut kit, "every NFT again", |kit| shows_only(kit, None));
}

/// With one identity the Collection has no filter: «All» and the chips are not there.
#[test]
fn one_identity_has_no_filter() {
    let (_turn, mut kit) = demo_app("collection", "en");
    wait_for(&mut kit, "every NFT", |kit| shows_only(kit, None));
    assert!(
        kit.query_by_label("All").is_some(),
        "two identities: the filter"
    );
    show_wallet_edited(
        &mut kit,
        3,
        "one identity",
        |w| w.identities.truncate(1),
        |kit| kit.query_by_label("All").is_none(),
    );
    assert!(shows_only(&kit, None), "the NFTs still all shown");
}

/// The demo's two identities: the Collection shows both identities' NFTs (FEAT-0055), the
/// second's among them.
#[test]
fn the_collection_shows_the_nfts_of_every_identity() {
    let (_turn, mut kit) = demo_app("collection", "en");
    wait_for(&mut kit, "every NFT", |kit| shows_only(kit, None));
    let owners: std::collections::HashSet<String> =
        nfts(&kit).iter().map(|n| n.owner_id.clone()).collect();
    assert!(
        owners.len() > 1,
        "NFTs of more than one identity: {owners:?}"
    );
}

/// FEAT-0055 r1: under one identity's chip its own list's state. @satoshi-lab's list is read and
/// empty while @satoshi's read failed: lab's chip says «no NFTs yet», not «could not load»;
/// «All» says the failure, with satoshi's saved NFTs.
#[test]
fn an_identitys_chip_says_its_own_lists_state() {
    let (_turn, mut kit) = demo_app("collection", "en");
    wait_for(&mut kit, "every NFT", |kit| shows_only(kit, None));
    // The demo's @satoshi-lab (`evolite_demo` LAB_IDENTITY).
    let lab = "5Kx2LabQ7pWmR9sTzV4nB8cYd1Je3HuF6gNqA2oX7wE";
    let edit = move |w: &mut evolite_api::Wallet| {
        let me = w.identities[0].id.clone();
        w.nfts.retain(|n| n.owner_id != lab);
        w.nfts_status = NftsStatus::Failed("timeout".into());
        w.nfts_statuses = vec![
            (me, NftsStatus::Failed("timeout".into())),
            (lab.to_string(), NftsStatus::Ready),
        ];
    };
    show_wallet_edited(&mut kit, 3, "satoshi's read failed", edit, |kit| {
        has_words(kit, "did not refresh from Dash Platform")
    });
    kit.get_by_label("@satoshi-lab").click();
    draw_edited(&mut kit, 3, |app| {
        if let Some(w) = app.snapshot.as_mut().and_then(|s| s.wallet.as_mut()) {
            edit(w);
        }
    });
    assert_eq!(kit.state().collection.only.as_deref(), Some(lab));
    assert!(
        has_text(&kit, "You have no NFTs yet"),
        "lab's own state: ready, empty"
    );
    assert!(
        !has_text(&kit, "Could not load the NFTs"),
        "not satoshi's failure"
    );
    assert!(
        kit.query_by_label("All").is_some(),
        "the filter stays to go back"
    );
}
