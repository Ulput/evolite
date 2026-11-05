//! What the mock answers by rules, as the real core will: DPNS names (taken, contested,
//! free) and app links (dash-key:, dash-st:, dash-kx:).

use evolite_api::*;

/// Someone the mock "finds" in DPNS, with their DashPay profile.
pub(super) struct Person {
    pub name: &'static str,
    pub display: Option<&'static str>,
    pub about: Option<&'static str>,
    pub id: &'static str,
}

/// The people of the Figma screens; the first is the wallet's own identity.
pub(super) const PEOPLE: [Person; 6] = [
    Person {
        name: "satoshi",
        display: Some("Satoshi Nakamoto"),
        about: Some(tr!(
            "I write about Dash Platform and collect generative NFTs. Pay through a DashPay contact — that way each payment goes to a new address."
        )),
        id: "8rd3Ux7pQfZ2cW9yLmT4aB6nE1sK5gH3dR7vXkV2mJ38R",
    },
    Person {
        name: "alice",
        display: Some("Alice Chen"),
        about: Some(tr!(
            "I draw illustrations and take payment in DASH. Write to me on Jembe: @alice."
        )),
        id: "3Ah7Kp2Qx9Lm4Vt8Rn5Wc1Jd6Yb2Fs7Ge4Hu3Mz9Lp1",
    },
    Person {
        name: "bob",
        display: Some("Bob"),
        about: Some(tr!(
            "I run a masternode on testnet and write about how it works."
        )),
        id: "4Fq9Lm2Rt8Vx3Kc7Hn1Pw5Zs6Jd9Gb4Ya2pX2a",
    },
    Person {
        name: "dash-core-dev",
        display: None,
        about: None,
        id: "6Hk2Ft9wQmRz8LpXcV4nB7sYd3Ke1JaPu5q9W5",
    },
    Person {
        name: "kate",
        display: Some("Kate"),
        about: Some(tr!("Learning to write Rust.")),
        id: "2Lx8Vb5Nq3Rt7Wm1Kc9Hd4Fj6Ys2Pg8Tr4N",
    },
    // Not a contact: for "Add a contact".
    Person {
        name: "carol",
        display: Some("Carol Diaz"),
        about: Some(tr!(
            "I make games on Dash Platform. Write if you need a tester."
        )),
        id: "9Pw3Ex6Rb2Kn8Vq4Lt1Hc7Ys5Mz3Fd9Ga2Ju6N",
    },
];

impl Person {
    pub fn profile(&self) -> Profile {
        Profile {
            display: self.display.map(Into::into),
            about: self.about.map(Into::into),
            avatar: None,
            avatar_url: None,
            updated_at: None,
        }
    }
}

/// The demo contacts as on the Figma screen "19 · Contacts": a request from @bob, @alice and
/// @dash-core-dev made, ours to @kate not answered yet.
pub(super) fn demo_contacts(now: Timestamp) -> Vec<Contact> {
    let day = 24 * 3600;
    let contact = |i: usize, state, since, payments, last: Option<Timestamp>| Contact {
        identity_id: PEOPLE[i].id.into(),
        name: Some(PEOPLE[i].name.into()),
        profile: PEOPLE[i].profile(),
        state,
        since: now - since,
        payments,
        last_payment: last.map(|ago| now - ago),
    };
    vec![
        contact(2, ContactState::Incoming, 3 * 3600, 0, None),
        contact(1, ContactState::Established, 18 * day, 4, Some(day)),
        contact(3, ContactState::Established, 27 * day, 1, Some(3 * day)),
        contact(4, ContactState::Outgoing, 2 * 3600, 0, None),
    ]
}

