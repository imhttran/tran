# HOF-901 — Searchable Head of Family person selector

**Status:** Implemented on `feat/hof-901-searchable-person-selector`.

## Cause

The Head of Family nomination form loaded `/api/genealogy/records` once and
rendered the result in a fixed `<select>`. The handler called
`list_records(&mut client, 100, 0)`; `list_records` orders by `person_key` with
`LIMIT 100 OFFSET 0` and has no name filter. An existing archive person who
sorted after the first 100 keys was therefore absent, and free-form text could
not select a stable identity.

## Changes

- Added read-only `GET /api/genealogy/person-search?q=` authorized by
  `Operation::GenealogyRead`. It uses the existing deterministic
  `person_search::search` (catalog load + diacritic-insensitive ranking) across
  all authorized catalog records, bounded by the existing search caps.
- The search projection now returns `person_key`, `display_name`, `generation`,
  `evidence_status`, and:
  - `branch`, only from a current **verified** `branch_claim` joined to
    `family_branch`;
  - `birth_year`, only from a current **verified** `birth`/`birth_date`
    assertion's structured start year.
  No missing detail is inferred.
- Replaced the fixed dropdown in `HeadOfFamilyActions.tsx` with a searchable
  combobox:
  - 250 ms debounce and a request sequence guard so stale responses cannot
    replace newer results;
  - loading, no-results, and error states;
  - ArrowUp/ArrowDown/Enter/Escape keyboard support, combobox/listbox roles,
    labels, and active-descendant wiring;
  - match rows show name, branch, birth year, and generation when recorded;
  - selection requires an existing record; Nominate stays disabled until one is
    selected;
  - selection stores the stable `person_key` as `record_id`;
  - editing the search text clears the selection and the Candidate label;
  - selecting auto-populates the Candidate label from the display name.
- Kept nomination, cited-evidence approval, eligibility, authorization, and
  audit behavior unchanged.
- Boxed the `PersonResult::Found` candidate so the enlarged result enum remains
  clippy-clean without changing the read result.

## Tests

- `web/e2e/head-of-family-visual.spec.ts`: 13 tests, including search across
  similar names, keyboard selection, branch/birth-year display, auto-filled
  label, clear-on-edit, blocked unselected submission, and loading/empty/error
  states.
- `web/e2e/head-of-family-real.spec.ts`: real API flow searched and selected the
  synthetic person, then nominated, approved, and revoked.
- `web/e2e/genealogy-api.spec.ts`: anonymous search is denied; an authorized
  partial-name search returns the expected person.
- `web/e2e/experience.spec.ts`: navigation expectations updated for the
  photograph routes merged from PR #116. This is a stale-test fix unrelated to
  selector behavior.

## Verification

`API_PORT=8081 API_URL=http://127.0.0.1:8081 ./scripts/validate.sh` — **PASS**

- `cargo build`, `cargo test`, `cargo fmt --check`, `cargo clippy -D warnings`
- `make web-check` (typecheck + production build)
- `make web-e2e` — **290/290 Playwright tests**
- `make transition-evidence`
- `git diff --check` — **PASS**

Focused runs: HOF visual 13/13, HOF real 1/1, genealogy API 6/6, experience
39/39.

## Limitations

- Search is bounded by the existing catalog cap (`MAX_SEARCH_PEOPLE = 2000`).
  The API returns `catalog_truncated`; the selector shows a hint to refine the
  name when the cap is reached.
- Branch and birth year appear only when a current verified assertion records
  them. The selector never infers or fills missing details.
- Search results are bounded by the existing candidate cap; refining the name is
  the supported path to see more namesakes.
- No new eligibility or permission behavior was added. A match is a candidate,
  not an appointment or a grant.
- The disposable test databases were recreated before the final gate because
  branch-grant events are append-only; port 8081 was used because an unrelated
  `tunnel-client` occupies 8080.

## Governance

- Ran as independent ad-hoc SOP task `HOF-901` in an isolated worktree.
- The active archive-photographs plan, AP-106, and the fam-002 worktree were not
  modified.
- SOP stopped after a backend-only partial implementation because its tool
  allow-list rejects the configured validation command. The operator completed
  the web selector, E2E coverage, full validation, and delivery. No approval
  gate was bypassed.
