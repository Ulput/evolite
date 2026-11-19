//! Д11 (audit 2026-11-19): every place that waits for the core about a name says the same —
//! «Checking», then after a while that the network is slow (BUG-87) — and Send does not ask the
//! core about a name shorter than DPNS allows.

use super::*;

/// Long enough ago that the words say the network is slow.
fn long_ago() -> Option<std::time::Instant> {
    std::time::Instant::now()
        .checked_sub(crate::text::NAME_SLOW_AFTER + std::time::Duration::from_secs(1))
}

/// «Add a name»: a check long in coming says it is the network, not «Checking» for ever.
#[test]
fn add_name_says_the_network_is_slow() {
    let (_turn, mut kit) = demo_app("add-name", "en");
    {
        let form = kit
            .state_mut()
            .identity
            .adding_name
            .as_mut()
            .expect("the form");
        form.name = "slow-name".into();
        form.asked = "slow-name".into();
        form.check = None;
        form.asked_at = long_ago();
    }
    kit.run_steps(3);
    assert!(has_words(&kit, "The network is slow to answer"));
    assert!(!has_words(&kit, "Checking the name in DPNS"));
}

/// Creating an identity: the same words for its name.
#[test]
fn create_identity_says_the_network_is_slow() {
    let (_turn, mut kit) = demo_app("create-identity", "en");
    {
        let form = &mut kit.state_mut().create_identity;
        form.name = "slow-name".into();
        form.asked = "slow-name".into();
        form.check = None;
        form.asked_at = long_ago();
    }
    kit.run_steps(3);
    assert!(has_words(&kit, "The network is slow to answer"));
}

/// Send: «@ab» says it is not a Dash name and asks the core nothing; «@» alone says nothing.
#[test]
fn send_does_not_ask_about_a_name_too_short() {
    let (_turn, mut kit) = demo_app("send-book", "en");
    kit.state_mut().send.recipient = "@ab".into();
    kit.run_steps(3);
    assert!(has_words(&kit, "This is not a Dash name"));
    assert!(!has_words(&kit, "Checking the name in DPNS"));
    assert_eq!(kit.state().send.asked, "", "nothing asked of the core");
    kit.state_mut().send.recipient = "@".into();
    kit.run_steps(3);
    assert!(!has_words(&kit, "This is not a Dash name"));
    kit.state_mut().send.recipient = "@abc".into();
    kit.run_steps(3);
    assert_eq!(kit.state().send.asked, "@abc", "three characters are asked");
}
