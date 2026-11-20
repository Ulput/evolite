//! BUG-243: «not seen» right after our own write, or its error, is «not known» — kept in
//! [`sent::Unclear`], and a paid repeat waits its quiet; it is never decided by one read.
//!
//! Each site: the write ends with an error that may have come after it went out, the read after
//! it says «no» (a node a few blocks behind), the person tries again — the guard refuses, no
//! second paid write; a later read says «yes» and takes it out. Then the text of each site: its
//! guard comes before its write, and its error is kept, not decided. Mutant per site: the guard
//! removed, or the error answered by the read alone («one read decides») — red.

use super::*;
use crate::real::ops::identity::{claims, links};

/// The sequence the architect asks for, over the real `Unclear`: a paid write whose error may
/// have come after it went out; the read after it says «no». Returns how many paid writes went
/// out over two attempts, the second one inside the quiet.
fn paid_writes(
    watch: sent::Watch,
    kind: Option<journal::Kind>,
    guard: impl Fn(&sent::Unclear) -> bool,
) -> usize {
    let unclear = sent::Unclear::default();
    let mut writes = 0;
    for _attempt in 0..2 {
        if guard(&unclear) {
            continue;
        }
        writes += 1;
        // The send ended with an error that may have come after it went out, and the read
        // after it does not show the write: not known.
        let seen = false;
        if !seen {
            let _ = unclear.not_seen(sent::Pending::new(
                "me".into(),
                watch.clone(),
                kind.clone(),
                Some(10),
                Some("me".into()),
            ));
        }
    }
    // A later read shows it: taken out, once.
    let shown = unclear.take_if("me", |w| {
        std::mem::discriminant(w) == std::mem::discriminant(&watch)
    });
    assert!(shown.is_some(), "kept until a read shows it");
    writes
}

#[test]
fn a_name_claimed_and_not_seen_is_not_claimed_again() {
    let watch = sent::Watch::Name {
        label: "alice".into(),
    };
    let writes = paid_writes(watch, None, |u| {
        u.on_their_way("me", |p| claims(&p.watch, "Alice"))
    });
    assert_eq!(writes, 1, "(1, 4) preorder, domain and fund once");
}

#[test]
fn a_name_link_not_seen_is_not_created_again() {
    let watch = sent::Watch::Link {
        label: "alice".into(),
    };
    let writes = paid_writes(watch, None, |u| {
        u.on_their_way("me", |p| links(&p.watch, "alice"))
    });
    assert_eq!(writes, 1, "(2) one identityVerify document");
}

#[test]
fn app_connect_keys_not_seen_are_not_added_again() {
    let kind = journal::Kind::AppConnect {
        app: "app".into(),
        renewal: false,
    };
    let writes = paid_writes(sent::Watch::Answer, Some(kind), |u| {
        u.keys_on_their_way("me")
    });
    assert_eq!(writes, 1, "(3) the keys once");
}

#[test]
fn keys_changed_and_not_seen_hold_the_next_change() {
    let plan = evolite_api::key_plan::KeyPlan::default();
    let writes = paid_writes(sent::Watch::Keys(plan), None, |u| u.keys_on_their_way("me"));
    assert_eq!(writes, 1, "(5) no plan on the old keys");
}

#[test]
fn a_contact_request_not_seen_is_not_sent_again() {
    // As the resend keeps its own (FEAT-0020): any request of the identity waits.
    let writes = paid_writes(sent::Watch::ContactRequest, None, |u| {
        u.requests_on_their_way("me")
    });
    assert_eq!(writes, 1, "(6) one request, no key rotation");
}

#[test]
fn a_profile_not_seen_is_not_created_again() {
    let writes = paid_writes(sent::Watch::Profile, None, |u| {
        u.on_their_way("me", |p| matches!(p.watch, sent::Watch::Profile))
    });
    assert_eq!(writes, 1, "(6) one profile");
}

