//! Transactional username reservation (USR-003A).
//!
//! [`reserve`] holds one display username for one person. A reservation is
//! idempotent per person: a person who already has an active (not released)
//! reservation gets that row back unchanged, with no duplicate row.
//!
//! Collisions are resolved inside one transaction with
//! `INSERT ... ON CONFLICT (username_key) DO NOTHING`. The base key is tried
//! first, then `key2`, `key3`, and so on. The first inserted row wins. A
//! reservation never uses a birthday or a password and never mutates
//! `person_key`.

use postgres::Client;
use thiserror::Error;

use crate::onboarding::username::{self, NameParts};

/// The lifecycle state of one username reservation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReservationStatus {
    /// Held for the person and not yet claimed or released.
    Reserved,
    /// The person claimed the username.
    Claimed,
    /// The reservation was released and the key is reusable.
    Released,
}

impl ReservationStatus {
    /// The stable database and wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            ReservationStatus::Reserved => "reserved",
            ReservationStatus::Claimed => "claimed",
            ReservationStatus::Released => "released",
        }
    }

    /// Parses a status string from the database.
    fn from_db(value: &str) -> Option<Self> {
        match value {
            "reserved" => Some(ReservationStatus::Reserved),
            "claimed" => Some(ReservationStatus::Claimed),
            "released" => Some(ReservationStatus::Released),
            _ => None,
        }
    }
}

/// One username reservation row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reservation {
    /// The primary key of the reservation row.
    pub reservation_id: i64,
    /// The person the username is held for.
    pub person_id: i64,
    /// The display username, for example `nvan`.
    pub username: String,
    /// The case-insensitive canonical key, for example `nvan`.
    pub username_key: String,
    /// The lifecycle state.
    pub status: ReservationStatus,
}

/// Errors raised by reservation operations.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ReservationError {
    /// No person row exists for the given id.
    #[error("unknown person")]
    UnknownPerson,
    /// The database operation failed.
    #[error("reservation database operation failed: {0}")]
    Database(String),
    /// The username policy rejected the supplied name parts.
    #[error("username policy rejected the name: {0}")]
    Policy(String),
    /// The reservation is not in a state that permits the transition.
    #[error("reservation transition is not allowed: {0}")]
    InvalidTransition(&'static str),
    /// No free suffix remained for the requested key.
    #[error("no free username variant remains")]
    Exhausted,
}

/// The columns returned for one reservation row.
const RESERVATION_COLUMNS: &str = "reservation_id, person_id, username, username_key, status";

/// Builds a [`Reservation`] from a row read with [`RESERVATION_COLUMNS`].
fn row_to_reservation(row: postgres::Row) -> Result<Reservation, ReservationError> {
    let status_text: String = row.get(4);
    let status = ReservationStatus::from_db(&status_text).ok_or_else(|| {
        ReservationError::Database(format!("unknown reservation status {status_text:?}"))
    })?;
    Ok(Reservation {
        reservation_id: row.get(0),
        person_id: row.get(1),
        username: row.get(2),
        username_key: row.get(3),
        status,
    })
}

// The maximum number of collision suffixes attempted before giving up.
const MAX_SUFFIX_ATTEMPTS: u64 = 10_000;

