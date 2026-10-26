# Implementation Plan

## Project

Tran family archive — stage-one UI reskin

## Summary

Reskin the shared application shell and the existing archive question-and-answer
screen (`/ask`, labelled **Ask the archive**) with a modern, neutral interface
inspired by Vercel's visual simplicity and Raycast's polished search experience.

The change keeps the current Next.js App Router stack, routes, APIs,
authentication, permissions, retrieval rules, and document approval behaviour.
It adds a shared light/dark design-token foundation, a responsive application
shell, reusable control and surface styles, and a focused redesign of the Ask
screen and its states. Other routes inherit the shared tokens and component
styles; their detailed page-by-page redesign is out of scope for this stage.

### What must not change

- The Ask route stays `/ask` and the visible label stays **Ask the archive**.
- Existing navigation labels, order, role gating, and test identifiers stay
  intact: `site-nav`, `nav-toggle`, `account-menu`, `account-role`,
  `sign-in-button`, `sign-out-button`, `question-input`, `ask-button`,
  `ask-example`, `ask-suggestions-loading`, `ask-suggestions-empty`,
  `ask-suggestions-error`, `ask-loading`, `ask-expired`, `ask-denied`,
  `ask-error`, `insufficient`, `answer`, `sources`, and every `person-*`
  identifier used by the person-answer tests.
- The viewer navigation remains, in order: My profile, Family tree, Ask the
  archive, Head of Family, Branch review, Documents. Admin and upload items keep
  their existing permission gates and their existing position after the shared
  items.
- Suggestions remain derived from approved archive content. There is no
  hard-coded suggestion list.
- A recoverable answer failure never clears the question the member typed.
- Person answers remain visually and semantically distinct from document
  archive answers.
- No command palette, new backend feature, new API route, new dependency,
  framework migration, or browser-supplied authorization claim is introduced.

### Execution boundaries

SOP's command policy may treat build, test, migration, and browser commands as
approval-gated. The implementing agent therefore writes the code and the
documentation only. It does not attempt to run `npm`, `cargo`, `psql`,
Playwright, or a browser. The SOP validation stage runs the configured Rust
gates. The operator runs the focused web checks and captures the screenshots
after the edit.

Only the files named by each task may change. Do not reformat or rewrite
unrelated code, do not touch Rust sources, routes, APIs, migrations, plans, or
the local `.agent-sdlc/` state, and preserve pre-existing user changes such as
`.zed/settings.json`, `generate_audio.py`, and `.playwright-mcp/`.

All screenshots and examples use synthetic names, synthetic sources, and
synthetic citations. No real family data, private media, tokens, or credentials
may enter a file, screenshot, test, or report.

## Design specification

The implementation must express this specification through tokens and shared
classes in `web/src/app/globals.css`; page components use class names, not
inline styles.

### Direction

- Neutral, quiet, and precise. Think a well-built desktop utility, not a
  marketing site.
- One restrained jade accent for primary actions, selected navigation, focus
  rings, and small emphasis. Do not introduce a second competing accent.
- Restrained depth: a 1px border, a near-flat surface, and at most one short
  shadow. No decorative gradients, glow blobs, dot grids, oversized hero type,
  or ornamental animation.
- Information density is comfortable for a mixed-age family audience: visible
  labels, 44px minimum targets in navigation and primary forms, readable line
  lengths, and no reliance on hover.

### Tokens

Define a semantic token set on `:root` and override it under
`@media (prefers-color-scheme: dark)`. Set `color-scheme: light dark` so native
controls follow the same preference. System preference is the default; a manual
theme switch is not required in this stage.

Light values are a guide, not a demand for exact hex values. Keep the
relationships and ensure WCAG AA contrast.

| Token | Light intent | Dark intent |
| --- | --- | --- |
| `--bg` | near-white neutral canvas | near-black neutral canvas |
| `--surface` | white panel | dark raised panel |
| `--surface-muted` | quiet neutral inset | dark quiet inset |
| `--fg` | near-black text | near-white text |
| `--fg-muted` | neutral secondary text | neutral secondary text |
| `--border` | subtle neutral border | subtle dark border |
| `--border-strong` | input/control border | input/control border |
| `--accent` | deep jade | bright jade |
| `--accent-hover` | deeper jade | brighter jade |
| `--accent-fg` | white or near-black, whichever meets contrast | same rule |
| `--accent-soft` | very light jade wash | translucent jade wash |
| `--focus-ring` | visible jade ring | visible jade ring |
| `--danger`, `--danger-soft` | restrained red | restrained red |
| `--warning`, `--warning-soft` | restrained amber | restrained amber |
| `--success`, `--success-soft` | restrained green | restrained green |
| `--shadow-sm` | 1px ambient shadow | 1px ambient shadow |

