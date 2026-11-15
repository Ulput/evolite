//! FEAT-0020: a change of an identity's keys refused before the password, in words — the same
//! from the real core and the demo (R-2, R-3, AC-T2).

use evolite_api::key_plan::KeyRefusal;

pub fn words(refusal: &KeyRefusal) -> String {
    match refusal {
        KeyRefusal::Nothing => tr!("Nothing to change").into(),
        KeyRefusal::TooManyAdded => tr!("At most 6 keys can be added at once").into(),
        KeyRefusal::TooManyDisabled => tr!("At most 10 keys can be disabled at once").into(),
        KeyRefusal::NoSuchKey(id) => {
            format!(tr!("Key {} is not among the identity's acting keys"), id)
        }
        KeyRefusal::MasterOnlyReplaced => tr!(
            "The main key cannot be disabled: without it nobody manages the keys. It can only be replaced"
        )
        .into(),
        KeyRefusal::MasterNotThisWallets => tr!(
            "This identity's main key is from another wallet: the keys can be changed only there"
        )
        .into(),
        KeyRefusal::DashPayOnlyReplaced => tr!(
            "DashPay's encryption keys are not disabled one by one: replace the DashPay pair"
        )
        .into(),
        KeyRefusal::OtherEncryptionKeys => tr!(
            "First disable the apps' encryption keys: DashPay would take one of them instead of the new one"
        )
        .into(),
        KeyRefusal::AlreadyHas(_) => tr!("The identity already has such a key").into(),
    }
}