/// Reserves one username for one person.
///
/// The function confirms the person exists and returns any active reservation
/// unchanged, both inside one transaction so a concurrent reservation cannot
/// race the checks. It otherwise builds the display candidate with
/// [`username::candidate_username`] and the canonical key with
/// [`username::key`]. It then inserts in the same transaction, trying the base
/// key and then each numeric suffix until one insert succeeds. The first
/// inserted row is returned.
pub fn reserve(
    client: &mut Client,
    person_id: i64,
    parts: &NameParts,
    actor: &str,
) -> Result<Reservation, ReservationError> {
    let display = username::candidate_username(parts)
        .map_err(|err| ReservationError::Policy(err.to_string()))?;

    let mut transaction = client
        .transaction()
        .map_err(|err| ReservationError::Database(err.to_string()))?;

    // Lock the person row for the reservation transaction. Two concurrent
    // reservations for the same person serialize here: the loser sees the
    // committed reservation below instead of inserting a second one.
    let exists = transaction
        .query_opt(
            "SELECT person_id FROM person WHERE person_id = $1 FOR UPDATE",
            &[&person_id],
        )
        .map_err(|err| ReservationError::Database(err.to_string()))?
        .is_some();
    if !exists {
        return Err(ReservationError::UnknownPerson);
    }

    let existing_sql = format!(
        "SELECT {RESERVATION_COLUMNS}
           FROM onboarding_username_reservation
          WHERE person_id = $1 AND status <> 'released'
          ORDER BY reservation_id
          LIMIT 1"
    );
    let existing_rows = transaction
        .query(&existing_sql, &[&person_id])
        .map_err(|err| ReservationError::Database(err.to_string()))?;
    if let Some(row) = existing_rows.into_iter().next() {
        let reservation = row_to_reservation(row)?;
        transaction
            .commit()
            .map_err(|err| ReservationError::Database(err.to_string()))?;
        return Ok(reservation);
    }

    // The conflict target names the partial unique index created by migration
    // 0008 (`username_key WHERE status <> 'released'`); its predicate must
    // match the index exactly so a released key is reusable.
    let insert_sql = format!(
        "INSERT INTO onboarding_username_reservation
             (username, username_key, person_id, status, reserved_by)
         VALUES ($1, $2, $3, 'reserved', $4)
         ON CONFLICT (username_key) WHERE status <> 'released' DO NOTHING
         RETURNING {RESERVATION_COLUMNS}"
    );

    let mut attempt: u64 = 1;
    loop {
        if attempt > MAX_SUFFIX_ATTEMPTS {
            return Err(ReservationError::Exhausted);
        }
        // The visible alias carries the suffix too, so two people never share
        // the same display username; the canonical key is derived from it.
        let attempt_display = if attempt <= 1 {
            display.clone()
        } else {
            format!("{display}{attempt}")
        };
        let key = username::key(&attempt_display);
        let rows = transaction
            .query(&insert_sql, &[&attempt_display, &key, &person_id, &actor])
            .map_err(|err| ReservationError::Database(err.to_string()))?;
        if let Some(row) = rows.into_iter().next() {
            let reservation = row_to_reservation(row)?;
            transaction
                .commit()
                .map_err(|err| ReservationError::Database(err.to_string()))?;
            return Ok(reservation);
        }
        attempt += 1;
    }
}

/// Transitions one reservation from `reserved` to `claimed`.
///
/// Claiming an already-claimed reservation is idempotent, so a retried request
/// succeeds without a second write.
pub fn claim(client: &mut Client, reservation_id: i64) -> Result<(), ReservationError> {
    let updated = client
        .execute(
            "UPDATE onboarding_username_reservation
                SET status = 'claimed'
              WHERE reservation_id = $1 AND status IN ('reserved', 'claimed')",
            &[&reservation_id],
        )
        .map_err(|err| ReservationError::Database(err.to_string()))?;
    if updated == 0 {
        return Err(ReservationError::InvalidTransition(
            "reservation is not reserved or claimed",
        ));
    }
    Ok(())
}

/// Transitions one reservation from `reserved` to `released` and sets
/// `released_at`.
pub fn release(client: &mut Client, reservation_id: i64) -> Result<(), ReservationError> {
    let updated = client
        .execute(
            "UPDATE onboarding_username_reservation
                SET status = 'released', released_at = now()
              WHERE reservation_id = $1 AND status = 'reserved'",
            &[&reservation_id],
        )
        .map_err(|err| ReservationError::Database(err.to_string()))?;
    if updated == 0 {
        return Err(ReservationError::InvalidTransition(
            "reservation is not reserved",
        ));
    }
    Ok(())
}