Add spacing (`--space-1` … `--space-8` or an equivalent clear scale), radii
(`--radius-sm` for controls, `--radius-md` for cards, `--radius-lg` for large
panels, `--radius-pill` for pills), and typography tokens. Use a sans-serif
system stack that renders Vietnamese diacritics correctly, for example
`system-ui, -apple-system, "Segoe UI", Roboto, "Noto Sans", sans-serif`.
Use one sans family for interface text and headings. A monospace token may be
kept for code and citations.

### Shared component styling

- **Buttons:** a primary button uses the jade accent; secondary and ghost
  buttons use a neutral surface and border. Minimum 44px height for primary
  actions and navigation controls; 38px is acceptable in dense toolbars. Hover,
  active, disabled, and focus states are distinct. Never use red as a primary
  action.
- **Inputs:** 1px `--border-strong`, neutral surface, clear focus ring, and a
  visible label or accessible name. Textareas and selects follow the same rules.
- **Panels:** 1px border, card surface, modest radius, and one restrained
  shadow. Do not nest heavy cards inside heavy cards.
- **Alerts and status notices:** distinct info, success, warning, and error
  surfaces with a text label and icon or other non-colour signal. Errors are
  never conveyed by colour alone.
- **Badges and status chips:** pill shape, readable text, and a semantic
  surface. Keep the existing status words.
- **Focus:** every interactive element exposes a visible focus ring with a
  clear offset. Do not remove the outline without a replacement.
- **Motion:** 120–150ms on named properties only; honour
  `prefers-reduced-motion: reduce`. No layout animation.

### Legacy compatibility

The file currently contains component-specific rules for the family tree,
profile, review consoles, documents, narrative review, and other screens.
Preserve their selectors and layout behaviour. Replace hard-coded light
backgrounds, borders, and text colours with the semantic tokens wherever they
would otherwise become unreadable in dark mode. It is acceptable for a legacy
page to keep its current layout; it is not acceptable for a legacy page to
render light text on a hard-coded light surface or dark text on a hard-coded
dark surface.

## UI-101 — Neutral design tokens, light/dark themes, and shared styles

Replace the warm, gradient-heavy “light-premium” foundation in
`web/src/app/globals.css` with the neutral jade design specification above.

Files you may change:

- `web/src/app/globals.css`
- `web/src/app/layout.tsx`
- `docs/reference/brand.md`

Requirements:

- Define the complete semantic token set for light and dark themes, including
  colour, type, spacing, radii, borders, shadows, and focus.
- Set `color-scheme: light dark` and honour `prefers-color-scheme: dark`.
- Remove the decorative page-header radial gradient and dot grid, the body glow
  gradients, and the gradient button fills. Replace them with neutral surfaces
  and a restrained jade accent.
- Restyle the shared primitives used across the application: page headers,
  cards and panels, buttons, inputs, selects, textareas, form statuses,
  insufficient-evidence notices, badges, status chips, source quotes, and page
  footers.
- Keep every existing selector that a page or component depends on, and keep
  responsive layouts working. Convert hard-coded light colours to tokens where
  needed for dark-mode readability.
- Make Vietnamese text render cleanly: use a sans stack with Vietnamese
  coverage, normal line-height for body text, and no letter-spacing on body
  copy.
- Keep `layout.tsx` metadata and structure; add only what the theme and
  language rendering need.
- Rewrite `docs/reference/brand.md` to describe the new neutral/jade direction,
  the token names and values, typography, surfaces, motion, accessibility, and
  the light/dark theme policy. It must stop describing gradients, glows, dot
  grids, and serif interface headings as current requirements.

Do not add a dependency, font download, CSS framework, or JavaScript theme
provider.

### Dependencies

None

### Deliverables

- The rewritten token and shared-style foundation in `web/src/app/globals.css`.
- Any minimal `web/src/app/layout.tsx` change needed for the colour scheme.
- A rewritten `docs/reference/brand.md` consistent with the implementation.

### Acceptance Criteria

