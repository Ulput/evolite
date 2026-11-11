//! What the mock answers by rules, as the real core will: DPNS names (taken, contested,
//! free) and app links (App Connect v2; v1 refused).

use evolite_api::*;

/// Someone the mock "finds" in DPNS, with their DashPay profile.
pub struct Person {
    pub name: &'static str,
    pub display: Option<&'static str>,
    pub about: Option<&'static str>,
    pub id: &'static str,
}

/// The people of the Figma screens; the first is the wallet's own identity.
pub const PEOPLE: [Person; 6] = [
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
pub fn demo_contacts(now: Timestamp) -> Vec<Contact> {
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
pub fn resolve(name: &str, contacts: &[Contact]) -> Result<ResolvedName, CoreError> {
    let label = evolite_common::dpns::label(name)?;
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
pub fn preview(to: &str, contacts: &[Contact]) -> Result<String, CoreError> {
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

/// The DPNS rules of [`evolite_common::dpns`], with the names above taken.
pub fn check_name(label: &str) -> Result<NameCheck, CoreError> {
    let label = evolite_common::dpns::label(label)?;
    if TAKEN.contains(&label.as_str()) {
        return Ok(NameCheck::Taken);
    }
    Ok(if evolite_common::dpns::is_contested(&label) {
        NameCheck::Contested
    } else {
        NameCheck::Available
    })
}

/// Yappr's App Connect v2 request as its profile describes it (FEAT-0023): its contract group
/// at CRITICAL for 0.5 DASH and 90 days, DashPay `profile` at HIGH for 0.01 DASH and 90 days,
/// and an encryption key of its own contract.
/// `renews`: a sign-in again with the connection of the demo's Yappr (R2).
fn connect2_example(identity: String, wallet: &Wallet, renews: bool) -> AppRequestKind {
    let day = 24 * 60 * 60;
    let end = evolite_common::now() + 90 * day;
    let offer = |binding, level, budget, primary| SessionOffer {
        binding,
        level,
        proposed_budget: Some(budget),
        proposed_expiry: Some(end),
        budget,
        expires_at: end,
        changed: false,
        primary,
    };
    AppRequestKind::Connect2 {
        app_id: "6Hk2Ft9wQmRz8LpXcV4nB7sYd3Ke1JaPu5W9q".into(),
        label: "Yappr".into(),
        owner: "4Qp7Fs2wQmRz8LpXcV4nB7sYd3Ke1JaPu5W9q".into(),
        owner_name: Some("yappr".into()),
        identity,
        sessions: vec![
            offer(
                AppBinding::Group {
                    id: "8Gr4Up9wQmRz8LpXcV4nB7sYd3Ke1JaPu5W9q".into(),
                },
                KeyLevel::Critical,
                50_000_000,
                true,
            ),
            offer(AppBinding::DashPayProfile, KeyLevel::High, 1_000_000, false),
        ],
        encryption: vec![EncryptionOffer {
            contract: "6Hk2Ft9wQmRz8LpXcV4nB7sYd3Ke1JaPu5W9q".into(),
            encryption: true,
            given: true,
            added: true,
        }],
        limits: SessionLimits::wallet(),
        // A renewal: the identity already has keys of the new kind, so no warning (R2).
        breaks_old_clients: !renews,
        renews: renews.then_some(Renewal {
            budget: 50_000_000,
            spent: Some(2_000_000),
            until: end,
        }),
        created_at: evolite_common::now() - 60,
        callback_host: Some("yap.pr".into()),
        cost: wallet.fees.add_key * 3 + wallet.fees.document_write,
    }
}

/// The app a demo v2 sign-in leaves, with the limits the person chose.
pub fn connected_v2(asked: &AppRequestKind, grant: &SessionGrant) -> Option<ConnectedApp> {
    let AppRequestKind::Connect2 {
        app_id, sessions, ..
    } = asked;
    if grant.keys.len() != sessions.len() || grant.keys.first()?.is_none() {
        return None;
    }
    let session = sessions
        .iter()
        .zip(&grant.keys)
        .enumerate()
        .filter_map(|(i, (offer, chosen))| {
            let (budget, expires_at) = (*chosen)?;
            Some(SessionKey {
                id: 6 + i as u32,
                level: offer.level,
                binding: offer.binding.clone(),
                budget,
                expires_at,
                state: SessionKeyState::Live,
                remaining: None,
            })
        })
        .collect();
    Some(ConnectedApp {
        id: app_id.clone(),
        name: "@yappr".into(),
        label: "Yappr".into(),
        domain: "yap.pr".into(),
        last_used: evolite_common::now(),
        session,
        disconnected_at: None,
    })
}

/// The Apps screen's examples (Figma L1, L2): Yappr active, Pasta Notes with its limit used up,
/// Dash Forum past its end, and Jembe disconnected.
pub fn demo_apps(t: Timestamp, day: Timestamp, min: Timestamp) -> Vec<ConnectedApp> {
    let key = |id, binding, budget, left, end, state| SessionKey {
        id,
        level: KeyLevel::High,
        binding,
        budget,
        expires_at: end,
        state,
        remaining: Some(left),
    };
    let write = |id: &str| AppBinding::Group { id: id.into() };
    let app = |id: &str, name: &str, last_used, session, disconnected_at| ConnectedApp {
        id: id.into(),
        name: format!("@{}", name.to_lowercase().replace(' ', "")),
        label: name.into(),
        domain: String::new(),
        last_used,
        session,
        disconnected_at,
    };
    use SessionKeyState::{Disabled, Expired, Live};
    vec![
        app(
            "6Hk2Ft9wQmRz8LpXcV4nB7sYd3Ke1JaPu5W9q",
            "Yappr",
            t - 30 * min,
            vec![
                key(
                    6,
                    write("8Gr4Up9wQmRz8LpXcV4nB7sYd3Ke1JaPu5W9q"),
                    50_000_000,
                    48_000_000,
                    t + 90 * day,
                    Live,
                ),
                key(
                    7,
                    AppBinding::DashPayProfile,
                    1_000_000,
                    1_000_000,
                    t + 90 * day,
                    Live,
                ),
            ],
            None,
        ),
        app(
            "9QaEt4mT7wZp2KcV8nB3sYd6Lf1JaHur2Lk",
            "Pasta Notes",
            t - day,
            vec![key(
                8,
                write("5Pn7Gq2wQmRz8LpXcV4nB7sYd3Ke1JaPu5W9q"),
                10_000_000,
                0,
                t + 21 * day,
                Live,
            )],
            None,
        ),
        app(
            "3vNmQ8wZp2KcV8nB3sYd6Lf1JaHu5L48sTd",
            "Dash Forum",
            t - 12 * day,
            vec![key(
                9,
                write("7Df3Gq2wQmRz8LpXcV4nB7sYd3Ke1JaPu5W9q"),
                50_000_000,
                47_000_000,
                t - 10 * day,
                Expired,
            )],
            None,
        ),
        app(
            "9xQeR4mT7wZp2KcV8nB3sYd6Lf1JaHu5L4tN",
            "Jembe",
            t - 8 * day,
            vec![key(
                5,
                write("2Jb4Gq2wQmRz8LpXcV4nB7sYd3Ke1JaPu5W9q"),
                10_000_000,
                9_000_000,
                t + 60 * day,
                Disabled,
            )],
            Some(t - 3 * day),
        ),
    ]
}

/// What an app link asks, decoded as the real core would (the examples from Figma 11–13).
pub fn app_request(id: u64, text: &str, wallet: &Wallet) -> Result<AppRequest, CoreError> {
    let identity = wallet
        .identity
        .as_ref()
        .and_then(|i| i.name())
        .map(|n| format!("@{n}"))
        .ok_or(CoreError::NotAvailable(
            tr!("An identity is needed first").into(),
        ))?;
    let text = text.trim();
    let v2 = text.to_ascii_lowercase().starts_with("dashpay://connect")
        || (text.starts_with("dash-key:") && text.split(['?', '&']).any(|p| p == "v=2"));
    // The refusals as the real core gives them (E2, E3), for the screens' examples.
    if v2 && text.contains("&stale") {
        return Err(CoreError::AppLink(AppLinkRefusal::Stale));
    }
    if v2 && text.contains("n=m") {
        return Err(CoreError::AppLink(AppLinkRefusal::Mainnet));
    }
    let kind = if v2 {
        connect2_example(identity, wallet, text.contains("&renew"))
    } else if text.starts_with("dash-key:") || text.starts_with("dash-st:") {
        // As the real core: App Connect v1 is not supported (the customer, 2026-11-11).
        return Err(CoreError::AppLink(AppLinkRefusal::Version {
            v: "1".into(),
        }));
    } else {
        return Err(CoreError::BadLink);
    };
    Ok(AppRequest {
        id,
        app: "@yappr".into(),
        kind,
    })
}
