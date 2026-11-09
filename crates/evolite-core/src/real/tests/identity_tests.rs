use super::*;

#[test]
fn identity_failures_read_as_the_interface_expects() {
    assert_eq!(
        identity_failure("Insufficient funds: need 100000".into()),
        CoreError::NotEnough
    );
    assert_eq!(
        identity_failure(tr!("The wallet has no identity").into()),
        CoreError::NotAvailable(tr!("Make an identity first").into())
    );
    assert_eq!(
        identity_failure("timeout".into()),
        CoreError::Failed("timeout".into())
    );
    // BUG-7 review: a lock taken on an earlier attempt is done, said as is (not "failed").
    assert_eq!(
        identity_failure(chain::top_up_settled_text().into()),
        CoreError::NotAvailable(chain::top_up_settled_text().into())
    );
}

/// BUG-9 r1: a complete pass is done; one with slots or names missing shows what it found
/// and searches on; a failed one is told only while nothing was found.
#[test]
fn an_identity_search_is_done_only_when_complete() {
    let pass = |complete: bool| {
        Ok(chain::Discovery {
            found: 1,
            complete,
            slots_complete: complete,
            missing: (!complete).then(|| "slots".to_string()),
        })
    };
    let step = |status, found, again| DiscoveryStep {
        status,
        found,
        again,
    };
    // Complete.
    assert_eq!(
        discovery_step(pass(true), false),
        step(IdentitySearch::Done, true, false)
    );
    // Unanswered slots, or names not read: found, still searching, looking again.
    assert_eq!(
        discovery_step(pass(false), false),
        step(IdentitySearch::Searching, true, true)
    );
    assert_eq!(
        discovery_step(pass(false), true),
        step(IdentitySearch::Searching, true, true)
    );
    // Failed with nothing found yet: why; after a pass that found some: no error beside them.
    assert_eq!(
        discovery_step(Err("timeout".into()), false),
        step(IdentitySearch::Failed("timeout".into()), false, true)
    );
    assert_eq!(
        discovery_step(Err("timeout".into()), true),
        step(IdentitySearch::Searching, true, true)
    );
}

/// A lookup that is no longer current stops: locking bumps the number.
#[tokio::test]
async fn locking_ends_the_identity_lookup() {
    let mut core = core_with_vault("lookup", "pw");
    let before = core.discovery_run.load(Ordering::Relaxed);
    core.lock().await.unwrap();
    assert_ne!(core.discovery_run.load(Ordering::Relaxed), before);
    // Not unlocked: nothing to look for, the state stays as it was.
    core.discover_identities();
    assert_eq!(
        *core.identity_search.lock().unwrap(),
        IdentitySearch::Searching
    );
}

/// BUG-97: a name's preorder is charged only when the credits can also carry the domain. The
/// balance is in credits (1000 a duff), what a name needs in duffs.
#[test]
fn a_name_is_judged_by_the_credits_and_said_as_the_interface_expects() {
    const NEED: Duffs = 165_000;
    let credits = |duffs: u64| Some(duffs * 1000);
    assert_eq!(chain::name_credits_gate(credits(180_406), NEED), Ok(()));
    assert_eq!(chain::name_credits_gate(credits(165_000), NEED), Ok(()));
    let short = chain::name_credits_gate(Some(164_999_999), NEED).unwrap_err();
    assert_eq!(identity_failure(short.clone()), CoreError::NotEnough);
    assert_eq!(core_failure(short), CoreError::NotEnough);
    // The attempts of pass 12 that were charged and refused.
    assert!(chain::name_credits_gate(credits(95_285), NEED).is_err());
    assert!(chain::name_credits_gate(credits(102_105), NEED).is_err());
    // Not read: the same as not enough, nothing is sent, and it says so as it is.
    let unknown = chain::name_credits_gate(None, NEED).unwrap_err();
    assert_eq!(unknown, chain::name_credits_unknown_text());
    let reads = CoreError::NotAvailable(chain::name_credits_unknown_text().into());
    assert_eq!(identity_failure(unknown.clone()), reads);
    assert_eq!(core_failure(unknown), reads);
}

/// BUG-97, the must: nothing is broadcast when the credits cannot finish the name.
#[tokio::test]
async fn a_name_the_credits_cannot_finish_is_not_broadcast() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let sent = AtomicBool::new(false);
    let write = || async {
        sent.store(true, Ordering::Relaxed);
        Ok::<(), String>(())
    };
    let refused = chain::gated_write(Some(102_105_000), 165_000, write());
    assert!(refused.await.is_err());
    assert!(!sent.load(Ordering::Relaxed), "a refused name is not sent");
    let unknown = chain::gated_write(None, 165_000, write());
    assert!(unknown.await.is_err());
    assert!(
        !sent.load(Ordering::Relaxed),
        "unknown credits are not sent"
    );
    let ok = chain::gated_write(Some(165_000_000), 165_000, write());
    assert!(ok.await.is_ok());
    assert!(sent.load(Ordering::Relaxed), "enough credits are sent");
}

/// BUG-97: one way to a DPNS write, the gated one: a new call of the SDK next to it would pay
/// a preorder without the check.
#[test]
fn every_name_goes_through_the_credits_check() {
    // Each file of the chain module on its own, read while the test runs: a new file is
    // covered without being listed, and a test mark in one file cuts only that file.
    let dir = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../evolite-chain/src/spv/chain"
    );
    let (mut sdk_calls, mut gated_calls) = (0, 0);
    for entry in std::fs::read_dir(dir).expect("the chain module's folder") {
        let path = entry.expect("a file of the chain module").path();
        let is_code = path.extension().and_then(|e| e.to_str()) == Some("rs")
            && path.file_name().and_then(|n| n.to_str()) != Some("tests.rs");
        if !is_code {
            continue;
        }
        let source = std::fs::read_to_string(&path).expect("the file reads");
        let code = source.split("\n#[cfg(test)]").next().unwrap_or(&source);
        sdk_calls += code.matches("register_name_with_external_signer(").count();
        gated_calls += code.matches("gated_write(").count();
    }
    assert_eq!(
        sdk_calls, 1,
        "one call of the SDK's name registration, inside `write_name`"
    );
    assert_eq!(gated_calls, 1, "its one call");
}
