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