- Light and dark token sets exist, are applied through semantic variables, and
  follow the system preference without JavaScript.
- No decorative gradient, glow, or dot-grid treatment remains on the shared
  shell, page header, or Ask screen.
- The primary action and selected navigation state use the same restrained jade
  accent, and secondary controls are neutral.
- Every existing component selector still has a styled equivalent; no legacy
  screen loses its layout because a selector was deleted.
- Hard-coded light surfaces that would pair with light dark-mode text have been
  converted to tokens or given a dark-mode equivalent.
- Focus rings are visible on buttons, links, inputs, selects, and textareas.
- Body text uses a Vietnamese-capable sans stack at a readable size and line
  height.
- `docs/reference/brand.md` matches the delivered tokens and direction.

## UI-102 — Responsive application shell, navigation, and page header

Redesign the shared shell around the new foundation while preserving its
behaviour and test contract.

Files you may change:

- `web/src/components/ExperienceShell.tsx`
- `web/src/components/AccountMenu.tsx`
- `web/src/components/PageHeader.tsx`
- `web/src/components/PageFooter.tsx`
- `web/src/app/globals.css`

Requirements:

- Desktop: a quiet, neutral sidebar (about 240–260px) with a right border, a
  compact wordmark block, clearly labelled navigation items with the existing
  line icons, an unmistakable jade selected state, and comfortable targets.
  The sidebar may be sticky and should not scroll the page horizontally.
- Mobile: a compact top bar with the wordmark and a Menu button, a collapsible
  navigation panel, and the account controls. The menu must remain keyboard
  operable, close after navigating, and expose `aria-expanded` on the toggle.
  Keep the navigation hidden when closed so the existing mobile test passes.
- Keep the account area easy to find without letting it compete with the page
  title. It must still show the server-provided display label and role badge,
  and must still expose the existing sign-in/sign-out states.
- Make the page header compact and information-led: title, optional subtitle,
  and optional actions. Remove the oversized marketing treatment and
  decorative background. Preserve the `PageHeader` component API used by every
  page.
- Keep `PageFooter` simple and muted.
- Do not change which routes appear for a role, the order of the shared items,
  the labels, or the permission checks. Do not turn navigation into icon-only
  controls.
- Add a skip-to-content link that targets the main content region, and give
  the main region a stable id. Keep the skip link visually hidden until it is
  focused.
- Ensure no unintended horizontal overflow at 390px and 320px widths.

### Dependencies

- UI-101

### Deliverables

- The redesigned shell, account menu, page header, and footer components.
- The shell-specific styles in `web/src/app/globals.css`.

### Acceptance Criteria

- Viewer navigation contains exactly the six existing links, in the existing
  order, with visible labels and a correct `aria-current` state.
- Admin and upload items remain gated exactly as before.
- The mobile menu opens, navigates, and closes, and the navigation remains
  hidden when closed.
- The account menu still exposes the existing test identifiers and states.
- The page header is compact at desktop and mobile widths and keeps its
  title/subtitle/actions contract.
- A keyboard user can reach and see focus on the skip link, navigation, menu
  toggle, and account controls.
- There is no horizontal overflow at 390px or 320px.

## UI-103 — Ask the archive screen, states, answer, and citations

Redesign `/ask` as the focused centrepiece of the reskin. Keep all behaviour,
data flow, permissions, and test identifiers.

Files you may change:

- `web/src/app/ask/page.tsx`
- `web/src/components/PersonAnswer.tsx`
- `web/src/components/FormStatus.tsx`
- `web/src/components/StatusNotice.tsx` (new, optional)
- `web/src/app/globals.css`
- `web/e2e/experience.spec.ts` (add one regression test only)

Requirements:

- Give the question field clear visual priority in a dedicated panel: a large
  input, a persistent accessible label, the existing question icon, and a
  prominent Ask button. On mobile, the input and button stack without
  overflowing; the button remains a comfortable target.
- Add a short helper line that makes the two answer kinds distinguishable:
  questions about documents/events produce cited archive answers, while a
  person name produces a person record from stored assertions. Do not add a
  mode switch or change the API request shape.
- Preserve the content-derived suggestions and their behaviour: selecting a
  suggestion fills the input, keeps focus in the input, and does not submit.
  Style them as clear, readable options rather than decorative chips.