/// What the mock answers about `name`: the person with their profile and where they stand
/// among `contacts`.
pub(super) fn resolve(name: &str, contacts: &[Contact]) -> Result<ResolvedName, CoreError> {
    let label = crate::dpns::label(name)?;
    let person = PEOPLE
        .iter()
        .find(|p| p.name == label)
        .ok_or(CoreError::NameNotFound)?;
    Ok(ResolvedName {
        name: person.name.into(),
        display: person.display.map(Into::into),
        identity_id: person.id.into(),
        profile: Some(person.profile()),
        contact: contacts
            .iter()
            .find(|c| c.identity_id == person.id)
            .map(|c| c.state),
    })
}

/// Where the mock says a Core payment to `to` goes: an address stays itself, a contact gets an
/// example address of its key (the Figma example has no real keys).
pub(super) fn preview(to: &str, contacts: &[Contact]) -> Result<String, CoreError> {
    let to = to.trim();
    if !to.starts_with('@') {
        return Ok(to.to_owned());
    }
    let person = resolve(to, contacts)?;
    if person.contact != Some(ContactState::Established) {
        return Err(CoreError::NotAvailable(format!(
            tr!(
                "@{} is not in your contacts: a Core payment by name goes only to a DashPay contact"
            ),
            person.name
        )));
    }
    Ok("yRk7Hc2pWq9mN4vT8sLb3XeJ6uZa1Df5Gh".into())
}

/// Names the mock treats as taken in DPNS.
const TAKEN: [&str; 4] = ["satoshi", "alice", "bob", "nakamoto-dev"];

/// The DPNS rules of [`crate::dpns`], with the names above taken.
pub(super) fn check_name(label: &str) -> Result<NameCheck, CoreError> {
    let label = crate::dpns::label(label)?;
    if TAKEN.contains(&label.as_str()) {
        return Ok(NameCheck::Taken);
    }
    Ok(if crate::dpns::is_contested(&label) {
        NameCheck::Contested
    } else {
        NameCheck::Available
    })
}

/// What an app link asks, decoded as the real core would (the examples from Figma 11–13).
pub(super) fn app_request(id: u64, text: &str, wallet: &Wallet) -> Result<AppRequest, CoreError> {
    let identity = wallet
        .identity
        .as_ref()
        .and_then(|i| i.name())
        .map(|n| format!("@{n}"))
        .ok_or(CoreError::NotAvailable(
            tr!("An identity is needed first").into(),
        ))?;
    let text = text.trim();
    let kind = if text.starts_with("dash-key:") {
        AppRequestKind::Connect {
            contract_id: "6Hk2Ft9wQmRz8LpXcV4nB7sYd3Ke1JaPu5W9q".into(),
            contract_found: true,
            identity,
            // A new key: the next after the identity's #0–#4.
            key_id: 5,
            level: KeyLevel::Medium,
            contract_name: "Yappr".into(),
            cost: wallet.fees.add_key,
        }
    } else if text.starts_with("dash-st:") {
        AppRequestKind::Sign {
            action: tr!("Create a document").into(),
            contract: "jembe".into(),
            contract_id: "9xQeR4mT7wZp2KcV8nB3sYd6Lf1JaHu5L4tN".into(),
            document_type: "jam".into(),
            identity,
            key_id: 4,
            level: KeyLevel::High,
            fields: vec![(
                "text".into(),
                tr!("“The first jam from evolite. Signed on a computer, checked by light.”").into(),
            )],
            cost: 4_000,
            credits_after: wallet.balances.credits - 4_000,
            bytes_hex: "0201a4c3…e9f0".into(),
        }
    } else if text.starts_with("dash-kx:") {
        AppRequestKind::KeyExchange {
            identity,
            device: tr!("Jembe on the phone").into(),
            code: "482917".into(),
            channel_encrypted: true,
            request_signed: true,
        }
    } else {
        return Err(CoreError::BadLink);
    };
    let app = match &kind {
        AppRequestKind::Connect { .. } => "Yappr",
        _ => "Jembe",
    };
    Ok(AppRequest {
        id,
        app: app.into(),
        kind,
    })
}
