//! Head of Family nomination, appointment, and revocation (TRAN-003E).
//!
//! The workflow reuses TRAN-002's pure eligibility decision and evidence-backed
//! catalog. It never grants an application role: the designation is a genealogy
//! fact only. Writes run in one genealogy-database transaction, serialize on the
//! branch with an advisory lock, and rely on a partial unique index as the final
//! one-active-appointment guard.

use postgres::{Client, Transaction};
use serde::Serialize;

use super::eligibility::{self, EligibilityDecision};
use super::query::Citation;
use super::GenealogyError;

/// One active or historical appointment.
#[derive(Debug, Clone, Serialize)]
pub struct AppointmentRow {
    pub appointment_id: i64,
    pub person_key: String,
    pub display_name: String,
    pub branch_key: String,
    pub approved_by: String,
    pub evidence_assertion_ids: Vec<i64>,
    pub status: String,
    pub appointed_at: String,
}

/// One nomination.
#[derive(Debug, Clone, Serialize)]
pub struct NominationRow {
    pub nomination_id: i64,
    pub person_key: String,
    pub branch_key: String,
    pub nominated_by: String,
    pub status: String,
    pub evidence_assertion_ids: Vec<i64>,
    pub created_at: String,
}

fn json_ids(value: &str) -> Vec<i64> {
    serde_json::from_str(value).unwrap_or_default()
}

fn audit(
    tx: &mut Transaction<'_>,
    actor: &str,
    action: &str,
    appointment_id: Option<i64>,
    nomination_id: Option<i64>,
    detail: &str,
) -> Result<(), GenealogyError> {
    tx.execute(
        "INSERT INTO head_of_family_audit_event \
         (appointment_id, nomination_id, actor, action, detail) \
         VALUES ($1, $2, $3, $4, $5::text::jsonb)",
        &[&appointment_id, &nomination_id, &actor, &action, &detail],
    )
    .map_err(GenealogyError::Query)?;
    Ok(())
}

/// Lists active appointments, including the person's display label.
pub fn list_active_appointments(
    client: &mut Client,
) -> Result<Vec<AppointmentRow>, GenealogyError> {
    let rows = client
        .query(
            "SELECT a.appointment_id, a.person_key, p.display_name, a.branch_key, \
                    a.approved_by, a.evidence_assertion_ids::text, a.status, a.appointed_at::text \
               FROM head_of_family_appointment a \
               JOIN person p ON p.person_key = a.person_key \
              WHERE a.status = 'active' \
              ORDER BY a.branch_key",
            &[],
        )
        .map_err(GenealogyError::Query)?;
    Ok(rows
        .iter()
        .map(|row| {
            let evidence: String = row.get(5);
            AppointmentRow {
                appointment_id: row.get(0),
                person_key: row.get(1),
                display_name: row.get(2),
                branch_key: row.get(3),
                approved_by: row.get(4),
                evidence_assertion_ids: json_ids(&evidence),
                status: row.get(6),
                appointed_at: row.get(7),
            }
        })
        .collect())
}

/// Lists pending nominations.
pub fn list_pending_nominations(client: &mut Client) -> Result<Vec<NominationRow>, GenealogyError> {
    let rows = client
        .query(
            "SELECT nomination_id, person_key, branch_key, nominated_by, status, \
                    evidence_assertion_ids::text, created_at::text \
               FROM head_of_family_nomination \
              WHERE status = 'pending' ORDER BY created_at",
            &[],
        )
        .map_err(GenealogyError::Query)?;
    Ok(rows
        .iter()
        .map(|row| {
            let evidence: String = row.get(5);
            NominationRow {
                nomination_id: row.get(0),
                person_key: row.get(1),
                branch_key: row.get(2),
                nominated_by: row.get(3),
                status: row.get(4),
                evidence_assertion_ids: json_ids(&evidence),
                created_at: row.get(6),
            }
        })
        .collect())
}

