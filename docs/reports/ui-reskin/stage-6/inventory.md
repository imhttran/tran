# UI-601 — Head of Family contract inventory

Recorded before editing. Every item below is preserved by UI-601.

## API calls (unchanged)

| Method | Path | Payload | Header |
| --- | --- | --- | --- |
| GET | `/api/genealogy/head-of-family` | — (`cache: "no-store"`) | — |
| GET | `/api/genealogy/records` | — (`cache: "no-store"`) | — |
| GET | `/api/genealogy/nominations` | — (`cache: "no-store"`, admin only) | — |
| POST | `/api/genealogy/nominations` | `{ record_id, candidate_label }` | `x-csrf-token` from `rag_csrf` cookie |
| POST | `/api/genealogy/nominations/{id}` | `{ approve: true }` or `{ approve: false }` (reject) | `x-csrf-token` |
| POST | `/api/genealogy/appointments/{id}` | `{ reason }` | `x-csrf-token` |
| GET | `/api/auth/session` | — (shell session read) | — |

## Result kinds handled

- `denied` (HTTP 403)
- `no_access` (HTTP 404)
- `expired` (HTTP 401, carries `signInUrl`)
- `allowed` with `catalog: "unavailable"`
- loading (`result === null`)
- `allowed` with populated conclusions
- `allowed` with empty conclusions

## Role gates

- `canNominate(role)`: editor and admin
- `canApproveOrRevoke(role)`: admin only
- Viewer: no controls (`HeadOfFamilyActions` returns `null`).

## data-testid values

Page: `hof-loading`, `hof-mutation-error`.

`HeadOfFamilyView`: `hof-denied`, `hof-no-access`, `hof-expired`,
`hof-catalog-unavailable`, `hof-view`, `hof-head-name`,
`hof-appointment-count`, `hof-empty`, `hof-conclusion`, `hof-cited-head`,
`hof-record-id`, `hof-appointment-id`, `nomination-input`, `nominate-button`,
`approve-button`, `revoke-reason-input`, `revoke-button`, `confirm-dialog`,
`confirm-button`, `cancel-button`.

`HeadOfFamilyActions`: `hof-actions`, `hof-actions-error`, `hof-record-select`,
`hof-nomination-label`, `hof-nominate-submit`, `hof-no-nominations`,
`hof-pending-nomination`, `hof-approve-submit`, `hof-reject-submit`,
`hof-confirm`, `hof-confirm-yes`, `hof-confirm-cancel`.

Shared evidence components: `evidence-verified`, `evidence-unverified`,
`evidence-needs_review`, `evidence-links`, `citation-button`, `source-details`,
`source-metadata-only`.

## Stage-3 tokens and panel classes reused

Tokens: `--surface`, `--card`, `--line`, `--line-strong`, `--accent`,
`--accent-soft`, `--focus-ring`, `--ok`, `--warn`, `--err`, `--ok-soft`,
`--warn-soft`, `--err-soft`, `--info-soft`, `--radius-*`, `--space-*`.

Classes: `state-panel`, `state-panel-actions`, `state-panel-loading`,
`state-panel-expired`, `state-panel-unavailable`, `state-panel-empty`,
`state-panel-not-found`, `badge`, `status-chip`, `button-secondary`.
