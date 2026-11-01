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

/// Short names of letters, 0, 1 and hyphens only are contested: masternodes vote on them.
pub fn is_contested(label: &str) -> bool {
    let normalized = normalized(label);
    normalized.len() < 20
        && normalized
            .chars()
            .all(|c| c.is_ascii_lowercase() || c == '0' || c == '1' || c == '-')
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
}
