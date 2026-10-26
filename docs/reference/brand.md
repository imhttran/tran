# Brand and interface foundation

**Type:** Descriptive reference (brand identity, design tokens, and interface rules)

This is the authoritative description of how the Trần Family Archive looks and
sounds. It records the **neutral jade** direction: a quiet, neutral archive with
plain surfaces, one restrained jade accent, a sans interface typeface, and calm
motion. The interface follows the reader's light or dark system preference.
Application behavior, authorization, and publication rules live in the
architecture contracts, not here.

## Identity

The archive is **private, evidence-backed, and calm**. It is a place to read and
verify family records, not a marketing dashboard. The interface should feel like a
well-kept reading room: neutral surfaces, dark green text, one jade accent, and
clear labels.

- **Product name:** Trần Family Archive (UI wordmark: `TRẦN FAMILY`).
- **Personality:** quiet, precise, humane, archival.
- **Not:** promotional, playful, neon, gamified, or an AI persona.

## Voice

- Plain, specific, and short. Say what is recorded, what is missing, and who can
  see it.
- Evidence and review status come before confidence. Never imply a relationship,
  living status, or approval that the archive cannot support.
- "No children recorded" means **no available child links**. It does not mean the
  person had no children. Keep that distinction in every surface.
- No exclamation marks, no urgency, no "unlock your family story" copy.

## Wordmark

- Set **TRẦN FAMILY** in the UI sans stack, uppercase, no letter-spacing, in
  `--fg` on the neutral sidebar.
- The line beneath reads **Family archive** in the UI sans, small and quiet, in
  `--muted`.
- The wordmark is the brand mark; there is no separate icon or logotype.
- The wordmark uses the same sans family as the interface. There is no serif
  interface type.

## Palette

All tokens are defined in `web/src/app/globals.css`. The `:root` block holds the
light values. A `@media (prefers-color-scheme: dark)` block overrides the same
token names with the dark values. `:root` also sets `color-scheme: light dark`.

### Colour

| Token                  | Light     | Dark      | Use                                        |
| ---------------------- | --------- | --------- | ------------------------------------------ |
| `--bg`                 | `#f5f6f6` | `#111514` | Page base                                  |
| `--card`               | `#ffffff` | `#181d1c` | Cards, panels, popups, inputs              |
| `--surface`            | `#eff1f1` | `#1f2523` | Quiet inset panels, legends, quotes        |
| `--surface-strong`     | `#e6e9e8` | `#262d2b` | Hover and disabled neutral                 |
| `--fg`                 | `#1c2422` | `#e7ebe9` | Body and heading text                      |
| `--muted`              | `#5a6663` | `#a3b0ab` | Secondary text and hints                   |
| `--line`               | `#dfe3e2` | `#2c3432` | Structural borders and dividers            |
| `--line-strong`        | `#c4cdca` | `#3c4643` | Input borders                              |
| `--accent`             | `#0f6b4a` | `#4ccf95` | Primary action, selected navigation, links |
| `--accent-strong`      | `#0b5639` | `#6fe0ac` | Hover and pressed accent                   |
| `--accent-soft`        | `#e3f1ea` | `#16352a` | Selected navigation fill, active chips     |
| `--accent-soft-strong` | `#cfe8dc` | `#1d4436` | Accent hover fill                          |
| `--accent-contrast`    | `#ffffff` | `#082018` | Text on an accent fill                     |
| `--ok`                 | `#1a7f37` | `#5ecc86` | Verified                                   |
| `--warn`               | `#8a5b00` | `#e0b355` | Needs review                               |
| `--err`                | `#b42318` | `#f28b82` | Errors and destructive actions             |
| `--warn-soft`          | `#fdf4df` | `#2c2413` | Warning surface                            |
| `--warn-line`          | `#ecd9a8` | `#4a3c1c` | Warning border                             |
| `--ok-soft`            | `#e6f4ea` | `#16291d` | Success surface                            |
| `--err-soft`           | `#fceceb` | `#331a19` | Error surface                              |
| `--info-soft`          | `#e8eef5` | `#1a2430` | Neutral information surface                |

Jade is the only accent. The primary action and the selected navigation state
both use `--accent`. Secondary controls use neutral `--surface` and
`--line-strong`, never the accent. **Red is reserved for destructive and error
states**, never for a primary call to action.

Status color is never the only signal: every badge, chip, and edge also carries
its status word.

### Radii

| Token           | Value   |
| --------------- | ------- |
| `--radius-sm`   | `8px`   |
| `--radius-md`   | `12px`  |
| `--radius-lg`   | `16px`  |
| `--radius-pill` | `999px` |

### Spacing

| Token       | Value  |
| ----------- | ------ |
| `--space-1` | `4px`  |
| `--space-2` | `8px`  |
| `--space-3` | `12px` |
| `--space-4` | `16px` |
| `--space-5` | `24px` |

### Shadows

| Token           | Light                                                                      | Dark                                                                 |
| --------------- | -------------------------------------------------------------------------- | -------------------------------------------------------------------- |
| `--shadow-sm`   | `0 1px 2px rgba(16, 40, 30, 0.06)`                                         | `0 1px 2px rgba(0, 0, 0, 0.4)`                                       |
| `--shadow-card` | `0 1px 2px rgba(16, 40, 30, 0.05), 0 8px 24px -20px rgba(16, 40, 30, 0.4)` | `0 1px 2px rgba(0, 0, 0, 0.4), 0 10px 26px -20px rgba(0, 0, 0, 0.8)` |