/// Creates a pending nomination for an existing cataloged person.
pub fn nominate(
    client: &mut Client,
    person_key: &str,
    branch_key: &str,
    actor: &str,
) -> Result<i64, GenealogyError> {
    if person_key.trim().is_empty() || branch_key.trim().is_empty() || actor.trim().is_empty() {
        return Err(GenealogyError::Domain(
            "person, branch, and nominator are required".into(),
        ));
    }
    let exists = client
        .query_opt("SELECT 1 FROM person WHERE person_key = $1", &[&person_key])
        .map_err(GenealogyError::Query)?;
    if exists.is_none() {
        return Err(GenealogyError::Domain("unknown person".into()));
    }

    let input = eligibility::load_input(client, person_key, branch_key)?;
    let decision = eligibility::decide(&input);
    let evidence = decision.supporting_assertion_ids;
    let evidence_json = serde_json::to_string(&evidence).unwrap_or_else(|_| "[]".to_string());

    let row = client
        .query_one(
            "INSERT INTO head_of_family_nomination \
             (person_key, branch_key, nominated_by, status, evidence_assertion_ids) \
             VALUES ($1, $2, $3, 'pending', $4::text::jsonb) \
             RETURNING nomination_id",
            &[&person_key, &branch_key, &actor, &evidence_json],
        )
        .map_err(GenealogyError::Query)?;
    Ok(row.get(0))
}

/// Rejects a pending nomination without creating an appointment.
pub fn reject(
    client: &mut Client,
    nomination_id: i64,
    actor: &str,
    note: Option<&str>,
) -> Result<(), GenealogyError> {
    let mut tx = client.transaction().map_err(GenealogyError::Query)?;
    let row = tx
        .query_opt(
            "SELECT status FROM head_of_family_nomination \
              WHERE nomination_id = $1 FOR UPDATE",
            &[&nomination_id],
        )
        .map_err(GenealogyError::Query)?;
    let Some(row) = row else {
        return Err(GenealogyError::Domain("unknown nomination".into()));
    };
    if row.get::<_, String>(0) != "pending" {
        return Err(GenealogyError::Domain("nomination is not pending".into()));
    }
    tx.execute(
        "UPDATE head_of_family_nomination \
            SET status = 'rejected', decided_at = now(), decided_by = $2, note = $3 \
          WHERE nomination_id = $1",
        &[&nomination_id, &actor, &note],
    )
    .map_err(GenealogyError::Query)?;
    audit(
        &mut tx,
        actor,
        "head_of_family.nominate.reject",
        None,
        Some(nomination_id),
        "{}",
    )?;
    tx.commit().map_err(GenealogyError::Query)?;
    Ok(())
}

