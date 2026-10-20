# PLAN-TRAN-FAMILY-TREE — Family archive experience

**Status:** PROPOSED — documentation and design only; no implementation has started.
**Repository:** https://github.com/imhttran/tran
**Inspected baseline:** `6680ea48d4c0d9c252763e0ec560dcb10ce5dc8e` (2026-10-20).
**Suggested SOP identity:** `plan-tran-family-experience`.
**Design reference:** [Mockup and review guide](../mockups/family-experience/README.md).
**Precedence:** Existing [RBAC requirements](PLAN-TRAN-RBAC.md), [integration follow-up](PLAN-TRAN-RBAC-INTEGRATION-FOLLOWUP.md), and TRAN-002 provenance rules remain authoritative.

## Goal and scope

Deliver four connected screens in the existing Rust + Next.js app:
1. **Family tree:** Browse stored people and relationships.
2. **Ask the archive:** Ask questions and receive permitted, cited answers.
3. **My profile:** Manage one photo and one personal document.
4. **Documents:** Upload general documents and manage admin review and processing.

Use “RAG” for retrieval and grounded answers. No specific Architecture Decision Record (ADR) was identified in the product discussion. An ADR records a design decision; it does not process documents.

The mockup defines visual direction and user flow. It does not prove that a backend capability exists. All mockup identities and source references are synthetic.

## Current baseline and dependencies

- Next.js already has genealogy, Head of Family, session, and query surfaces.
- Axum now registers genealogy record, person, relationship, nomination, and appointment routes. Inspect and reuse these; do not recreate the earlier missing-route assumption.
- Existing uploads ingest directly. The new review workflow must prevent browser or direct-API calls from bypassing admin approval.
- The RBAC follow-up records unresolved acceptance requirements. Reconcile its current code and evidence before implementation. A registered route or merged PR alone does not prove acceptance.
- Keep `DATABASE_URL` and guarded `GENEALOGY_DATABASE_URL` separate. Do not create duplicate genealogy person, relationship, assertion, or appointment tables.
- Coordinate overlapping files with active work. The inspected baseline had PR #11 for Rust medium findings; recheck its state at implementation time.
- New access scopes below are proposed product policy. Record their mapping to existing grants before implementing storage and retrieval. Do not silently expand access.

## Product behavior and permissions

| Capability | Viewer | Editor | Admin |
| --- | --- | --- | --- |
| Read permitted tree, sources, and RAG answers | Yes | Yes | Yes |
| Manage own photo and one personal document | Yes | Yes | Yes |
| Upload general documents | No | Yes | Yes |
| Review, approve, reject, process, or retry general documents | No | No | Yes |
| Link an account to a genealogy person | Request/confirm own link | Request/confirm own link | Review/approve link |
| Change roles or Head of Family appointments | Existing RBAC policy only | Existing RBAC policy only | Existing RBAC policy only |

An account is identified by verified issuer + subject. It is separate from a genealogy person. Users may keep an unlinked profile. Match by a stable person key through a confirmed, admin-reviewed link; never link by name similarity alone. Prevent conflicting active links and audit link changes.

Personal assets default to owner-private. Proposed visibility options are owner-private and authorized-family access. Admin review access must be explicit and audited; an admin role must not silently expose owner-private content.

A personal document may be saved without indexing. If its owner requests indexing or family sharing, create a review submission. Reviewers can inspect only that submitted version under a named review permission. Index it only after approval. Keep a personal photo private until its owner selects family visibility; tree viewers see it only when authorized. Do not assume every admin-uploaded document is self-approved.

## Screen contracts

### Family tree — proposed route /genealogy

- Keep Tree and Catalog views available.
- Render stored parent–child and spouse edges with stable identifiers.
- Support expansion/collapse, pan/zoom, fit-to-view, selection, and person lookup.
- Search names and stored aliases with or without Vietnamese diacritics.
- Bring a selected search result into view, including a collapsed branch.
- Preserve unknown facts and display unverified/disputed relationships distinctly.
- Handle shared ancestors, multiple parents/spouses, disconnected components, and cycles.
- Show stored facts, authorized relatives, review status, and usable source/page references in a details panel.
- Display Head of Family only from an authoritative active appointment.
- On small screens, use a details drawer and retain an accessible catalog alternative.

### Ask the archive — proposed route /ask

- Use a speech-bubble/question icon, visible “Ask” action, and “Ask about your family or find a person…” placeholder.
- Offer person suggestions for name lookup and explicit question submission for RAG.
- Reuse existing Rust retrieval, generation, and citation services.
- Answer only from approved, ready, permitted source versions.
- Link to an existing person only through a supported stable-ID mapping. Otherwise show the citation without inventing a person match.
- Show insufficient evidence, uncertain claims, failures, and loading states.
- Generated answers never update genealogy facts, grants, or appointments.

### My profile — proposed route /profile

- Show the account display label, link state, photo, personal document, and visibility.
- Allow preview, replace, and remove for one photo and one personal document.
- Accept JPEG/PNG photos and the existing PDF/text/Markdown document formats. Retain the current 10 MiB document limit unless a documented requirement changes it; propose a 5 MiB photo limit.
- Validate content server-side, decode/re-encode images, strip image metadata, and reject unsafe or unsupported files.
- Replacement creates a new document version requiring review before indexing.
- Removal and visibility restriction immediately remove effective access to affected originals, derivatives, chunks, answers, and citations. Cleanup may finish asynchronously.
- Do not infer living status or verify family facts from a biography upload.

