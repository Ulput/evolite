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
    /// BUG-268: where the profile's name matched, when it did and the @name did not: the first
    /// character of the word and how many characters, drawn SemiBold.
    pub display_matched: Option<(usize, usize)>,
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
    /// A contact whose profile's name has a word that starts with it (BUG-268).
    ContactByProfile,
    /// One's own identity (Send only).
    Own,
    /// A name from the network.
    Network,
}

/// BUG-268 (the customer): where a word of `text` starts with `typed`, case aside — the first
/// character of that word and the length of `typed`, in characters. Never a letter in the middle
/// of a word: «l» is not found in «evolite probe», «pro» is.
pub fn word_start(text: &str, typed: &str) -> Option<(usize, usize)> {
    let typed: Vec<char> = typed.chars().flat_map(char::to_lowercase).collect();
    if typed.is_empty() {
        return None;
    }
    let chars: Vec<char> = text.chars().collect();
    (0..chars.len())
        .filter(|&i| i == 0 || !chars[i - 1].is_alphanumeric())
        .find(|&i| {
            let rest: Vec<char> = chars[i..]
                .iter()
                .take(typed.len())
                .flat_map(|c| c.to_lowercase())
                .collect();
            rest.starts_with(&typed)
        })
        .map(|i| (i, typed.len()))
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
        let display: Option<String> = contact.and_then(|c| c.profile.display.clone()).or(display);
        let display_matched = (group == Group::ContactByProfile && !by_name)
            .then(|| display.as_deref().and_then(|d| word_start(d, &words)))
            .flatten();
        all.push((
            group,
            paid,
            form,
            Row {
                label: label.to_owned(),
                identity_id: id.to_owned(),
                display,
                tag: tag_of(id),
                matched: if by_name { key.chars().count() } else { 0 },
                display_matched,
                // A contact's from the snapshot; a stranger's only as the core brought it
                // (the switch on, the hash checked).
                picture: contact.and_then(|c| c.profile.avatar.clone()).or(picture),
            },
        ));
    };
    for c in established() {
        let label = c.name.as_deref().unwrap_or_default();
        let by_name = search_key(label).is_some_and(|f| f.starts_with(&key));
        // BUG-268: the profile's name from the start of one of its words, never a letter inside.
        let by_profile = c
            .profile
            .display
            .as_deref()
            .is_some_and(|d| word_start(d, &words).is_some());
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

    /// BUG-268 (the customer): matching from the start, never a letter inside a word. The
    /// contact @werner555 «evolite probe»: «@l» offers nothing (neither the name nor a word of the
    /// profile's name starts with it); «@w» offers it by its name; «@pro» by a word of its
    /// profile's name, that word's start marked; «l» and «obe» not at all.
    #[test]
    fn contacts_match_from_the_start_of_a_name_or_a_word() {
        let werner = contact("werner555", Some("evolite probe"), None);
        let ids = |typed: &str| {
            rank(typed, Place::Send, std::slice::from_ref(&werner), &[], &[])
                .into_iter()
                .map(|r| (r.label, r.matched, r.display_matched))
                .collect::<Vec<_>>()
        };
        assert!(ids("@l").is_empty(), "«@l»");
        assert!(ids("l").is_empty(), "«l»: inside «evolite»");
        assert!(ids("obe").is_empty(), "«obe»: inside «probe»");
        assert_eq!(ids("@w"), [("werner555".to_owned(), 1, None)]);
        assert_eq!(
            ids("@WER"),
            [("werner555".to_owned(), 3, None)],
            "case aside"
        );
        assert_eq!(
            ids("pro"),
            [("werner555".to_owned(), 0, Some((8, 3)))],
            "«pro»: the start of «probe», marked"
        );
        assert_eq!(
            ids("@pro"),
            [("werner555".to_owned(), 0, Some((8, 3)))],
            "«@pro» in «To»: the same"
        );
        assert_eq!(ids("evo"), [("werner555".to_owned(), 0, Some((0, 3)))]);
    }

    /// The word's start in characters, whatever the letters' bytes.
    #[test]
    fn a_words_start_is_counted_in_characters() {
        assert_eq!(word_start("Анна Петрова", "пет"), Some((5, 3)));
        assert_eq!(word_start("Анна Петрова", "етр"), None);
        assert_eq!(word_start("Jean-Luc", "luc"), Some((5, 3)));
        assert_eq!(word_start("anything", ""), None);
    }
}