/// The mutant «one read decides»: without the keep, the same sequence pays twice.
#[test]
fn deciding_by_one_read_would_pay_twice() {
    let unclear = sent::Unclear::default();
    let mut writes = 0;
    for _ in 0..2 {
        if unclear.on_their_way("me", |p| claims(&p.watch, "alice")) {
            continue;
        }
        writes += 1;
        // «not seen» taken as «not done»: nothing kept.
    }
    assert_eq!(writes, 2);
}

/// `source`'s function `name`, to its end (the next top-level `fn` or `pub` item).
fn body<'a>(source: &'a str, name: &str) -> &'a str {
    let at = source.find(name).unwrap_or_else(|| panic!("{name}"));
    &source[at..]
}

/// In `f`, `first` comes before `then`.
fn before(f: &str, first: &str, then: &str, what: &str) {
    let a = f
        .find(first)
        .unwrap_or_else(|| panic!("{what}: no «{first}»"));
    let b = f
        .find(then)
        .unwrap_or_else(|| panic!("{what}: no «{then}»"));
    assert!(a < b, "{what}: «{first}» after «{then}»");
}

/// Each site's text: the guard before the write, and the write's error kept as not known.
#[test]
fn every_site_guards_before_its_write_and_keeps_what_it_did_not_see() {
    let identity = include_str!("../ops/identity.rs");
    let claim = body(identity, "fn claim_name(");
    before(
        claim,
        "unclear.on_their_way(&shown, |p| claims(",
        ".add_name(",
        "(1) claim",
    );
    before(claim, ".add_name(", "unclear.not_seen(", "(1) claim kept");
    let plain = body(identity, "fn add_name(");
    before(
        plain,
        "unclear.on_their_way(&acting, |p| claims(",
        ".add_name(",
        "(1) name",
    );
    let link = body(identity, "fn set_name_link(");
    before(
        link,
        "unclear.on_their_way(&shown, |p| links(",
        ".write_identity_link(",
        "(2) link",
    );
    before(
        link,
        ".write_identity_link(",
        "unclear.not_seen(",
        "(2) link kept",
    );
    let resume = body(identity, "fn resume_identity(");
    before(resume, "name_may_land(", ".resume_identity(", "(4) resume");
    before(
        resume,
        ".resume_identity(",
        "name_not_seen(",
        "(4) resume kept",
    );
    let create = body(identity, "fn create_identity(");
    before(
        create,
        ".create_identity(",
        "name_not_seen(",
        "(4) create kept",
    );
    let unfinished = body(identity, "fn unfinished_identity(");
    before(
        unfinished,
        ".on_their_way(",
        "identity: made.map(",
        "(4) unfinished",
    );

    let apps = include_str!("../ops/app_connect2.rs");
    let approve = body(apps, "fn approve_connect2(");
    before(
        approve,
        "!all_on(&added, &fresh.keys)",
        "unclear.not_seen(keys_not_seen(",
        "(3) keys kept",
    );

    let keys = include_str!("../ops/identity_keys.rs");
    let update = body(keys, "fn update_keys(");
    before(
        update,
        "self.unclear.keys_on_their_way(&identity)",
        "sent::run(",
        "(5) guard",
    );
    before(
        update,
        "key_update::shows(&after_send, &plan)",
        "unclear.not_seen(stale_keys(",
        "(5) kept",
    );

    let dashpay = include_str!("../ops/dashpay.rs");
    let contact = body(dashpay, "fn add_contact(");
    before(
        contact,
        "unclear.requests_on_their_way(&me)",
        ".add_contact(",
        "(6) contact",
    );
    before(
        contact,
        ".add_contact(",
        "contact_not_seen(",
        "(6) contact kept",
    );
    let accept = body(dashpay, "fn contact_job(");
    before(
        accept,
        "unclear.requests_on_their_way(&mine)",
        ".accept_contact(",
        "(6) accept",
    );
    let profile = body(dashpay, "fn save_profile(");
    before(
        profile,
        "sent::Watch::Profile))",
        ".save_profile(",
        "(6) profile",
    );
    before(
        profile,
        ".save_profile(",
        "unclear.not_seen(",
        "(6) profile kept",
    );
}
