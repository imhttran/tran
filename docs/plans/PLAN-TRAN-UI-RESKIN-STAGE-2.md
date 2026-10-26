# Implementation Plan

## Project

Tran family archive — stage-two UI reskin: family tree and person details

## Summary

Apply the stage-one neutral/jade design to the family tree (`/genealogy`), the
family profile (`/family/[personKey]`), and the shared person cards and detail
panels those routes use.

The tree keeps its data model, layout, camera, pan/zoom/fit behaviour, search,
selection, focus traversal, full-screen mode, help, personal-history dialog, and
every evidence and privacy rule. The profile page keeps its approved-only data
contract and every test identifier. The work is presentation and interaction
polish, not a new genealogy feature.

### What must not change

- Routes, API endpoints, request shapes, authentication, RBAC, publication
  rules, evidence rules, and approval behaviour.
- Tree data, relationship meaning, card positions, layout constants, camera
  semantics, popup placement, search normalization, focus traversal, and the
  existing pointer/keyboard interactions.
- Person-detail privacy: no relationship, date, portrait, biography, evidence,
  or review state may be inferred or invented. Only values the API returns may
  be shown.
- Every existing `data-testid` used by the family-tree, family-dates,
  family-history, family-profile, and shared-shell suites. In particular:
  `genealogy-tree`, `genealogy-loading`, `genealogy-empty`,
  `genealogy-no-links`, `genealogy-records`, `genealogy-record`,
  `family-tree-canvas`, `family-canvas`, `family-search`,
  `family-search-results`, `family-fit-all`, `family-focus-selected`,
  `family-back`, `family-show-entire-tree`, `family-canvas-toggle`,
  `family-collapse`, `family-navigation-hint`, `family-help`,
  `family-help-panel`, `family-help-close`, `family-person`,
  `family-person-life`, `family-person-history`, `family-person-details`,
  `family-popup-close`, `family-popup-lifespan`, `family-popup-summary`,
  `family-popup-notice`, `family-popup-truncated`, `family-show-parents`,
  `family-show-children`, `family-no-children`, `family-no-parents`,
  `family-relative`, `family-relation`, `family-assertions`,
  `family-social-link`, `family-focus`, `personal-history-dialog`,
  `personal-history-close`, `personal-history-loading`,
  `personal-history-error`, `personal-history-empty`,
  `personal-history-passage`, `personal-history-source-link`,
  `family-profile`, `family-profile-loading`, `family-profile-signin`,
  `family-profile-signin-button`, `family-profile-not-found`,
  `family-profile-unavailable`, `family-profile-retry`,
  `family-profile-back`, `family-profile-dates`,
  `family-profile-date-*`, `family-profile-history`,
  `family-history-passage-*`, `family-history-source`,
  `family-profile-photos`, `family-profile-photo-*`,
  `family-profile-section-*`, `family-profile-field-*`,
  `family-profile-documents`, `family-profile-document-*`, and
  `family-profile-empty`.
- The popup stays about 420px wide on desktop, keeps its 180ms opacity/transform
  transition, respects reduced motion, scrolls its content, and stays within
  12px of a narrow viewport.
- The exact navigation-hint text and help-panel copy that the browser suite
  asserts.
- `scripts/web-e2e.sh` and the stage-one RAG evidence are untouched. The stage
  one live-RAG timeout remains an outstanding verification item; do not rerun
  RAG checks for this visual work.

### Execution boundaries

SOP's command policy may treat build, test, migration, and browser commands as
approval-gated. The implementing agent writes code and documentation only. It
does not run `npm`, `cargo`, `psql`, Playwright, or a browser. The SOP
validation stage runs the configured Rust gates; the operator runs the focused
web checks and captures screenshots after the edit.

Only the files named by each task may change. Do not touch Rust sources, API
routes, migrations, `.agent-sdlc/` state, `.zed/settings.json`,
`generate_audio.py`, or `.playwright-mcp/`. Use synthetic names, records, and
citations in tests, screenshots, and reports.

