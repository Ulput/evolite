//! DashPay on platform-wallet: the contact crypto from the open wallet's keys, and what the
//! interface shows of contacts and profiles.
//!
//! platform-wallet keeps no private keys, so DIP-15's key work (ECDH for the encrypted xpub,
//! account references, contactInfo) is done here, from the keys derived from the phrase while
//! the wallet is open, through its [`ContactCryptoProvider`].

use std::collections::HashMap;

use async_trait::async_trait;
use dpp::platform_value::string_encoding::Encoding;
use dpp::prelude::Identifier;
use evolite_api::{Activity, ActivityKind, Contact, ContactState, Profile, Timestamp};
use key_wallet::bip32::{ChildNumber, DerivationPath, ExtendedPubKey};
use platform_wallet::{
    ContactCryptoProvider, ContactInfoOpened, ContactInfoSealed, DashPayProfile, ManagedIdentity,
    PlatformWalletError,
};
use zeroize::Zeroizing;

/// DashPay crypto from the open wallet's keys.
pub struct SeedCrypto(pub key_wallet::Wallet);

#[allow(
    clippy::result_large_err,
    reason = "PlatformWalletError is the error type ContactCryptoProvider requires; these helpers feed it"
)]
impl SeedCrypto {
    fn xprv(&self, path: &DerivationPath) -> Result<Zeroizing<[u8; 32]>, PlatformWalletError> {
        let x = self
            .0
            .derive_extended_private_key(path)
            .map_err(|e| PlatformWalletError::InvalidIdentityData(e.to_string()))?;
        Ok(Zeroizing::new(*x.private_key.to_bytes()))
    }

    fn secret(
        &self,
        path: &DerivationPath,
    ) -> Result<dpp::secp256k1::SecretKey, PlatformWalletError> {
        dpp::secp256k1::SecretKey::from_secret_bytes(*self.xprv(path)?)
            .map_err(|e| PlatformWalletError::KeyDerivation(e.to_string()))
    }

    fn contact_info_child(
        &self,
        root: &DerivationPath,
        index: u32,
        enc: bool,
    ) -> Result<Zeroizing<[u8; 32]>, PlatformWalletError> {
        let feature = if enc {
            key_wallet::dip9::DASHPAY_CONTACT_INFO_ENC_TO_USER_ID_CHILD
        } else {
            key_wallet::dip9::DASHPAY_CONTACT_INFO_PRIVATE_DATA_CHILD
        };
        let child = |i| {
            ChildNumber::from_hardened_idx(i)
                .map_err(|e| PlatformWalletError::InvalidIdentityData(e.to_string()))
        };
        let path = root.clone().extend([child(feature)?, child(index)?]);
        self.xprv(&path)
    }
}

#[async_trait]
#[allow(
    clippy::result_large_err,
    reason = "PlatformWalletError is the error type ContactCryptoProvider requires"
)]
impl ContactCryptoProvider for SeedCrypto {
    async fn receiving_xpub(
        &self,
        path: &DerivationPath,
    ) -> Result<ExtendedPubKey, PlatformWalletError> {
        self.0
            .derive_extended_public_key(path)
            .map_err(|e| PlatformWalletError::InvalidIdentityData(e.to_string()))
    }

    async fn ecdh_shared_secret(
        &self,
        path: &DerivationPath,
        peer: &dpp::secp256k1::PublicKey,
    ) -> Result<Zeroizing<[u8; 32]>, PlatformWalletError> {
        Ok(Zeroizing::new(platform_encryption::derive_shared_key_ecdh(
            &self.secret(path)?,
            peer,
        )))
    }

    async fn export_auto_accept_private_key(
        &self,
        path: &DerivationPath,
    ) -> Result<dpp::secp256k1::SecretKey, PlatformWalletError> {
        self.secret(path)
    }

    async fn export_invitation_private_key(
        &self,
        path: &DerivationPath,
    ) -> Result<dpp::secp256k1::SecretKey, PlatformWalletError> {
        self.secret(path)
    }

    async fn account_reference(
        &self,
        path: &DerivationPath,
        compact_xpub: &[u8],
        account_index: u32,
        version: u32,
    ) -> Result<u32, PlatformWalletError> {
        Ok(platform_encryption::calculate_account_reference(
            &*self.xprv(path)?,
            compact_xpub,
            account_index,
            version,
        ))
    }