- Give each state a distinct, useful presentation while preserving the
  existing test identifiers and user-facing text:
  - suggestions loading;
  - suggestions empty (no approved searchable content);
  - suggestions unavailable (the form still works);
  - answer loading;
  - insufficient evidence / no result;
  - recoverable error;
  - session expired;
  - permission denied.
  Use `role="status"` for progress and non-urgent states, and `role="alert"`
  for errors that need attention. Do not rely on colour alone.
- Keep the entered question intact when a recoverable request fails. Add one
  Playwright test in `web/e2e/experience.spec.ts` that asks with a 503 response
  and asserts both the error notice and the retained input value.
- Present archive answers in a readable measure (roughly 60–72ch) with a clear
  heading, comfortable line height, and a clear distinction between the answer
  and its sources.
- Render sources as numbered citations with source, section, page metadata, and
  the existing quote text. Make the quote a readable preview (for example a
  disclosure or an inset quote block). Do not invent links, URLs, or previews
  the API does not return.
- Keep person answers clearly separate from archive answers: a distinct
  “Person record” label or eyebrow, the existing assertion review badges, the
  existing evidence links/dialog, and the existing profile link. Preserve every
  `person-*` test identifier and the existing candidate-selection behaviour.
- Keep the existing note that answers never change family records or appoint a
  Head of Family.
- Do not add a command palette, streaming, a new query parameter, or any
  backend change.

### Dependencies

- UI-102

### Deliverables

- The redesigned Ask page and its person-answer presentation.
- Any small shared status-notice component the page uses.
- The Ask-specific styles in `web/src/app/globals.css`.
- One regression test for retaining the question after a 503.

### Acceptance Criteria

- The question input, Ask button, suggestion buttons, answer, sources, and all
  existing state test identifiers remain available with their existing
  behaviour.
- Selecting a suggestion fills the input and focuses it; it does not submit.
- A 503 leaves the error notice visible and the question still in the input.
- Insufficient-evidence, suggestions-empty, suggestions-error, loading,
  expired, and denied states are visually distinct and readable in both themes.
- An answer is limited to a readable line length, and each citation shows its
  numbered source, section, page metadata, and quoted preview.
- Person answers are visibly distinct from archive answers and keep the
  existing evidence and profile links.
- No route, API, authentication, permission, retrieval, or approval behaviour
  changes.
- The screen contains no decorative gradient, oversized marketing header, or
  unnecessary animation.

## UI-104 — Reskin evidence, screenshot tooling, and PR notes

Add a reproducible, synthetic-data screenshot tool and a stage report so the
operator can attach before/after evidence and record verification.

Files you may change:

- `web/scripts/capture-ui-reskin.mjs` (new)
- `package.json` only if a local script alias is genuinely useful
- `docs/reports/ui-reskin/stage-1/README.md` (new)
- `docs/reports/ui-reskin/stage-1/after/` (screenshots produced by the
  operator)
- `docs/reports/ui-reskin/stage-1/before/` (baseline screenshots already
  captured by the operator)

Requirements:

- The capture tool uses the installed Playwright dependency. It does not add a
  package. It targets the running Next.js app (default
  `http://127.0.0.1:3000`) and intercepts only the browser-facing synthetic
  endpoints: `/api/auth/session`, `/api/query/suggestions`, and `/api/query`.
- Capture at least: desktop light idle and answered, desktop dark answered,
  mobile light answered, mobile dark answered, plus loading, empty, and error
  states where practical. Use a 1440×1024 desktop viewport and a 390×844 mobile
  viewport.
- Use only synthetic labels, names, documents, and citations. Never capture a
  real family record, private media, a token, or a credential.
- Write a report that explains the direction, lists the changed surfaces,
  embeds the before/after images with relative links, records the exact
  verification commands and results, lists known limitations, and names the
  remaining screens for the next reskin stage.
- The report must not claim a command passed unless its result was recorded.
  Use `NOT RUN` for anything not executed.

### Dependencies

- UI-103

### Deliverables

- `web/scripts/capture-ui-reskin.mjs`.
- `docs/reports/ui-reskin/stage-1/README.md`.
- The after screenshots produced when the operator runs the script.

### Acceptance Criteria

- The capture tool is self-contained, requires no new dependency, and works
  against a running local app without a database or Ollama.
- It produces the required desktop/mobile and light/dark screenshots from
  synthetic intercepted data.
- The report embeds the before/after evidence and records verification results
  and known limitations honestly.
- No real family data or secret appears in any screenshot or report.
