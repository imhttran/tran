//! Admin > Family Members: the account directory and provider-managed password
//! reset requests.
//!
//! The directory reads the identity database; linked-person status is a
//! read-only lookup in the genealogy database through the USR-003C link table.
//! Invitations, links, and branch reviewer grants are owned by the onboarding
//! slices and are not written here.
//!
//! A password reset is only ever *requested* from the configured identity
//! provider, which owns the verified address and delivers its own single-use
//! link. This module never sees a password, reset link, token, or address in
//! the outbound request, and an unconfigured provider fails closed.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use crate::identity::Role;

/// The directory page size bounds.
pub const DEFAULT_PAGE_SIZE: i64 = 50;
pub const MAX_PAGE_SIZE: i64 = 100;

/// The audit action for every reset request outcome.
pub const RESET_AUDIT_ACTION: &str = "password_reset_requested";

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MemberStatus {
    Active,
    Deactivated,
}

impl MemberStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            MemberStatus::Active => "active",
            MemberStatus::Deactivated => "deactivated",
        }
    }

    pub fn parse(value: &str) -> Option<MemberStatus> {
        match value {
            "active" => Some(MemberStatus::Active),
            "deactivated" => Some(MemberStatus::Deactivated),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct MemberFilter {
    pub query: Option<String>,
    pub status: Option<MemberStatus>,
    pub role: Option<Role>,
    pub after_id: Option<i64>,
    pub limit: Option<i64>,
}

impl MemberFilter {
    fn page_size(&self) -> i64 {
        self.limit
            .unwrap_or(DEFAULT_PAGE_SIZE)
            .clamp(1, MAX_PAGE_SIZE)
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Member {
    pub user_id: i64,
    pub issuer: String,
    pub subject: String,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub roles: Vec<Role>,
    pub status: MemberStatus,
    pub created_at: String,
}

/// Escapes `%`, `_`, and `\` so a search term matches literally in `ILIKE`.
fn like_pattern(term: &str) -> String {
    let mut escaped = String::with_capacity(term.len() + 2);
    escaped.push('%');
    for ch in term.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped.push('%');
    escaped
}

/// Lists accounts with their active roles, keyset-paginated by id. Returns the
/// page and the cursor for the next page, if any.
pub fn list_members(
    client: &mut postgres::Client,
    filter: &MemberFilter,
) -> Result<(Vec<Member>, Option<i64>), String> {
    let limit = filter.page_size();
    let pattern = filter
        .query
        .as_deref()
        .map(str::trim)
        .filter(|term| !term.is_empty())
        .map(like_pattern);
    let status = filter.status.map(MemberStatus::as_str);
    let role = filter.role.map(|role| role.as_str());
    let after = filter.after_id.unwrap_or(0);
    let rows = client
        .query(
            "SELECT u.id, u.issuer, u.subject, u.email, u.display_name,
                    u.deactivated_at IS NOT NULL AS deactivated,
                    to_char(u.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'),
                    COALESCE(array_agg(r.role ORDER BY r.role)
                             FILTER (WHERE r.role IS NOT NULL), '{}')
               FROM users u
               LEFT JOIN role_assignments r
                      ON r.user_id = u.id AND r.revoked_at IS NULL
              WHERE u.id > $1
                AND ($2::text IS NULL OR u.subject ILIKE $2 OR u.email ILIKE $2
                     OR u.display_name ILIKE $2)
                AND ($3::text IS NULL
                     OR ($3 = 'active') = (u.deactivated_at IS NULL))
                AND ($4::text IS NULL OR EXISTS (
                     SELECT 1 FROM role_assignments f
                      WHERE f.user_id = u.id AND f.revoked_at IS NULL AND f.role = $4))
              GROUP BY u.id
              ORDER BY u.id
              LIMIT $5",
            &[&after, &pattern, &status, &role, &(limit + 1)],
        )
        .map_err(|err| err.to_string())?;
    let mut members: Vec<Member> = rows
        .iter()
        .map(|row| {
            let roles: Vec<String> = row.get(7);
            Member {
                user_id: row.get(0),
                issuer: row.get(1),
                subject: row.get(2),
                email: row.get(3),
                display_name: row.get(4),
                status: if row.get(5) {
                    MemberStatus::Deactivated
                } else {
                    MemberStatus::Active
                },
                created_at: row.get(6),
                roles: roles.iter().filter_map(|name| Role::parse(name)).collect(),
            }
        })
        .collect();
    let next = if members.len() as i64 > limit {
        members.truncate(limit as usize);
        members.last().map(|member| member.user_id)
    } else {
        None
    };
    Ok((members, next))
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LinkedPerson {
    pub link_id: i64,
    pub person_key: String,
    pub display_name: String,
    pub username: Option<String>,
}

/// Reads the active person link and reserved username for each account id.
/// Read-only; accounts without a link are absent from the map.
pub fn linked_people(
    genealogy: &mut postgres::Client,
    account_ids: &[i64],
) -> Result<HashMap<i64, LinkedPerson>, String> {
    if account_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = genealogy
        .query(
            "SELECT l.account_id, p.person_key, p.display_name, r.username, l.link_id
               FROM onboarding_person_account_link l
               JOIN person p ON p.person_id = l.person_id
               LEFT JOIN onboarding_username_reservation r
                      ON r.person_id = l.person_id AND r.status <> 'released'
              WHERE l.account_id = ANY($1)",
            &[&account_ids],
        )
        .map_err(|err| err.to_string())?;
    Ok(rows
        .iter()
        .map(|row| {
            (
                row.get(0),
                LinkedPerson {
                    link_id: row.get(4),
                    person_key: row.get(1),
                    display_name: row.get(2),
                    username: row.get(3),
                },
            )
        })
        .collect())
}

/// Reads the active linked person keys for each person key.
/// Read-only; keys without a link are absent from the set.
pub fn linked_person_keys(
    genealogy: &mut postgres::Client,
    person_keys: &[String],
) -> Result<HashSet<String>, String> {
    if person_keys.is_empty() {
        return Ok(HashSet::new());
    }
    let rows = genealogy
        .query(
            "SELECT p.person_key
               FROM onboarding_person_account_link l
               JOIN person p ON p.person_id = l.person_id
              WHERE p.person_key = ANY($1)",
            &[&person_keys],
        )
        .map_err(|err| err.to_string())?;
    Ok(rows.iter().map(|row| row.get(0)).collect())
}

/// The outcome of a reset request. `Requested` means the provider accepted the
/// request; delivery is the provider's and is not confirmed here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetState {
    NotConfigured,
    Requested,
    Failed,
    Throttled,
}

impl ResetState {
    pub fn as_str(self) -> &'static str {
        match self {
            ResetState::NotConfigured => "not_configured",
            ResetState::Requested => "requested",
            ResetState::Failed => "failed",
            ResetState::Throttled => "throttled",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResetError {
    #[error("unknown account")]
    UnknownUser,
    #[error("account is deactivated")]
    Deactivated,
    #[error("account has no email address")]
    NoEmail,
    #[error("database: {0}")]
    Database(String),
}

/// The configured reset endpoint. It receives `{issuer, subject}` only.
#[derive(Clone, PartialEq, Eq)]
pub struct ResetProvider {
    pub url: String,
    pub bearer: Option<String>,
}

// The bearer is a credential: never print it.
impl std::fmt::Debug for ResetProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResetProvider")
            .field("url", &self.url)
            .field("bearer", &self.bearer.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// Who is asking, and for whom. Identities are always `(issuer, subject)`.
pub struct ResetRequest<'a> {
    pub actor_issuer: &'a str,
    pub actor_subject: &'a str,
    pub issuer: &'a str,
    pub subject: &'a str,
}

fn db(err: postgres::Error) -> ResetError {
    ResetError::Database(err.to_string())
}

/// Requests a provider-managed password reset for one active account and
/// audits the outcome. The account row is locked for the cooldown check, the
/// provider call, and the audit insert, so concurrent requests for the same
/// account cannot both reach the provider.
pub fn request_password_reset(
    identity: &mut postgres::Client,
    provider: Option<&ResetProvider>,
    http: &reqwest::blocking::Client,
    request: &ResetRequest<'_>,
    cooldown: Duration,
) -> Result<ResetState, ResetError> {
    let mut tx = identity.transaction().map_err(db)?;
    let row = tx
        .query_opt(
            "SELECT deactivated_at IS NOT NULL, NULLIF(btrim(COALESCE(email, '')), '') IS NOT NULL
               FROM users WHERE issuer = $1 AND subject = $2 FOR UPDATE",
            &[&request.issuer, &request.subject],
        )
        .map_err(db)?
        .ok_or(ResetError::UnknownUser)?;
    if row.get::<_, bool>(0) {
        return Err(ResetError::Deactivated);
    }
    if !row.get::<_, bool>(1) {
        return Err(ResetError::NoEmail);
    }

    let state = match provider {
        None => ResetState::NotConfigured,
        Some(provider) => {
            let cooldown_secs = cooldown.as_secs_f64();
            let recent: bool = tx
                .query_one(
                    "SELECT EXISTS (
                        SELECT 1 FROM audit_events
                         WHERE action = $1 AND subject = $2
                           AND detail->>'issuer' = $3 AND detail->>'state' = 'requested'
                           AND created_at > now() - make_interval(secs => $4))",
                    &[
                        &RESET_AUDIT_ACTION,
                        &request.subject,
                        &request.issuer,
                        &cooldown_secs,
                    ],
                )
                .map_err(db)?
                .get(0);
            if recent {
                ResetState::Throttled
            } else {
                send_reset(http, provider, request.issuer, request.subject)
            }
        }
    };

    let detail = serde_json::json!({
        "issuer": request.issuer,
        "actor_issuer": request.actor_issuer,
        "state": state.as_str(),
    });
    tx.execute(
        "INSERT INTO audit_events (actor, action, subject, detail) VALUES ($1, $2, $3, $4::text::jsonb)",
        &[
            &request.actor_subject,
            &RESET_AUDIT_ACTION,
            &request.subject,
            &detail.to_string(),
        ],
    )
    .map_err(db)?;
    tx.commit().map_err(db)?;
    Ok(state)
}

/// Posts the identity to the provider. Only a 2xx response counts as accepted.
fn send_reset(
    http: &reqwest::blocking::Client,
    provider: &ResetProvider,
    issuer: &str,
    subject: &str,
) -> ResetState {
    let mut call = http
        .post(&provider.url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(serde_json::json!({ "issuer": issuer, "subject": subject }).to_string());
    if let Some(bearer) = &provider.bearer {
        call = call.bearer_auth(bearer);
    }
    match call.send() {
        Ok(response) if response.status().is_success() => ResetState::Requested,
        Ok(response) => {
            eprintln!(
                "admin_members: reset provider returned {}",
                response.status()
            );
            ResetState::Failed
        }
        Err(_) => {
            eprintln!("admin_members: reset provider unreachable");
            ResetState::Failed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_size_is_clamped() {
        let size = |limit| {
            MemberFilter {
                limit,
                ..MemberFilter::default()
            }
            .page_size()
        };
        assert_eq!(size(None), DEFAULT_PAGE_SIZE);
        assert_eq!(size(Some(0)), 1);
        assert_eq!(size(Some(10_000)), MAX_PAGE_SIZE);
    }

    #[test]
    fn search_terms_match_literally() {
        assert_eq!(like_pattern("a_b%c\\"), "%a\\_b\\%c\\\\%");
    }

    #[test]
    fn wire_names_are_stable() {
        assert_eq!(ResetState::NotConfigured.as_str(), "not_configured");
        assert_eq!(ResetState::Throttled.as_str(), "throttled");
        assert_eq!(
            MemberStatus::parse("deactivated"),
            Some(MemberStatus::Deactivated)
        );
        assert_eq!(MemberStatus::parse("invited"), None);
    }

    #[test]
    fn provider_debug_redacts_the_bearer() {
        let provider = ResetProvider {
            url: "https://reset.example".into(),
            bearer: Some("secret-value".into()),
        };
        assert!(!format!("{provider:?}").contains("secret-value"));
    }
}
