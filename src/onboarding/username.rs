//! Username policy for the Tran onboarding feature.
//!
//! This module is library-only and pure. It generates a display name, a
//! case-insensitive ASCII key, a candidate username, and a unique username. It
//! never touches persistence, passwords, birthdays, or HTTP endpoints.
//!
//! Candidate shape: `[lastInitial][middleInitial][givenName]` using verified
//! structured name parts. The family name is first and the given name is last.

use thiserror::Error;
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

/// Verified structured name parts for one person.
///
/// A part is `None` when it is absent. Callers provide only verified parts.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NameParts {
    /// The family name (last name), for example `Nguyen`.
    pub family: Option<String>,
    /// The middle name, for example `Van`.
    pub middle: Option<String>,
    /// The given name (first name), for example `An`.
    pub given: Option<String>,
}

/// Errors raised by the username policy.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum UsernameError {
    /// The family name is missing or empty.
    #[error("family name is missing")]
    MissingFamily,
    /// The given name is missing or empty.
    #[error("given name is missing")]
    MissingGiven,
    /// Only one token is present, so the name is ambiguous.
    #[error("single-token name is ambiguous")]
    AmbiguousSingleToken,
}

/// Preserves the Unicode display name, trims surrounding whitespace, and
/// collapses internal whitespace runs to one space.
pub fn normalize_display_name(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Produces a case-insensitive ASCII key by lowercasing and removing Vietnamese
/// diacritics, including mapping `đ`/`Đ` to `d`.
pub fn key(name: &str) -> String {
    normalize_display_name(name)
        .nfd()
        .filter(|ch| !is_combining_mark(*ch))
        .flat_map(char::to_lowercase)
        .map(|ch| if ch == 'đ' { 'd' } else { ch })
        .collect()
}

/// Returns `true` when the text has no non-whitespace content.
fn is_blank(text: &str) -> bool {
    text.trim().is_empty()
}

/// Returns the uppercased first char of a trimmed part, or an empty string.
fn initial(part: &Option<String>) -> String {
    match part {
        Some(value) if !is_blank(value) => value
            .trim()
            .chars()
            .next()
            .map(|ch| ch.to_uppercase().collect::<String>())
            .unwrap_or_default(),
        _ => String::new(),
    }
}

/// Builds the lowercase, accent-free candidate username.
///
/// A missing middle initial is omitted. For example, `Nguyễn Văn An` becomes
/// `nvan`. Full display names retain their original spelling.
pub fn candidate_username(parts: &NameParts) -> Result<String, UsernameError> {
    let family = parts.family.as_deref().map(str::trim).unwrap_or("");
    let given = parts.given.as_deref().map(str::trim).unwrap_or("");

    let has_family = !is_blank(family);
    let has_given = !is_blank(given);

    let token_count = [&parts.family, &parts.middle, &parts.given]
        .iter()
        .filter(|part| {
            part.as_deref()
                .map(|value| !is_blank(value))
                .unwrap_or(false)
        })
        .count();

    // A single-token name has exactly one name part present: the family name
    // or the given name is unknown, so the name is ambiguous. This check runs
    // before the missing-part checks so it reports the more precise error.
    if token_count == 1 {
        return Err(UsernameError::AmbiguousSingleToken);
    }
    if !has_family {
        return Err(UsernameError::MissingFamily);
    }
    if !has_given {
        return Err(UsernameError::MissingGiven);
    }

    let mut candidate = String::new();
    candidate.push_str(&initial(&parts.family));
    candidate.push_str(&initial(&parts.middle));
    candidate.push_str(given);
    Ok(key(&candidate))
}

/// Enforces case-insensitive uniqueness by appending the smallest deterministic
/// numeric suffix starting at 2 (`nvan`, `nvan2`, `nvan3`).
///
/// Comparison uses [`key`] on the base and on every taken entry. This never uses
/// a date of birth.
pub fn unique_username(base: &str, taken: &[String]) -> String {
    let base_key = key(base);
    let taken_keys: Vec<String> = taken.iter().map(|item| key(item)).collect();

    if !taken_keys.iter().any(|item| item == &base_key) {
        return base_key;
    }

    let mut suffix = 2u64;
    loop {
        let candidate = format!("{base_key}{suffix}");
        if !taken_keys.iter().any(|item| item == &candidate) {
            return candidate;
        }
        suffix += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_name_trims_and_collapses_whitespace() {
        assert_eq!(
            normalize_display_name("  Nguyen   Van\tAn "),
            "Nguyen Van An"
        );
    }

    #[test]
    fn display_name_preserves_unicode() {
        assert_eq!(
            normalize_display_name("  Nguyễn   Văn An "),
            "Nguyễn Văn An"
        );
    }

    #[test]
    fn key_folds_vietnamese_diacritics() {
        assert_eq!(key("Nguyễn Văn An"), "nguyen van an");
    }

    #[test]
    fn key_maps_d_bar() {
        assert_eq!(key("Đ"), "d");
        assert_eq!(key("đ"), "d");
        assert_eq!(key("Đặng Thị Hồng"), "dang thi hong");
    }

    #[test]
    fn candidate_is_lowercase_without_accents() {
        let parts = NameParts {
            family: Some("Nguyễn".to_string()),
            middle: Some("Văn".to_string()),
            given: Some("Án".to_string()),
        };
        assert_eq!(candidate_username(&parts).unwrap(), "nvan");
        assert_eq!(key("NVAn"), "nvan");
    }

    #[test]
    fn candidate_omits_missing_middle() {
        let parts = NameParts {
            family: Some("Nguyen".to_string()),
            middle: None,
            given: Some("An".to_string()),
        };
        assert_eq!(candidate_username(&parts).unwrap(), "nan");
    }

    #[test]
    fn candidate_handles_decomposed_accents_and_d_bar() {
        let parts = NameParts {
            family: Some("Đặng".to_string()),
            middle: Some("Thị".to_string()),
            given: Some("HỒNG".nfd().collect()),
        };
        assert_eq!(candidate_username(&parts).unwrap(), "dthong");
        assert_eq!(unique_username("ĐTHỒNG", &["dthong".into()]), "dthong2");
    }

    #[test]
    fn candidate_errors_on_missing_family() {
        let parts = NameParts {
            family: None,
            middle: Some("Van".to_string()),
            given: Some("An".to_string()),
        };
        assert_eq!(
            candidate_username(&parts),
            Err(UsernameError::MissingFamily)
        );
    }

    #[test]
    fn candidate_errors_on_missing_given() {
        let parts = NameParts {
            family: Some("Nguyen".to_string()),
            middle: Some("Van".to_string()),
            given: None,
        };
        assert_eq!(candidate_username(&parts), Err(UsernameError::MissingGiven));
    }

    #[test]
    fn candidate_errors_on_single_token() {
        let parts = NameParts {
            family: None,
            middle: None,
            given: Some("An".to_string()),
        };
        assert_eq!(
            candidate_username(&parts),
            Err(UsernameError::AmbiguousSingleToken)
        );
    }

    #[test]
    fn unique_username_returns_base_when_free() {
        assert_eq!(unique_username("nvan", &[]), "nvan");
    }

    #[test]
    fn unique_username_appends_2_on_collision() {
        let taken = vec!["nvan".to_string()];
        assert_eq!(unique_username("nvan", &taken), "nvan2");
    }

    #[test]
    fn unique_username_appends_3_on_double_collision() {
        let taken = vec!["nvan".to_string(), "nvan2".to_string()];
        assert_eq!(unique_username("nvan", &taken), "nvan3");
    }

    #[test]
    fn unique_username_compares_case_insensitively() {
        let taken = vec!["NVAn".to_string()];
        assert_eq!(unique_username("nvan", &taken), "nvan2");
    }
}