## Design specification

Reuse `web/src/app/globals.css` tokens and the stage-one shell. Do not introduce
a second accent, a new font, a new dependency, or a page-level gradient. The
tree canvas is the primary surface; surrounding controls are quiet and compact.

### Tree workspace

- Put the canvas in a single bordered surface with a quiet neutral background.
  The canvas should dominate the page; the toolbar should read as a compact
  utility bar, not a row of primary calls to action.
- Use a neutral surface for the toolbar. Only one action is primary (prefer
  **Fit all**); zoom buttons are compact icon controls; navigation controls are
  secondary/ghost buttons with visible labels.
- Group controls into search, view, and navigation groups. Keep the search field
  first and full-width on mobile. The whole toolbar must wrap or scroll inside
  its own surface; the page itself must not overflow.
- Increase the usable canvas height on mobile (roughly 420px) while preserving
  the camera viewBox and pan/zoom semantics.
- Keep the legend compact and legible; use line samples, not just colour bars.

### Person cards

- Add a fallback avatar: a circle containing the first initial of the person's
  display name. There is no portrait URL in the tree payload; do not invent one.
  The initial is decorative; the card's accessible name remains
  `Select <person name>`.
- Keep the 190×76 card geometry and existing layout constants. Place the avatar
  left, then the readable name, evidence status, lifespan, and the existing
  personal-history link. Truncate only the visible name; the accessible name is
  always the full label.
- Selected state: jade border and soft jade fill. Focus state: a visible jade
  ring/stroke that does not depend on hover. Hover: a quieter border change.
  Status is always also a word.
- Relationship lines use rounded caps, a readable weight in both themes, and
  the existing status distinctions: verified solid, needs review dashed,
  unverified dotted. Do not change relationship meaning.

### Person detail panel

Keep the existing anchored popup and its behaviour, but make the content read as
a clear person record:

1. Header: name, evidence badge, lifespan, recorded child/parent counts.
2. Actions: View profile, Show parents, Show children.
3. Personal history action when the person has published history, using the
   existing history dialog. Do not open history automatically.
4. Relationships: recorded relationship labels, other person, status, and
   evidence links.
5. Recorded facts: stored assertions with review status and evidence links.
6. Sources: keep each evidence link attached to the fact or relationship it
   supports; use a consistent “Sources”/citation treatment rather than a new
   aggregate source list. The source dialog remains metadata-only.

Do not add photographs to the tree popup: the tree payload has none. The
profile page is where approved photographs are presented.

### Family profile

- Keep the existing page header, h1 display name, back link, and approved-only
  data rules.
- Add a two-column desktop layout: a compact person summary (approved portrait
  or initials fallback, verified dates, and derived counts for approved facts,
  photographs, documents, and history) beside a main content column. Stack to
  one column on mobile.
- The summary portrait is the first approved profile photo when one exists;
  otherwise show initials. A person with no media must render no `<img>`, as the
  privacy suite asserts.
- Organize the main column into clear sections, in this order:
  1. About (approved facts and prose)
  2. Personal story
  3. Family memories
  4. History from the archive (source wording and document/page metadata)
  5. Photographs
  6. Shared documents
- Keep long prose at a readable measure (about 68ch), render whitespace, and
  keep the existing approved-version/date provenance lines.
- Provide a quiet “Explore relationships in the family tree” link/back path in
  the summary; relationships are not in the profile payload and must not be
  fabricated.
- Loading, empty, unavailable, denied, expired, and not-found states use
  consistent state panels with a clear title, short explanation, and the
  existing actions/test ids. Keep the retry action.

### Motion and accessibility

- 120–150ms transitions on named properties; 180ms for the anchored popup;
  honour `prefers-reduced-motion`.
- Visible focus on every control, card, link, and popup action.
- Buttons and card targets meet the stage-one target sizes; the tree's dense
  toolbar controls may use 38–40px targets.
