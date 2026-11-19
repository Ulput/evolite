//! Which names a suggestion list shows, and in what order (FEAT-0039, AC-A2, AC-T5): a pure
//! function of what was typed, the snapshot's contacts and identities, and the network's answer.

use evolite_api::{Contact, ContactState, IdentityChoice, NameSuggestion, Timestamp};
use evolite_core::dpns::search_key;

/// The most rows a list shows.
pub const ROWS: usize = 5;

/// Where the list is: what it may offer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Place {
    /// «Send › To»: one's own identities too — one may pay oneself.
    Send,
    /// «Add a contact»: not one's own identities.
    AddContact,
}

/// The tag on the row's right.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tag {
    /// A stranger: nothing — the checked card after the choice says «Not a contact».
    None,
    /// «In contacts»: an established contact.
    Contact,
    /// «Your identity».
    Own,
}

/// One row of the list.
#[derive(Clone, PartialEq, Debug)]
pub struct Row {
    /// The name as written on the network, without «@»: what the choice puts in the field.
    pub label: String,
    pub identity_id: String,
    /// The profile's name, when known.
    pub display: Option<String>,
    pub tag: Tag,
    /// How many leading characters of `label` matched what was typed (drawn SemiBold); 0 when
    /// only the profile's name matched.
    pub matched: usize,
    /// The contact's picture, from the avatars' cache.
    pub picture: Option<String>,
}

/// The rows' groups, in the order shown (analyst's rules 2–4, analyst-128).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Group {
    /// The name typed, whole: always first.
    Exact,
    /// A contact whose @name starts with it.
    ContactByName,
    /// A contact whose profile's name has it.
    ContactByProfile,
    /// One's own identity (Send only).
    Own,
    /// A name from the network.
    Network,
}