### Documents — proposed route /documents

- Editors/admins can upload, add title/description, select a category, and propose visibility.
- Separate personal submissions from general documents while sharing review infrastructure.
- General uploads enter Pending review and remain outside retrieval.
- Editors see their submissions and review outcomes. Admins see authorized review queues.
- Admins can preview, set permitted scope, approve/reject with a reason, then start processing.
- Show review status and processing status separately.
- Show uploader label, version, reviewer, timestamps, outcome, and safe failure details.
- Use explicit confirmations for approve/reject/process actions and report success only after the server confirms it.
- A “Ready” result requires a successful ingestion/index commit. Approval alone does not mean indexed.

## State and processing contract

Review: **Pending review → Approved | Rejected**.
Processing: **Not started → Queued → Processing → Ready | Failed**.
Admin retry: **Failed → Queued**.

- An approved, unchanged version with authorized scope is required to queue processing.
- Bind approval to document ID, content fingerprint, version, and access scope.
- A replacement or scope expansion requires a new review. Scope restriction takes effect immediately.
- Use an atomic state transition and idempotent job identity to prevent duplicate processing.
- Before publication, recheck approval, version, removal state, and scope. Cancel stale work.
- Preserve original-source, transcription, translation, and derivative relationships.
- Keep document approval separate from genealogy assertion verification.
- Use existing incomplete-PDF behavior. Show OCR-required status and failed processing where applicable; OCR implementation is outside this scope.
- Treat uploaded text as untrusted source data, not instructions.
- Check authorization at upload, preview/download, review, processing, retrieval, and citation reads.
- Apply access restrictions before retrieval and model calls. UI hiding is not enforcement.
- Audit uploads, version changes, visibility, review, processing, retries, removal, and account/person linking.
- Direct legacy ingest routes must not bypass review. Preserve the documented trusted local CLI boundary separately.

## Implementation phases

| Task | Deliverable | Acceptance |
| --- | --- | --- |
| FAM-001: Reconcile | Current-main review, SOP preflight, route/permission/data-owner matrix, RBAC dependency mapping | Identify implemented behavior and required changes; preserve existing SOP history |
| FAM-002: Contracts and persistence | Typed APIs, additive metadata/version/review/job migrations, asset ownership and visibility | Stable IDs; both database boundaries remain intact; approved version/scope is explicit |
| FAM-003: Upload and review | Profile assets, general submissions, admin review, confirmed account/person link | Roles enforced by Rust; pending/rejected content cannot enter RAG |
| FAM-004: Processing | Durable queue or database job runner using existing ingestion services | Idempotent processing; stale jobs cannot publish; failures/retries are visible |
| FAM-005: Tree | Stored graph, catalog, person panel, search, viewport controls | All authorized components remain reachable; uncertain links stay uncertain |
| FAM-006: Ask | Question UI, person suggestions, cited answers, navigation | Only ready/permitted versions contribute; no fabricated source/person links |
| FAM-007: Integrate and hand off | Shared navigation, responsive layouts, affected checks, public-safe screenshots, implementation PR | Core flows work across roles; required checks pass or an exact blocker is reported |

Run FAM-001 before selecting implementation details. Complete backend authorization contracts before wiring mutations. Use a distinct SOP run and preserve the RBAC follow-up history. This planning PR does not start or close an implementation run.

## Acceptance and verification

Use synthetic data in all committed fixtures, examples, screenshots, and CI.

- Tree: test collapse/search reveal, Vietnamese lookup, cycles, shared ancestors, and disconnected components.
- Profile: test own/other access, private defaults, confirmed linking, replacement/removal, photo handling, and personal-submission review access.
- Documents: exercise editor upload → admin review → process → ready → cited answer against real APIs and disposable databases.
- Negative cases: viewer general upload, editor approval, direct-ingest bypass, private preview/citation leaks, pending/rejected retrieval, stale approval, concurrent/retried jobs, and processing failure.
- Verify permission changes/removal prevent access to stale indexes and cited content.
- Use focused Rust and browser checks during implementation. Reuse passing checks when inputs are unchanged.
- At the integration milestone, run required repository Rust/Next.js gates and the affected real-browser flow. Do not replace backend verification with mocked responses alone.
- Record checks actually executed. Do not claim visual or runtime acceptance from a design artifact.

## Stop conditions and exclusions

- Continue through implementation and open a PR. Stop only for missing access, a required policy decision that cannot be resolved from the requirements, or a failing required gate that cannot be repaired within scope.
- Continue independent work when a deployment prerequisite is unavailable.
- Do not merge or publicly deploy without explicit authorization.
- Do not commit real family records, private documents, source hashes/excerpts, or credentials. Keep `docs/tran/` untracked.
- Do not appoint a Head of Family or infer a successor. Eligibility still requires verified living status, male status, first-born male birth order, and membership in the third branch. Unknown/deceased status requires the separate succession rule.
- Do not add automatic fact promotion, face recognition, photo-based kinship, OCR, or a second RAG pipeline.

## Agent handoff

Implement this plan end to end in a new feature branch/worktree from current main.
Read repository instructions and the applicable installed SOP workflow.
Reconcile current RBAC work and reuse existing APIs, schemas, and Rust services.
Use the linked mockup as visual direction; every production action needs real server confirmation.
Run affected checks and required integration gates without repeating unaffected passing checks.
Open an implementation PR with screenshots, results, and exact limitations. Do not merge.