### Focus

| Token            | Light     | Dark      |
| ---------------- | --------- | --------- |
| `--focus-ring`   | `#0f6b4a` | `#6fe0ac` |
| `--focus-offset` | `2px`     | `2px`     |

## Typography

- **Interface type:** `--font-sans` =
  `"Segoe UI", system-ui, -apple-system, Roboto, "Helvetica Neue", Arial, "Noto Sans", "Segoe UI Symbol", sans-serif`.
  The stack uses installed system fonts only. It gives Vietnamese diacritics
  clean coverage with no font download.
- **Mono type:** `--font-mono` = `ui-monospace, SFMono-Regular, Menlo, monospace`,
  for code samples only.
- **Body text:** `16px`, `line-height: 1.55`, `letter-spacing: normal`. Body copy
  never carries tracking.
- **Headings:** the same sans stack, 700 weight, `letter-spacing: normal`. The
  page title is `clamp(1.6rem, 3.2vw, 2rem)`; card headings are about `1.05rem`.
  There is no serif interface heading.
- **Micro-labels:** uppercase, `0.72rem`, small tracking on labels only, in
  `--muted`.
- **Metadata:** `0.9rem` in `--muted`; keep line length near `70ch`.
- Use tabular numerals for counts and tables where alignment matters.

## Surfaces and depth

- Every surface uses a token: `--bg`, `--card`, `--surface`, or
  `--surface-strong`. No surface hard-codes a light colour.
- The shell, the site navigation, the page header, and the profile hero are plain
  neutral surfaces with a `1px` `--line` border. They carry no background image.
- Depth comes from a border and one `--shadow-card`. Do not draw heavy outlines.
- No page carries a decorative background. Do not add a gradient, a glow, or a dot
  grid to the body, header, navigation, or any panel.
- **Images:** a `1px` `--line` outline, never a tinted neutral.

## Iconography

- One line-icon set, inline SVG, `24` viewBox, `1.5px` stroke, `18px` rendered,
  `currentColor`, `aria-hidden` with a visible text label.
- Outline is the default. State comes from color and opacity, not a second asset.
- Do not use text glyphs (`☺`, `♧`, `?`) as icons.

## Motion

- High-frequency state (hover, focus, press): **150ms**,
  `cubic-bezier(0.2, 0, 0, 1)`, on named properties only.
- Person popup: **180ms** opacity + `translateY(6px)`.
- Animate opacity and transform only. No layout animation.
- `prefers-reduced-motion: reduce` removes transforms and transitions.
- Motion is never the only feedback: every state also changes color, an icon, or
  a label.

## Accessibility

- Meet WCAG AA contrast for text and controls in both themes.
- Focus ring: `3px solid var(--focus-ring)` with `2px` offset, visible on
  buttons, links, inputs, selects, and textareas. Inputs also add a soft
  `--accent-soft` ring.
- Interactive targets are at least `44px` in navigation and primary forms, and
  `38px` in dense toolbars and popups.
- A skip-to-content link is the first focusable control and targets the main
  content region.
- Status, visibility, and review state always have text labels.
- Keyboard paths: Tab, Enter/Space, Escape. Nothing depends on hover or
  double-click.

## Light and dark themes

- `:root` sets `color-scheme: light dark`.
- The light token set is the default. A `@media (prefers-color-scheme: dark)`
  block overrides the same token names.
- The browser selects the theme from the reader's system preference. There is no
  JavaScript theme provider and no in-app toggle.
- Both themes define the same token names. A component reads only the semantic
  token, never a raw colour.

## Where the brand lives

| Concern                                | Location                                                          |
| -------------------------------------- | ----------------------------------------------------------------- |
| Tokens, surfaces, type, motion, themes | `web/src/app/globals.css`                                         |
| Colour scheme declaration              | `web/src/app/layout.tsx` (root document)                          |
| Navigation icon set                    | `web/src/components/ExperienceShell.tsx` (`NavIcon`)              |
| Wordmark and skip link                 | `ExperienceShell.tsx` (`.site-nav .brand`, `.skip-link`)          |
| Person popup motion and placement      | `web/src/components/FamilyTree.tsx`, `web/src/lib/family-tree.ts` |
| Page header surface                    | `.page-header` in `globals.css`                                   |

Change the document and the tokens together. A visual change that alters the
identity (palette, type families, wordmark, motion language) is a brand change,
not a page tweak.

## Do and don't

| Do                                                                | Don't                                                |
| ----------------------------------------------------------------- | ---------------------------------------------------- |
| Keep neutral surfaces and one jade accent                         | Introduce a second competing accent                  |
| Use `--accent` for the primary action and the selected navigation | Give secondary controls the accent fill              |
| Use a sans interface stack with Vietnamese coverage               | Download a web font or use a serif interface heading |
| Label every status in words                                       | Rely on color alone                                  |
| Use red only for destructive/error                                | Use red as a primary CTA                             |
| Prefer 150ms, named-property transitions                          | Use `transition: all` or animate layout              |
| Read colours from theme tokens                                    | Hard-code a light colour on a surface                |
| Say "No children recorded" and explain its limits                 | Imply the person had no children                     |
