# UI reskin — stage 2

**Status:** Implemented; awaiting review. **Scope:** the family tree at
`/genealogy`, the family profile at `/family/[personKey]`, and the shared person
cards and detail panels those routes use. The stage-one shell, tokens,
typography, and primitives are reused unchanged.

## Direction

Stage two applies the stage-one jade-and-neutral system to the tree workspace and
the person profile. It does not introduce a second accent, a new dependency, or a
new screen. The tree canvas keeps visual priority: the toolbar is compact and
grouped, the canvas is the largest surface, and the detail popup floats over it
without disturbing the layout.

The tree keeps every existing behaviour: data, relationship meaning, camera
(`CARD_WIDTH` 190, `CARD_HEIGHT` 76), pan, zoom, fit, focus, expand/collapse,
keyboard selection, the ~420 px popup with its 180 ms fade, and reduced-motion
handling. No relationship is inferred and no family information is invented: the
cards show only what the tree payload already carries, and the profile shows only
approved, family-visible values.

## Before

The baseline was captured against the pre-change app with the same synthetic
intercepts. It used ungrouped flat toolbar buttons, no portrait/initial avatars,
inline card text, and a single-column profile without a summary.

| Desktop tree                                                 | Desktop popup                                                  | Desktop profile                                                    |
| ------------------------------------------------------------ | -------------------------------------------------------------- | ------------------------------------------------------------------ |
| ![Before: desktop tree](./before/desktop-light-tree.png)     | ![Before: desktop popup](./before/desktop-light-popup.png)     | ![Before: desktop profile](./before/desktop-light-profile.png)     |
| ![Before: dark tree](./before/desktop-dark-tree.png)         | ![Before: dark popup](./before/desktop-dark-popup.png)         | ![Before: dark profile](./before/desktop-dark-profile.png)         |
| ![Before: mobile tree](./before/mobile-light-tree.png)       | ![Before: mobile popup](./before/mobile-light-popup.png)       | ![Before: mobile profile](./before/mobile-light-profile.png)       |

## After

### Tree workspace

| Desktop light                                                           | Desktop dark                                                          |
| ----------------------------------------------------------------------- | --------------------------------------------------------------------- |
| ![After: desktop light tree](./after/desktop-light-tree.png)            | ![After: desktop dark tree](./after/desktop-dark-tree.png)            |
| ![After: desktop light popup](./after/desktop-light-popup.png)          | ![After: desktop dark popup](./after/desktop-dark-popup.png)          |

The toolbar groups search, zoom, view, and navigation controls; only **Fit all**
is primary. Person cards show an initials avatar, a readable name, the existing
evidence status, and dates. Relationship lines keep a distinct dash and colour
per evidence status in both themes.

### Family profile

| Desktop light profile                                                  | Desktop dark profile                                                 |
| ---------------------------------------------------------------------- | -------------------------------------------------------------------- |
| ![After: desktop light profile](./after/desktop-light-profile.png)     | ![After: desktop dark profile](./after/desktop-dark-profile.png)     |

The profile becomes a summary aside plus one main column, in a fixed order:
About, Personal story, Family memories, Dates, History from the family archive,
Photographs, and Shared documents. The summary shows the approved photo or
initials fallback, reviewed dates, counts, and a link back to the tree.

### Mobile

| Light tree                                                    | Light popup                                                     | Light profile                                                     |
| ------------------------------------------------------------- | --------------------------------------------------------------- | ----------------------------------------------------------------- |
| ![After: mobile tree](./after/mobile-light-tree.png)          | ![After: mobile popup](./after/mobile-light-popup.png)          | ![After: mobile profile](./after/mobile-light-profile.png)        |

| Dark profile                                                   |
| -------------------------------------------------------------- |
| ![After: mobile dark profile](./after/mobile-dark-profile.png) |

Controls wrap inside the toolbar at 390 px, the canvas scrolls internally, and
the page itself does not overflow horizontally.

### Empty and unavailable states

| Tree unavailable                                                       | Profile unavailable                                                          |
| ---------------------------------------------------------------------- | ---------------------------------------------------------------------------- |
| ![After: tree unavailable](./after/desktop-light-tree-unavailable.png) | ![After: profile unavailable](./after/desktop-light-profile-unavailable.png) |

| Tree empty                                                       | Profile empty                                                          |
| ---------------------------------------------------------------- | ---------------------------------------------------------------------- |
| ![After: tree empty](./after/desktop-light-tree-empty.png)       | ![After: profile empty](./after/desktop-light-profile-empty.png)       |

A 5xx or network failure now shows a retryable *unavailable* state distinct from
an *empty* archive. An invalid or self-inconsistent tree payload is still treated
as a denial, not as a retryable outage.

All screenshots use synthetic labels, records, dates, media, and citations. The
captures use `Nguyễn Văn An`, `Trần Thị Bình`, and other invented people to verify
Vietnamese diacritics; no real family data or secret appears.

## What changed

- `web/src/components/FamilyTree.tsx`: grouped, responsive toolbar (search, zoom,
  view, navigation); the single primary **Fit all** action; initials avatars with
  a stable glyph fallback; popup sections for header, relationships, facts,
  sources, and the existing Personal history action; and a retryable
  *unavailable* panel for 5xx/network failures.