    async fn unmask_account_reference(
        &self,
        path: &DerivationPath,
        compact_xpub: &[u8],
        account_reference: u32,
    ) -> Result<(u32, u32), PlatformWalletError> {
        Ok(platform_encryption::unmask_account_reference(
            account_reference,
            &*self.xprv(path)?,
            compact_xpub,
        ))
    }

    async fn contact_info_seal(
        &self,
        root_path: &DerivationPath,
        derivation_index: u32,
        contact_id: &[u8; 32],
        plaintext: &[u8],
        iv: &[u8; 16],
    ) -> Result<ContactInfoSealed, PlatformWalletError> {
        let enc = self.contact_info_child(root_path, derivation_index, true)?;
        let private = self.contact_info_child(root_path, derivation_index, false)?;
        Ok(ContactInfoSealed {
            enc_to_user_id: platform_encryption::encrypt_enc_to_user_id(&enc, contact_id),
            private_data: platform_encryption::encrypt_private_data(&private, iv, plaintext),
        })
    }

    async fn contact_info_open(
        &self,
        root_path: &DerivationPath,
        derivation_index: u32,
        enc_to_user_id: &[u8; 32],
        blob: &[u8],
    ) -> Result<ContactInfoOpened, PlatformWalletError> {
        let enc = self.contact_info_child(root_path, derivation_index, true)?;
        let private = self.contact_info_child(root_path, derivation_index, false)?;
        Ok(ContactInfoOpened {
            contact_id: platform_encryption::decrypt_enc_to_user_id(&enc, enc_to_user_id),
            private_data: platform_encryption::decrypt_private_data(&private, blob)
                .map_err(|e| PlatformWalletError::InvalidIdentityData(e.to_string()))?,
        })
    }
}

pub fn base58(id: &Identifier) -> String {
    id.to_string(Encoding::Base58)
}

/// The wallet's history (txid and operation) with the DashPay contact named on payments to and
/// from contacts: "@alice", or a short id for one without a name.
pub fn name_payments(
    managed: &ManagedIdentity,
    names: &HashMap<String, Option<String>>,
    records: Vec<(String, Activity)>,
) -> Vec<(String, Activity)> {
    let payees: HashMap<&str, String> = managed
        .dashpay()
        .payments
        .iter()
        .map(|(txid, entry)| {
            let id = base58(&entry.counterparty_id);
            let shown = Contact {
                name: names.get(&id).cloned().flatten(),
                identity_id: id,
                profile: Profile::default(),
                state: ContactState::Established,
                since: 0,
                payments: 0,
                last_payment: None,
            }
            .handle();
            (txid.as_str(), shown)
        })
        .collect();
    name_history(&payees, records)
}

/// Names the counterparty of the operations whose txid `payees` knows, unless the operation
/// already says who.
fn name_history(
    payees: &HashMap<&str, String>,
    mut records: Vec<(String, Activity)>,
) -> Vec<(String, Activity)> {
    for (txid, act) in &mut records {
        let Some(payee) = payees.get(txid.as_str()) else {
            continue;
        };
        match &mut act.kind {
            ActivityKind::Received { from: from @ None } => *from = Some(payee.clone()),
            ActivityKind::Sent { to } => *to = payee.clone(),
            _ => {}
        }
    }
    records
}

/// Text someone else wrote (a profile, an NFT), as it may be shown: without control characters
/// (but line breaks), the marks and overrides that change text direction, and invisible
/// characters. With them a name could show as something else — `@alice` written backwards
/// behind an override — once the interface reorders text with Arabic in it.
pub fn shown(text: &str) -> String {
    text.chars()
        .filter(|&c| {
            c == '\n'
                || !(c.is_control()
                    || matches!(
                        c,
                        '\u{061c}'
                            | '\u{200b}'..='\u{200f}'
                            | '\u{202a}'..='\u{202e}'
                            | '\u{2060}'..='\u{2069}'
                            | '\u{feff}'
                    ))
        })
        .collect()
}