- No hover-only interaction. Keep the existing keyboard paths: Tab, Enter/Space,
  Escape, arrow-free card traversal, and the full-screen Escape behaviour.

## UI-201 — Tree workspace, compact toolbar, and state panels

Files you may change:

- `web/src/components/FamilyTree.tsx`
- `web/src/app/globals.css`
- `web/e2e/family-tree.spec.ts`

Requirements:

- Restructure the existing toolbar into a search group plus view and navigation
  groups. Preserve the existing button actions, accessible names, test ids, and
  the exact navigation-hint text. Replace the `＋`/`−` glyphs with inline SVG
  zoom icons while keeping the accessible names **Zoom in** and **Zoom out**.
  Keep one primary action (Fit all) and make the other controls secondary.
- Keep the search results dropdown keyboard-operable and preserve the
  accent-insensitive search and alias behaviour.
- Give the canvas a larger, quieter surface and a more useful mobile height. Keep
  the SVG `viewBox`, pan, wheel/button zoom, Fit all, Focus selected, Back, Show
  entire tree, full-screen, collapse, search selection, and personal-history
  interactions unchanged.
- Add a distinct unavailable state for a 5xx/network tree failure with a retry
  action, separate from the existing 401/403 denial and 404 no-access states.
  Keep `genealogy-loading`, `genealogy-empty`, and `genealogy-no-links`
  behaviour and text. Preserve the denial view for authorization outcomes.
- Style the legend as a compact line/status key and keep the catalog view
  readable. Do not change its data or selection behaviour.
- Ensure the page has no horizontal overflow at 390px and 320px; only the
  toolbar may scroll horizontally when it cannot wrap.
- Add two browser tests to `family-tree.spec.ts`:
  - a 503 tree response shows the retryable unavailable state, and retry after
    a 200 response renders the tree;
  - a 390px viewport has no page-level horizontal overflow and the search and
    principal controls remain reachable.

### Dependencies

None

### Deliverables

- The redesigned tree workspace, toolbar, canvas, legend, and state panels.
- The two new tests.

### Acceptance Criteria

- Search, selection, pan, wheel/button zoom, Fit all, Focus selected, Back, Show
  entire tree, full-screen, collapse, and personal history work as before.
- The toolbar is grouped and compact, the canvas is the dominant surface, and
  mobile controls remain reachable without page overflow.
- A 503/network failure is distinct from a denial and offers a working retry.
- All existing family-tree tests pass, including the exact navigation hint,
  camera, popup, help, touch, and reduced-motion assertions.

## UI-202 — Person cards, avatars, and the detail panel sections

Files you may change:

- `web/src/components/FamilyTree.tsx`
- `web/src/app/globals.css`
- `web/src/lib/person-display.ts` (new, optional; a small shared initial helper)
- `web/e2e/family-tree.spec.ts`

Requirements:

- Add the initials avatar to every SVG person card. Use the first letter of the
  display name, preserve the full accessible label, and keep the card geometry
  and layout constants. Do not infer a portrait.
- Improve name/status/lifespan readability and spacing within the fixed card
  size. Keep the existing selected, focus, hover, click, double-click, drag,
  and keyboard behaviour.
- Organize the anchored popup into the header, actions, relationships, recorded
  facts, and sources sections described above. Add a **Personal history** action
  in the popup when `history_passages > 0`, wired to the existing history
  dialog; do not open it automatically and do not change the card's existing
  personal-history link.
- Keep the popup width, placement, motion, scrolling, reduced-motion, Escape,
  outside-click, and focus-restoration behaviour. Keep the existing evidence
  links, source dialog, review-status labels, and privacy rules.
- Style relationship lines and the legend for readable light/dark contrast.
- Add a browser test for the visible initials/avatar and the popup's
  relationships/facts/sources sections, using the existing synthetic tree
  fixture. Do not add a new data shape.

### Dependencies

- UI-201

### Deliverables

