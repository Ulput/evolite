//! DPNS label rules, the same for the mock and the real core: what a valid label is, how DPNS
//! normalizes it against homographs, and which labels go to a masternode vote.

use evolite_api::CoreError;

/// The label as DPNS takes it: without `@` and `.dash`, lowercase; refused unless 3–63 ASCII
/// letters, digits and inner hyphens.
pub fn label(input: &str) -> Result<String, CoreError> {
    let label = input.trim().trim_start_matches('@').to_lowercase();
    let label = label.strip_suffix(".dash").unwrap_or(&label).to_owned();
    let valid = (3..=63).contains(&label.len())
        && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        && !label.starts_with('-')
        && !label.ends_with('-');
    if valid {
        Ok(label)
    } else {
        Err(CoreError::BadName)
    }
}

/// DPNS's homograph-safe form: o → 0, i and l → 1. Two labels with the same form are one name.
pub fn normalized(label: &str) -> String {
    label
        .chars()
        .map(|c| match c {
            'o' => '0',
            'i' | 'l' => '1',
            c => c,
        })
        .collect()
}

/// The fewest characters of a name's beginning the suggestions ask the network about
/// (FEAT-0039, analyst-132): no DPNS name is shorter.
pub const SEARCH_FROM: usize = 3;

/// The homograph-safe form of a name's beginning as typed, for the suggestions (FEAT-0039):
/// without the leading `@` and a whole `.dash`, lowercase, then [`normalized`]. `None` when it
/// cannot begin a DPNS name (a space, a dot, a letter outside ASCII…): an address or an id,
/// not a name. «AIice», «alice» and «a1ice» give one form. Empty for «@» alone.
pub fn search_key(typed: &str) -> Option<String> {
    let lower = typed.trim().trim_start_matches('@').to_lowercase();
    let lower = lower.strip_suffix(".dash").unwrap_or(&lower);
    if lower.len() > 63
        || lower.starts_with('-')
        || !lower.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return None;
    }
    Some(normalized(lower))
}

/// Short names of letters, 0, 1 and hyphens only are contested: masternodes vote on them.
pub fn is_contested(label: &str) -> bool {
    let normalized = normalized(label);
    normalized.len() < 20
        && normalized
            .chars()
            .all(|c| c.is_ascii_lowercase() || c == '0' || c == '1' || c == '-')
}

/// The most a link «where my identity is confirmed» may hold: the `url` of the network's
/// `identityVerify` schema (FEAT-0021 АРХ-6), not the 255 first written in R-10.
pub const IDENTITY_LINK_MAX: usize = 128;

/// Why a link «where my identity is confirmed» is refused before the password (AC-T11).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LinkProblem {
    /// Only `https://`: the schema lets `http` through, a link without encryption is no use.
    NotHttps,
    /// Over [`IDENTITY_LINK_MAX`] characters.
    TooLong,
    /// Not a link a browser opens: no host, a space, a character a URI does not hold.
    NotALink,
}

/// The link as written into the document (AC-T11): trimmed, `https://`, at most
/// [`IDENTITY_LINK_MAX`] characters, a host with a dot and only the characters of a URI.
pub fn identity_link(text: &str) -> Result<String, LinkProblem> {
    let link = text.trim();
    let Some(rest) = link
        .get(..8)
        .filter(|scheme| scheme.eq_ignore_ascii_case("https://"))
        .map(|_| &link[8..])
    else {
        return Err(LinkProblem::NotHttps);
    };
    if link.chars().count() > IDENTITY_LINK_MAX {
        return Err(LinkProblem::TooLong);
    }
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = host.rsplit('@').next().unwrap_or_default();
    let host = host.split(':').next().unwrap_or_default();
    let host_ok = host.contains('.')
        && !host.starts_with('.')
        && !host.ends_with('.')
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.');
    let uri_char = |c: char| c.is_ascii_alphanumeric() || "-._~:/?#[]@!$&'()*+,;=%".contains(c);
    if !host_ok || !rest.chars().all(uri_char) {
        return Err(LinkProblem::NotALink);
    }
    Ok(link.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_labels_as_dpns_does() {
        assert_eq!(label(" @Alice "), Ok("alice".into()));
        assert_eq!(label("alice.dash"), Ok("alice".into()));
        assert_eq!(label("ab"), Err(CoreError::BadName));
        assert_eq!(label("-ab"), Err(CoreError::BadName));
        assert_eq!(label("ab-"), Err(CoreError::BadName));
        assert_eq!(label("a_b"), Err(CoreError::BadName));
        assert_eq!(label(&"a".repeat(64)), Err(CoreError::BadName));
    }

    /// FEAT-0039 AC-T1: the beginning typed is asked in DPNS's own form; what cannot begin a
    /// name is not asked at all.
    #[test]
    fn a_typed_beginning_is_searched_in_the_homograph_safe_form() {
        assert_eq!(search_key("AIICE").as_deref(), Some("a11ce"));
        assert_eq!(search_key(" @AIice ").as_deref(), Some("a11ce"));
        assert_eq!(search_key("a1ice").as_deref(), Some("a11ce"));
        assert_eq!(search_key("bob.dash").as_deref(), Some("b0b"));
        assert_eq!(search_key("@").as_deref(), Some(""));
        assert_eq!(search_key("al-"), Some("a1-".into()));
        assert_eq!(search_key("-al"), None);
        assert_eq!(search_key("al ice"), None);
        assert_eq!(search_key("яна"), None);
        assert_eq!(search_key(&"a".repeat(64)), None);
    }

    #[test]
    fn normalizes_homographs_and_finds_contested_names() {
        // As the name registered on testnet came back from DPNS.
        assert_eq!(
            normalized("evolite-probe-115567219"),
            "ev011te-pr0be-115567219"
        );
        assert!(is_contested("evolite"));
        assert!(is_contested("o1i-lo"));
        assert!(!is_contested("evolite-2026"));
        assert!(!is_contested("a-very-long-name-of-letters"));
    }

    /// FEAT-0021 AC-T11: a link is taken only `https://`, within 128 characters, a host with a dot
    /// and the characters of a URI; refused in words otherwise, before any password.
    #[test]
    fn an_identity_link_is_https_within_128_and_a_real_link() {
        assert_eq!(
            identity_link("  https://example.com/me  "),
            Ok("https://example.com/me".into())
        );
        assert_eq!(
            identity_link("HTTPS://x.org/a?b=1#c"),
            Ok("HTTPS://x.org/a?b=1#c".into())
        );
        assert_eq!(
            identity_link("http://example.com"),
            Err(LinkProblem::NotHttps)
        );
        assert_eq!(identity_link("example.com"), Err(LinkProblem::NotHttps));
        assert_eq!(
            identity_link("ftp://example.com"),
            Err(LinkProblem::NotHttps)
        );
        let at_most = format!("https://example.com/{}", "a".repeat(IDENTITY_LINK_MAX - 20));
        assert_eq!(at_most.chars().count(), IDENTITY_LINK_MAX);
        assert!(identity_link(&at_most).is_ok());
        assert_eq!(
            identity_link(&format!("{at_most}a")),
            Err(LinkProblem::TooLong)
        );
        for bad in [
            "https://",
            "https:///path",
            "https://localhost",
            "https://exa mple.com",
            "https://example.com/a b",
            "https://.com",
            "https://example.com/«me»",
        ] {
            assert_eq!(identity_link(bad), Err(LinkProblem::NotALink), "{bad}");
        }
    }
}