/// What the interface shows of a DashPay profile. `avatar`: the key of its picture, when the
/// core has the bytes and they match the profile's hash.
pub fn profile_of(p: &DashPayProfile, avatar: Option<String>) -> Profile {
    let text = |s: &Option<String>| s.as_deref().map(shown).filter(|s| !s.trim().is_empty());
    Profile {
        display: text(&p.display_name),
        // DashPay's "publicMessage" is what the profile says about the person; `bio` is an
        // older field some clients write.
        about: text(&p.public_message).or_else(|| text(&p.bio)),
        avatar,
        avatar_url: text(&p.avatar_url),
        updated_at: None,
    }
}

/// Whether a contact's picture is downloaded. Not for a request someone sent us: until it is
/// accepted the sender is a stranger, and fetching their picture would tell their server this
/// computer's address and when the wallet is online — for the price of one request.
pub fn fetches_picture(state: ContactState) -> bool {
    state != ContactState::Incoming
}

/// The identity's contacts: requests both ways and the contacts made, newest first; requests
/// from hidden senders left out. `names`: DPNS labels known for identities; `avatar`: the
/// picture key for an identity, when the core has it; `paid_at`: when the wallet's history has
/// the transaction with this txid (DashPay keeps payments by txid, without a time).
pub fn contacts_of(
    managed: &ManagedIdentity,
    names: &HashMap<String, Option<String>>,
    avatar: impl Fn(&Identifier, &DashPayProfile) -> Option<String>,
    paid_at: impl Fn(&str) -> Option<Timestamp>,
) -> Vec<Contact> {
    let dashpay = managed.dashpay();
    let ms = |ms: u64| (ms / 1000) as i64;
    let profile = |id: &Identifier, state: ContactState| {
        dashpay
            .contact_profiles
            .get(id)
            .and_then(|entry| entry.profile.as_ref())
            .map(|p| {
                let picture = fetches_picture(state).then(|| avatar(id, p)).flatten();
                profile_of(p, picture)
            })
            .unwrap_or_default()
    };
    // Both ways: how many, and the latest the history has a time for.
    let payments = |id: &Identifier| {
        let mut count = 0;
        let mut last = None;
        for (txid, entry) in &dashpay.payments {
            if entry.counterparty_id == *id {
                count += 1;
                last = last.max(paid_at(txid));
            }
        }
        (count, last)
    };
    let mut out = Vec::new();
    let mut add = |id: &Identifier, state, since: u64| {
        let key = base58(id);
        let (payments, last_payment) = if state == ContactState::Established {
            payments(id)
        } else {
            (0, None)
        };
        out.push(Contact {
            name: names.get(&key).cloned().flatten(),
            identity_id: key,
            profile: profile(id, state),
            state,
            since: ms(since),
            payments,
            last_payment,
        });
    };
    for (id, contact) in dashpay.established_contacts() {
        if contact.is_hidden {
            continue;
        }
        let since = contact
            .outgoing_request
            .created_at
            .min(contact.incoming_request.created_at);
        add(id, ContactState::Established, since);
    }
    for (id, request) in dashpay.sent_contact_requests() {
        if !dashpay.established_contacts().contains_key(id) {
            add(id, ContactState::Outgoing, request.created_at);
        }
    }
    for (id, request) in dashpay.incoming_contact_requests() {
        if !dashpay.established_contacts().contains_key(id) && !managed.is_sender_ignored(id) {
            add(id, ContactState::Incoming, request.created_at);
        }
    }
    out.sort_by_key(|c| std::cmp::Reverse(c.since));
    out
}

/// Where `to` stands among `managed`'s contacts in platform-wallet's state at this moment —
/// what a new contact request is checked against (BUG-13), not a snapshot of whichever
/// identity was shown when it was made. Hiding is not looked at: a hidden contact made is
/// still among the established ones, and a hidden request to us is `Incoming` — ours may go.
pub fn standing(managed: &ManagedIdentity, to: &Identifier) -> Option<ContactState> {
    let dashpay = managed.dashpay();
    standing_of(
        dashpay.established_contacts().contains_key(to),
        dashpay.sent_contact_requests().contains_key(to),
        dashpay.incoming_contact_requests().contains_key(to),
    )
}

/// The contact made, else our request sent, else theirs; `None`: no request either way.
fn standing_of(established: bool, sent: bool, incoming: bool) -> Option<ContactState> {
    if established {
        Some(ContactState::Established)
    } else if sent {
        Some(ContactState::Outgoing)
    } else if incoming {
        Some(ContactState::Incoming)
    } else {
        None
    }
}

