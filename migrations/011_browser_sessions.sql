-- FAM-RAG-03: server-side browser sessions for automatic renewal.
--
-- The browser keeps only an opaque, HttpOnly `rag_sid` cookie. The refresh and
-- ID tokens live here, encrypted at rest with AES-256-GCM by the Rust API.
-- This table is additive: no existing row is modified, and the pre-renewal
-- behavior (cookie-only sessions bounded by the ID token) remains when the
-- renewal configuration is absent.
--
-- `session_id_hash` is the SHA-256 hex of the opaque cookie value. The opaque
-- value itself is never stored, so a database dump cannot be used as a session
-- credential.
--
-- `refresh_generation` and `renewal_lease_until` implement concurrent renewal:
-- a claimant increments the generation and takes a short lease, and only that
-- generation may write the rotated credentials.
--
-- Idle and absolute expiries are enforced by the store; sign-out sets
-- `revoked_at` and clears the stored credentials.

CREATE TABLE IF NOT EXISTS browser_session (
    session_id_hash      TEXT PRIMARY KEY,
    issuer               TEXT NOT NULL,
    subject              TEXT NOT NULL,
    id_token             BYTEA NOT NULL,
    id_token_exp         TIMESTAMPTZ NOT NULL,
    refresh_token        BYTEA NOT NULL,
    refresh_generation   BIGINT NOT NULL DEFAULT 0,
    renewal_lease_until  TIMESTAMPTZ,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    idle_expires_at      TIMESTAMPTZ NOT NULL,
    absolute_expires_at  TIMESTAMPTZ NOT NULL,
    revoked_at           TIMESTAMPTZ,
    last_error           TEXT,
    CONSTRAINT browser_session_hash_not_blank
        CHECK (length(btrim(session_id_hash)) > 0),
    CONSTRAINT browser_session_issuer_not_blank
        CHECK (length(btrim(issuer)) > 0),
    CONSTRAINT browser_session_subject_not_blank
        CHECK (length(btrim(subject)) > 0)
);

CREATE INDEX IF NOT EXISTS browser_session_expiry_idx
    ON browser_session (idle_expires_at, absolute_expires_at)
    WHERE revoked_at IS NULL;

CREATE INDEX IF NOT EXISTS browser_session_absolute_idx
    ON browser_session (absolute_expires_at)
    WHERE revoked_at IS NULL;