- `web/src/components/FamilyProfileView.tsx`: a summary aside and an ordered
  main column (About, Personal story, Family memories, Dates, History,
  Photographs, Shared documents); approved photo or initials fallback; and
  consistent `state-panel` states for loading, expired session, denied,
  not-found, unavailable, and empty.
- `web/src/lib/person-display.ts` (new): `initialLetter()` with a safe `?`
  fallback. It is display-only and never infers a person.
- `web/src/app/globals.css`: tree toolbar, canvas, card, avatar, popup, legend,
  unavailable, profile summary, and profile section styles built on the stage-one
  tokens; mobile wrapping and reduced-motion handling preserved.
- `web/e2e/family-tree.spec.ts`: six added cases — retryable 503, 390 px
  overflow, avatar, avatar fallback, popup sections, and popup history.
- `web/e2e/family-profile-visual.spec.ts` (new): four fully mocked cases — the
  approved summary and ordered sections, initials plus zero `<img>` when no media
  exists, 390 px no-overflow, and distinct empty versus unavailable states.
- `web/scripts/capture-ui-reskin-stage2.mjs` (new): reproducible synthetic
  screenshot tool for tree, popup, profile, mobile, dark, and state evidence.
- `docs/plans/PLAN-TRAN-UI-RESKIN-STAGE-2.md`: the stage-two plan.

No route, API, authentication rule, permission, retrieval rule, tree library, or
document-approval behaviour changed.

## Verification

| Check                                                                     | Result                                                    |
| ------------------------------------------------------------------------- | --------------------------------------------------------- |
| `./scripts/web-control.sh typecheck` (`tsc --noEmit`)                     | **PASS**                                                  |
| `./scripts/web-control.sh build` (`next build`, lockfile Next 16.4.2)     | **PASS**                                                  |
| `npx playwright test e2e/family-profile-visual.spec.ts`                   | **PASS — 4/4** (1.6 s)                                    |
| `npx playwright test e2e/family-tree.spec.ts e2e/family-dates.spec.ts`    | **PASS — 65/65** (28.5 s)                                 |
| `npx playwright test e2e/experience.spec.ts` (shared shell)               | **PASS — 34/34** (12.0 s)                                 |
| Cross-route shell smoke, 13 routes × desktop/mobile                       | **PASS — 26/26**, 0 horizontal overflow, 0 page errors    |

The cross-route smoke visited `/profile`, `/genealogy`,
`/family/[personKey]`, `/head-of-family`, `/branch-review`, `/documents`,
`/ask`, the five admin routes, and `/signin` at 1440×1024 and 390×844. It is an
ad-hoc check (not committed); it found no page error and no horizontal overflow.

The tree suite covers search, selection, pan, zoom, fit, expand/collapse, the
compact navigation hint, help, touch taps, popup bounds, focus restoration, and
reduced motion. The profile suite covers the summary, section order, approved
values, the media-free case, mobile overflow, and the empty/unavailable split.
Vietnamese names render correctly in every capture.

### Not rerun

- **Rust gates** (`cargo build`, `cargo test`, `cargo fmt --check`,
  `cargo clippy`): **NOT RUN** for this stage. No Rust source, migration, or
  contract changed. The stage-one run remains the recorded Rust result.
- **Live-browser RAG** (`e2e/rag.spec.ts`): **OUTSTANDING from stage one** — the
  local chat model returned 503 after a 224-second generation timeout. It is not
  related to this visual change and was deliberately not rerun. It should be
  rerun with a healthy local model before a full live record is claimed.

## Known limitations

- The in-app theme still follows the system preference only; there is no manual
  theme switch in this stage.
- The profile shows no relationship graph of its own; it links back to the family
  tree, which remains the place relationships are explored. This avoids inferring
  or duplicating relationship data.
- The tree still renders initials rather than portraits, because the tree payload
  carries no portrait URL. Approved photographs appear on the profile only.
- Persisted SOP detail: UI-203 was recovered by hand after a tool overwrote
  `web/src/app/globals.css`. The stylesheet was restored from git and the stage
  rules were re-appended; typecheck, build, and the profile suites pass. The
  plan's UI-203/UI-204 tasks were completed externally by the operator rather
  than by a fresh model run, and this report records that honestly.
- Other screens inherit the shared tokens and primitives but were not
  individually redesigned.

## Next reskin stage

The remaining screens, in suggested order:

1. My profile and its revision/media states (`/profile`).
2. Documents and review consoles (`/documents`, `/admin/reviews`,
   `/branch-review`).
3. Head of Family (`/head-of-family`).
4. Admin extraction, narrative review, source of truth, and member administration
   (`/admin/document-extraction`, `/admin/narrative-review`,
   `/admin/source-of-truth`, `/admin/family-members`).
5. Sign-in (`/signin`) and the remaining empty/denied states.

## Reproduce the screenshots

```sh
# with the Next.js app running on 127.0.0.1:3000
cd web
node scripts/capture-ui-reskin-stage2.mjs
```

The tool intercepts only the browser-facing session, tree, person-detail,
family-history, family-profile, and family-media endpoints. It needs no database,
Ollama, or credentials. Override `BASE_URL` or `OUT_DIR` when needed.