/// The rows for `typed`, at most [`ROWS`]: the exact name first, then contacts (by @name, then by
/// profile's name; the last paid first), one's own identities, then the network's names (shorter
/// first, then by the name). Compared in DPNS's homograph-safe form; one row per identity, by
/// the closest of its names. Empty when `typed` cannot begin a name.
pub fn rank(
    typed: &str,
    place: Place,
    contacts: &[Contact],
    own: &[IdentityChoice],
    network: &[NameSuggestion],
) -> Vec<Row> {
    let Some(key) = search_key(typed).filter(|k| !k.is_empty()) else {
        return Vec::new();
    };
    let words = typed.trim().trim_start_matches('@').to_lowercase();
    let established = || {
        contacts
            .iter()
            .filter(|c| c.state == ContactState::Established && c.name.is_some())
    };
    let contact_of = |id: &str| established().find(|c| c.identity_id == id);
    let is_own = |id: &str| own.iter().any(|i| i.id == id);
    let tag_of = |id: &str| {
        if is_own(id) {
            Tag::Own
        } else if contact_of(id).is_some() {
            Tag::Contact
        } else {
            Tag::None
        }
    };
    // (group, last payment for contacts, the name's form) and the row.
    let mut all: Vec<(Group, Option<Timestamp>, String, Row)> = Vec::new();
    let mut push = |group: Group,
                    paid: Option<Timestamp>,
                    label: &str,
                    id: &str,
                    display,
                    picture: Option<String>| {
        let form = search_key(label).unwrap_or_default();
        let by_name = form.starts_with(&key);
        let group = if form == key { Group::Exact } else { group };
        let contact = contact_of(id);
        all.push((
            group,
            paid,
            form,
            Row {
                label: label.to_owned(),
                identity_id: id.to_owned(),
                display: contact.and_then(|c| c.profile.display.clone()).or(display),
                tag: tag_of(id),
                matched: if by_name { key.chars().count() } else { 0 },
                // A contact's from the snapshot; a stranger's only as the core brought it
                // (the switch on, the hash checked).
                picture: contact.and_then(|c| c.profile.avatar.clone()).or(picture),
            },
        ));
    };
    for c in established() {
        let label = c.name.as_deref().unwrap_or_default();
        let by_name = search_key(label).is_some_and(|f| f.starts_with(&key));
        let by_profile = c
            .profile
            .display
            .as_deref()
            .is_some_and(|d| d.to_lowercase().contains(&words));
        if by_name || by_profile {
            let group = if by_name {
                Group::ContactByName
            } else {
                Group::ContactByProfile
            };
            push(group, c.last_payment, label, &c.identity_id, None, None);
        }
    }
    if place == Place::Send {
        for i in own {
            if let Some(label) = i.name.as_deref()
                && search_key(label).is_some_and(|f| f.starts_with(&key))
            {
                push(Group::Own, None, label, &i.id, None, None);
            }
        }
    }
    for n in network {
        // An answer to a shorter beginning is shown narrowed until the next one comes.
        if search_key(&n.label).is_some_and(|f| f.starts_with(&key)) {
            let group = match tag_of(&n.identity_id) {
                Tag::Contact => Group::ContactByName,
                Tag::Own => Group::Own,
                Tag::None => Group::Network,
            };
            push(
                group,
                contact_of(&n.identity_id).and_then(|c| c.last_payment),
                &n.label,
                &n.identity_id,
                n.display_name.clone(),
                n.picture.clone(),
            );
        }
    }
    if place == Place::AddContact {
        all.retain(|(_, _, _, row)| row.tag != Tag::Own);
    }
    all.sort_by(|a, b| {
        let order = a.0.cmp(&b.0);
        let within = match a.0 {
            // The last paid first, never paid last.
            Group::ContactByName | Group::ContactByProfile => b.1.cmp(&a.1),
            _ => std::cmp::Ordering::Equal,
        };
        order
            .then(within)
            .then(a.2.len().cmp(&b.2.len()))
            .then_with(|| a.2.cmp(&b.2))
    });
    let mut rows: Vec<Row> = Vec::new();
    for (_, _, _, row) in all {
        if rows.len() == ROWS {
            break;
        }
        if !rows.iter().any(|r| r.identity_id == row.identity_id) {
            rows.push(row);
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolite_api::Profile;

    fn contact(name: &str, display: Option<&str>, paid: Option<Timestamp>) -> Contact {
        Contact {
            identity_id: format!("id-{name}"),
            name: Some(name.into()),
            profile: Profile {
                display: display.map(Into::into),
                ..Profile::default()
            },
            state: ContactState::Established,
            since: 0,
            payments: u32::from(paid.is_some()),
            last_payment: paid,
            channel_broken: false,
        }
    }

    fn net(label: &str, id: &str) -> NameSuggestion {
        NameSuggestion {
            label: label.into(),
            identity_id: id.into(),
            display_name: None,
            picture: None,
        }
    }

    fn own(name: &str) -> IdentityChoice {
        IdentityChoice {
            id: format!("id-{name}"),
            name: Some(name.into()),
            slot: 0,
        }
    }

    fn labels(rows: &[Row]) -> Vec<&str> {
        rows.iter().map(|r| r.label.as_str()).collect()
    }

    /// AC-T5: the exact name first even from the network; then contacts — by @name, the last
    /// paid first, then by the profile's name; one's own; the network shorter first.
    #[test]
    fn exact_then_contacts_then_own_then_the_network() {
        let contacts = [
            contact("alison", None, Some(10)),
            contact("alice-shop", None, Some(20)),
            contact("zed", Some("Ali Baba"), Some(30)),
        ];
        let network = [
            net("ali", "id-ali"),
            net("alice-shop", "id-alice-shop"),
            net("alicia", "id-alicia"),
            net("alibi", "id-alibi"),
        ];
        let rows = rank("ali", Place::Send, &contacts, &[own("alix")], &network);
        assert_eq!(
            labels(&rows),
            ["ali", "alice-shop", "alison", "zed", "alix"]
        );
        assert_eq!(rows[0].tag, Tag::None);
        assert_eq!(rows[1].tag, Tag::Contact);
        assert_eq!(
            rows[3].matched, 0,
            "found by the profile's name: nothing bold"
        );
        assert_eq!(rows[3].display.as_deref(), Some("Ali Baba"));
        assert_eq!(rows[4].tag, Tag::Own);
        let rows = rank("ali", Place::Send, &[], &[], &network);
        assert_eq!(labels(&rows), ["ali", "alibi", "alicia", "alice-shop"]);
    }

    /// AC-A2: compared in DPNS's form — «AIICE» finds @alice, and the bold part counts typed
    /// characters.
    #[test]
    fn homographs_are_one_name() {
        let network = [net("alice", "a"), net("Al1ce-pay", "b"), net("alex", "c")];
        let rows = rank("@AIICE", Place::Send, &[], &[], &network);
        assert_eq!(labels(&rows), ["alice", "Al1ce-pay"]);
        assert_eq!(rows[0].matched, 5);
        assert!(rank("al ice", Place::Send, &[], &[], &network).is_empty());
        assert!(rank("@", Place::Send, &[], &[], &network).is_empty());
    }

    /// AC-T5: one row per identity, by its closest name.
    #[test]
    fn one_row_per_identity() {
        let network = [
            net("bob", "same"),
            net("bobby", "same"),
            net("bobcat", "other"),
        ];
        let rows = rank("bob", Place::Send, &[], &[], &network);
        assert_eq!(labels(&rows), ["bob", "bobcat"]);
        let rows = rank("bobb", Place::Send, &[], &[], &network);
        assert_eq!(labels(&rows), ["bobby"]);
    }

    /// AC-A7: «Add a contact» offers no own identity, from the snapshot or the network; a
    /// contact is tagged.
    #[test]
    fn add_contact_offers_no_own_identity() {
        let network = [net("satoshi", "id-satoshi"), net("satoshi-fan", "fan")];
        let contacts = [contact("sato", None, None)];
        let rows = rank(
            "sat",
            Place::AddContact,
            &contacts,
            &[own("satoshi")],
            &network,
        );
        assert_eq!(labels(&rows), ["sato", "satoshi-fan"]);
        assert_eq!(rows[0].tag, Tag::Contact);
        let rows = rank("sat", Place::Send, &contacts, &[own("satoshi")], &network);
        assert_eq!(labels(&rows), ["sato", "satoshi", "satoshi-fan"]);
        assert_eq!(rows[1].tag, Tag::Own);
    }

    /// AC-A10: a stranger's picture is the one the core brought; a contact's is the snapshot's.
    #[test]
    fn a_strangers_picture_comes_from_the_core_a_contacts_from_the_snapshot() {
        let mut stranger = net("alina", "s");
        stranger.picture = Some("avatar:s:1".into());
        let mut known = contact("alice", None, None);
        known.profile.avatar = Some("avatar:c:2".into());
        let mut also = net("alice", "id-alice");
        also.picture = Some("avatar:other".into());
        let rows = rank("ali", Place::Send, &[known], &[], &[stranger, also]);
        let pictures: Vec<_> = rows.iter().map(|r| r.picture.as_deref()).collect();
        assert_eq!(pictures, [Some("avatar:c:2"), Some("avatar:s:1")]);
    }

    /// At most five; a contact not established is a stranger.
    #[test]
    fn five_at_most_and_only_established_contacts_count() {
        let network: Vec<_> = (0..10)
            .map(|i| net(&format!("kat{i}"), &format!("k{i}")))
            .collect();
        assert_eq!(rank("kat", Place::Send, &[], &[], &network).len(), ROWS);
        let mut asked = contact("kate", None, None);
        asked.state = ContactState::Outgoing;
        let rows = rank("kat", Place::Send, &[asked], &[], &[]);
        assert!(
            rows.is_empty(),
            "not established: not offered from the snapshot"
        );
    }
}