- The avatar cards, relationship styling, and reorganized person detail panel.
- The new card/detail test.

### Acceptance Criteria

- Cards show a readable initial avatar and the full name/status/lifespan; the
  accessible label and card geometry are unchanged.
- Selected and keyboard-focus states are visibly distinct in both themes.
- The popup has clear relationships, facts, and sources sections, keeps all
  existing evidence/source actions, and retains its width/motion/placement and
  narrow-screen scrolling.
- A person with published history can open the existing history dialog from the
  popup without breaking the card link or focus restoration.

## UI-203 — Family profile summary and section layout

Files you may change:

- `web/src/components/FamilyProfileView.tsx`
- `web/src/app/globals.css`
- `web/src/lib/person-display.ts` (if created by UI-202)
- `web/e2e/family-profile-visual.spec.ts` (new)

Requirements:

- Add the two-column desktop layout and the person summary described above:
  approved photo or initials fallback, verified dates, derived approved counts,
  and a quiet relationship-exploration link to `/genealogy`. Stack on mobile.
- Keep the existing h1, subtitle, back link, and all approved-only data rules.
  Do not add contact fields, private values, drafts, or new API requests.
- Reorder/organize the main sections as About, Personal story, Family memories,
  History from the archive, Photographs, and Shared documents. Preserve the
  existing headings, descriptions, provenance lines, test ids, and source
  metadata. Keep long prose at a readable measure.
- Make the loading, empty, unavailable, denied, expired, and not-found states
  consistent with the stage-one state panels and preserve the existing retry
  and sign-in actions.
- Add a mocked browser suite `family-profile-visual.spec.ts` that:
  - renders a synthetic approved profile with a Vietnamese display name, dates,
    history, a synthetic photo, and a document;
  - asserts the summary initial/photo fallback, section order/ids, and that the
    approved values render;
  - verifies a 390px viewport has no horizontal overflow;
  - verifies the empty and unavailable states remain distinct.

### Dependencies

- UI-202

### Deliverables

- The redesigned family profile layout and states.
- The new mocked profile visual suite.

### Acceptance Criteria

- The profile uses the stage-one design and is visually related to the tree
  person cards.
- Approved facts, history, photographs, and documents are organized into clear
  sections; relationships are not invented and point back to the tree.
- A person with no media renders no `<img>`; approved media still renders.
- The page has no horizontal overflow at 390px and remains keyboard navigable.
- Existing family-profile, family-dates, family-history, and privacy tests
  retain their test ids and behaviour.

## UI-204 — Stage-two evidence, screenshots, and PR notes

Files you may change:

- `web/scripts/capture-ui-reskin-stage2.mjs` (new)
- `docs/reports/ui-reskin/stage-2/README.md` (new)
- `docs/reports/ui-reskin/stage-2/before/` and
  `docs/reports/ui-reskin/stage-2/after/` (operator screenshots)

Requirements:

- The capture tool uses the installed Playwright dependency and intercepts only
  the browser-facing session, tree, person-detail, family-history, family-profile,
  and family-media endpoints. It needs no database, Ollama, or credentials.
- Capture desktop/mobile, light/dark tree and profile screens, the open person
  popup, and the empty/unavailable states. Use 1440×1024 and 390×844.
- Use synthetic Vietnamese names, records, dates, media, and citations only.
- Write a report with the before/after images, change summary, exact verification
  commands and results, known limitations, and the remaining screens for the
  next stage. Record the stage-one live-RAG timeout as outstanding and do not
  rerun unrelated RAG checks.
- Do not claim a command ran unless its result was recorded. Use `NOT RUN` when
  a check was skipped.

### Dependencies

- UI-203

### Deliverables

- The capture tool, report, and operator-produced screenshots.

### Acceptance Criteria

- The capture tool is self-contained and works against a running local app.
- The report embeds tree, popup, profile, mobile, dark-theme, and state evidence.
- No real family data or secret appears in any screenshot or report.
- The report records verification results and remaining limitations honestly.