/// Approves a pending nomination.
///
/// The transaction locks the branch, re-evaluates the verified eligibility
/// evidence, checks that no active appointment exists, inserts the appointment,
/// updates the nomination, and writes the audit event atomically.
pub fn approve(
    client: &mut Client,
    nomination_id: i64,
    branch_key: &str,
    admin: &str,
) -> Result<AppointmentRow, GenealogyError> {
    if branch_key.trim().is_empty() || admin.trim().is_empty() {
        return Err(GenealogyError::Domain(
            "branch and approver are required".into(),
        ));
    }
    let mut tx = client.transaction().map_err(GenealogyError::Query)?;

    // Serialize competing approvals for this branch.
    let lock_key = format!("hof:{branch_key}");
    tx.execute(
        "SELECT pg_advisory_xact_lock(hashtext($1)::bigint)",
        &[&lock_key],
    )
    .map_err(GenealogyError::Query)?;

    let row = tx
        .query_opt(
            "SELECT person_key, branch_key, status, nominated_by FROM head_of_family_nomination \
              WHERE nomination_id = $1 FOR UPDATE",
            &[&nomination_id],
        )
        .map_err(GenealogyError::Query)?;
    let Some(row) = row else {
        return Err(GenealogyError::Domain("unknown nomination".into()));
    };
    let person_key: String = row.get(0);
    let nomination_branch: String = row.get(1);
    let status: String = row.get(2);
    if status != "pending" {
        return Err(GenealogyError::Domain("nomination is not pending".into()));
    }
    if row.get::<_, String>(3) == admin {
        return Err(GenealogyError::Domain(
            "a nomination cannot be approved by its nominator".into(),
        ));
    }
    if nomination_branch != branch_key {
        return Err(GenealogyError::Domain(
            "nomination branch does not match the configured third branch".into(),
        ));
    }

    let active = tx
        .query_opt(
            "SELECT 1 FROM head_of_family_appointment \
              WHERE branch_key = $1 AND status = 'active' FOR UPDATE",
            &[&branch_key],
        )
        .map_err(GenealogyError::Query)?;
    if active.is_some() {
        return Err(GenealogyError::Domain(
            "an active Head of Family already exists for this branch".into(),
        ));
    }

    let input = eligibility::load_input(&mut tx, &person_key, branch_key)?;
    let decision: EligibilityDecision = eligibility::decide(&input);
    if !decision.is_eligible() {
        return Err(GenealogyError::Domain(format!(
            "candidate is not eligible: {}",
            decision.reason_code.as_str()
        )));
    }
    let evidence = decision.supporting_assertion_ids;
    let evidence_json = serde_json::to_string(&evidence).unwrap_or_else(|_| "[]".to_string());

    let appointment_id: i64 = tx
        .query_one(
            "INSERT INTO head_of_family_appointment \
             (nomination_id, person_key, branch_key, approved_by, evidence_assertion_ids, status) \
             VALUES ($1, $2, $3, $4, $5::text::jsonb, 'active') \
             RETURNING appointment_id",
            &[
                &nomination_id,
                &person_key,
                &branch_key,
                &admin,
                &evidence_json,
            ],
        )
        .map_err(GenealogyError::Query)?
        .get(0);

    tx.execute(
        "UPDATE head_of_family_nomination \
            SET status = 'approved', decided_at = now(), decided_by = $2 \
          WHERE nomination_id = $1",
        &[&nomination_id, &admin],
    )
    .map_err(GenealogyError::Query)?;

    audit(
        &mut tx,
        admin,
        "head_of_family.approve",
        Some(appointment_id),
        Some(nomination_id),
        "{}",
    )?;

    tx.commit().map_err(GenealogyError::Query)?;

    Ok(AppointmentRow {
        appointment_id,
        person_key,
        display_name: String::new(),
        branch_key: branch_key.to_string(),
        approved_by: admin.to_string(),
        evidence_assertion_ids: evidence,
        status: "active".to_string(),
        appointed_at: String::new(),
    })
}

/// Revokes or suspends an active appointment. It never creates a successor.
pub fn end_appointment(
    client: &mut Client,
    appointment_id: i64,
    actor: &str,
    reason: &str,
    suspend: bool,
) -> Result<(), GenealogyError> {
    if reason.trim().is_empty() {
        return Err(GenealogyError::Domain("reason is required".into()));
    }
    let mut tx = client.transaction().map_err(GenealogyError::Query)?;
    let row = tx
        .query_opt(
            "SELECT status FROM head_of_family_appointment \
              WHERE appointment_id = $1 FOR UPDATE",
            &[&appointment_id],
        )
        .map_err(GenealogyError::Query)?;
    let Some(row) = row else {
        return Err(GenealogyError::Domain("unknown appointment".into()));
    };
    if row.get::<_, String>(0) != "active" {
        return Err(GenealogyError::Domain("appointment is not active".into()));
    }
    let status = if suspend { "suspended" } else { "revoked" };
    tx.execute(
        "UPDATE head_of_family_appointment \
            SET status = $2, ended_at = now(), end_reason = $3 \
          WHERE appointment_id = $1",
        &[&appointment_id, &status, &reason],
    )
    .map_err(GenealogyError::Query)?;
    audit(
        &mut tx,
        actor,
        if suspend {
            "head_of_family.suspend"
        } else {
            "head_of_family.revoke"
        },
        Some(appointment_id),
        None,
        "{}",
    )?;
    tx.commit().map_err(GenealogyError::Query)?;
    Ok(())
}

/// Loads citations for an appointment's supporting assertion IDs.
pub fn citations_for_assertions(
    client: &mut Client,
    assertion_ids: &[i64],
) -> Result<Vec<Citation>, GenealogyError> {
    if assertion_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = client
        .query(
            "SELECT document_ref, page, locator, excerpt FROM assertion_evidence \
              WHERE assertion_id = ANY($1) AND withdrawn_at IS NULL \
              ORDER BY document_ref, page, locator",
            &[&assertion_ids],
        )
        .map_err(GenealogyError::Query)?;
    Ok(rows
        .iter()
        .map(|row| Citation {
            document_ref: row.get(0),
            page: row.get(1),
            locator: row.get(2),
            excerpt: row.get(3),
        })
        .collect())
}
