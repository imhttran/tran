-- Normalize every stored onboarding username. OIDC subjects and person names
-- are unchanged. PostgreSQL 13+ with UTF-8 supports Unicode normalization.
BEGIN;

CREATE OR REPLACE FUNCTION onboarding_username_normalize(value TEXT)
RETURNS TEXT LANGUAGE SQL IMMUTABLE STRICT PARALLEL SAFE AS $$
    SELECT translate(lower(regexp_replace(normalize(value, NFD),
        U&'[\0300-\036F]', '', 'g')), 'đ', 'd')
$$;

-- The existing partial unique index rejects active-name collisions. The entire
-- migration rolls back on a collision; it never merges accounts or drops rows.
UPDATE onboarding_username_reservation
   SET username = onboarding_username_normalize(username),
       username_key = onboarding_username_normalize(username)
 WHERE username IS DISTINCT FROM onboarding_username_normalize(username)
    OR username_key IS DISTINCT FROM onboarding_username_normalize(username);

ALTER TABLE onboarding_username_reservation
    DROP CONSTRAINT IF EXISTS onboarding_username_normalized;
ALTER TABLE onboarding_username_reservation
    ADD CONSTRAINT onboarding_username_normalized CHECK (
        username = onboarding_username_normalize(username)
        AND username_key = username
    );

COMMIT;