/// A `profile` document of the DashPay contract, as platform-wallet reads one.
pub fn profile_from_document(doc: &dpp::document::Document) -> DashPayProfile {
    use dpp::document::DocumentV0Getters;
    use dpp::platform_value::Value;
    let props = doc.properties();
    let text = |key: &str| {
        props
            .get(key)
            .and_then(|v: &Value| v.as_str().map(ToString::to_string))
            .filter(|s| !s.is_empty())
    };
    let bytes = |key: &str| props.get(key).and_then(|v: &Value| v.as_bytes_slice().ok());
    let public_message = text("publicMessage");
    DashPayProfile {
        display_name: text("displayName"),
        bio: public_message.clone(),
        avatar_url: text("avatarUrl"),
        avatar_hash: bytes("avatarHash").and_then(|b| <[u8; 32]>::try_from(b).ok()),
        avatar_fingerprint: bytes("avatarFingerprint").and_then(|b| <[u8; 8]>::try_from(b).ok()),
        public_message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolite_api::{Chain, Proof};

    /// BUG-13: a request is held back by what platform-wallet has for the sending identity.
    #[test]
    fn a_contact_stands_by_the_strongest_tie() {
        assert_eq!(
            standing_of(true, true, true),
            Some(ContactState::Established)
        );
        assert_eq!(standing_of(false, true, true), Some(ContactState::Outgoing));
        assert_eq!(
            standing_of(false, false, true),
            Some(ContactState::Incoming)
        );
        assert_eq!(standing_of(false, false, false), None);
    }

    fn op(kind: ActivityKind) -> Activity {
        Activity {
            id: 1,
            kind,
            chain: Chain::Core,
            at: 0,
            amount: 1,
            proof: Proof::ChainLock,
            txid: None,
            fee: None,
            block: None,
            address: None,
            layers: evolite_api::layer::CORE,
            flow: evolite_api::Flow::Out,
            key: String::new(),
            note: None,
        }
    }

    #[test]
    fn a_strangers_picture_waits_until_the_request_is_accepted() {
        assert!(!fetches_picture(ContactState::Incoming));
        assert!(fetches_picture(ContactState::Established));
        assert!(
            fetches_picture(ContactState::Outgoing),
            "we asked them ourselves"
        );
    }

    #[test]
    fn someone_elses_text_loses_direction_tricks_and_invisible_characters() {
        // "@alice" written backwards behind a right-to-left override, with an Arabic letter
        // that makes the interface reorder the line.
        let spoof = "\u{202e}ecila@\u{202c} \u{0628}";
        assert_eq!(shown(spoof), "ecila@ \u{0628}");
        assert_eq!(shown("a\u{200b}l\u{2066}i\u{feff}ce\u{0007}"), "alice");
        assert_eq!(shown("line one\nline two\t"), "line one\nline two");
        assert_eq!(shown("Grüße, 世界 🎉"), "Grüße, 世界 🎉");
        let profile = DashPayProfile {
            display_name: Some("\u{202e}\u{200f}".into()),
            public_message: Some("hi\u{202e}".into()),
            ..Default::default()
        };
        let shown_profile = profile_of(&profile, None);
        assert_eq!(shown_profile.display, None, "only tricks: nothing to show");
        assert_eq!(shown_profile.about.as_deref(), Some("hi"));
    }

    #[test]
    fn names_payments_to_and_from_contacts() {
        let payees = HashMap::from([("aa", "@alice".to_string()), ("bb", "@bob".to_string())]);
        let named = name_history(
            &payees,
            vec![
                ("aa".into(), op(ActivityKind::Received { from: None })),
                ("bb".into(), op(ActivityKind::Sent { to: String::new() })),
                ("cc".into(), op(ActivityKind::Sent { to: String::new() })),
            ],
        );
        assert_eq!(
            named[0].1.kind,
            ActivityKind::Received {
                from: Some("@alice".into())
            }
        );
        assert_eq!(named[1].1.kind, ActivityKind::Sent { to: "@bob".into() });
        assert_eq!(
            named[2].1.kind,
            ActivityKind::Sent { to: String::new() },
            "not a contact's"
        );
    }
}
