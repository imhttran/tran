# Brand and interface foundation

**Type:** Descriptive reference (brand identity, design tokens, and interface rules)

This is the authoritative description of how the Trần Family Archive looks and
sounds. It records the **light-premium** direction: a warm, quiet archive with
luminous green light, editorial serif branding, crisp sans interface type, and
calm motion. Application behavior, authorization, and publication rules live in
the architecture contracts, not here.

## Identity

The archive is **private, evidence-backed, and calm**. It is a place to read and
verify family records, not a marketing dashboard. The interface should feel like
a well-kept reading room: warm paper, deep evergreen, soft daylight, and clear
labels.

- **Product name:** Trần Family Archive (UI wordmark: `TRẦN FAMILY`).
- **Personality:** quiet, precise, humane, archival.
- **Not:** promotional, playful, neon, gamified, or an AI persona.

## Voice

- Plain, specific, and short. Say what is recorded, what is missing, and who can
  see it.
- Evidence and review status come before confidence. Never imply a relationship,
  living status, or approval that the archive cannot support.
- “No children recorded” means **no available child links**. It does not mean the
  person had no children. Keep that distinction in every surface.
- No exclamation marks, no urgency, no “unlock your family story” copy.

## Wordmark

- Set **TRẦN FAMILY** in Georgia, uppercase, `letter-spacing: 2px`, cream
  (`#f2e8c9`) on the evergreen sidebar.
- The line beneath reads **Family archive** in the UI sans, small and quiet.
- The wordmark is the brand mark; there is no separate icon or logotype.
- Keep the wordmark serif even though interface headings are sans. It is the
  editorial anchor.

## Palette

All tokens are defined in `web/src/app/globals.css` under
`Light-premium foundation`.

| Token | Value | Use |
| --- | --- | --- |
| `--bg` | `#f7f5ee` | Page base (warm paper) |
| `--card` | `#fffefb` | Cards, popups, reading surfaces |
| `--surface` | `#f1efe6` | Quiet inset panels, legends, notes |
| `--fg` | `#20362b` | Body text |
| `--muted` | `#5e6f65` | Secondary text and hints |
| `--line` | `rgba(24, 61, 48, 0.14)` | Structural borders and dividers |
| `--line-strong` | `rgba(24, 61, 48, 0.22)` | Input borders |
| `--accent` | `#183d30` | Headings, brand, primary text |
| `--accent-strong` | `#123026` | Pressed/deep states |
| `--accent-bright` | `#2f9e6a` | Focus accents, selected edges |
| `--accent-soft` | `#e7f3ea` | Active navigation, secondary buttons |
| `--accent-soft-strong` | `#d3ebdc` | Secondary hover |
| `--gold` | `#996d21` | Editorial eyebrow labels |
| `--ok` | `#1a7f37` | Verified |
| `--warn` | `#9a6700` | Needs review |
| `--err` | `#b42318` | Errors and destructive actions |

Luminous green is light, never neon: use `rgba(47, 158, 106, 0.10–0.14)` radial
glows and the `--accent-bright` for small accents. **Red is reserved for
destructive and error states**, never for a primary call to action.

Status color is never the only signal: every badge and edge also carries its
status word.

## Typography

- **Interface type:** `system-ui, -apple-system, "Segoe UI", Roboto, sans-serif`.
- **Headings:** 700 weight, tight tracking (`-0.02em` to `-0.025em`). The page
  title is `clamp(1.75rem, 3.4vw, 2.15rem)`; card headings are about `1.05rem`.
- **Editorial type:** Georgia, reserved for the wordmark and long-form reading.
- **Micro-labels:** uppercase, `0.72rem`, `0.12–0.14em` tracking, gold.
- **Metadata:** `0.9rem` muted; keep line length near `70ch`.
- Use tabular numerals for counts and tables where alignment matters.

## Surfaces and depth

- **Radii:** controls `10px`, inner cards `14px`, cards and panels `20px`, the
  person popup `18px`, pills `999px`.
- **Shadows:** layered transparent shadow
  `0 1px 2px rgba(16,40,30,.04), 0 16px 40px -28px rgba(16,40,30,.35)`.
- **Borders:** structure and state only, at `--line` / `--line-strong`. Do not
  draw heavy outlines to create depth.
- **One glow per page:** the page header carries the mint radial light and the
  faint dot grid. Cards do not repeat it. The body carries only a soft top glow.
- **Images:** a `1px` low-opacity outline, never a tinted neutral.

## Iconography

- One line-icon set, inline SVG, `24` viewBox, `1.5px` stroke, `18px` rendered,
  `currentColor`, `aria-hidden` with a visible text label.
- Outline is the default. State comes from color and opacity, not a second asset.
- Do not use text glyphs (`☺`, `♧`, `?`) as icons.

## Motion

- High-frequency state (hover, focus, press): **150ms**,
  `cubic-bezier(0.2, 0, 0, 1)`, on named properties only.
- Person popup: **180ms** opacity + `translateY(6px)`.
- Press feedback: `scale(0.96)`; nothing lower.
- Animate opacity and transform only. No layout animation.
- `prefers-reduced-motion: reduce` removes transforms and transitions.
- Motion is never the only feedback: every state also changes color, an icon, or
  a label.

## Accessibility

- Meet WCAG AA contrast for text and controls.
- Focus ring: `3px solid #1f7a50`, `2px` offset; inputs add a soft mint ring.
- Interactive targets are at least `40px` in popups and `38px` in dense toolbars.
- Status, visibility, and review state always have text labels.
- Keyboard paths: Tab, Enter/Space, Escape. Nothing depends on hover or
  double-click.

## Where the brand lives

| Concern | Location |
| --- | --- |
| Tokens, surfaces, type, motion | `web/src/app/globals.css` (`Light-premium foundation`) |
| Navigation icon set | `web/src/components/ExperienceShell.tsx` (`NavIcon`) |
| Wordmark | `ExperienceShell.tsx` (`.site-nav .brand`) |
| Person popup motion and placement | `web/src/components/FamilyTree.tsx`, `web/src/lib/family-tree.ts` |
| Page header surface | `.page-header` in `globals.css` |

Change the document and the tokens together. A visual change that alters the
identity (palette, type families, wordmark, motion language) is a brand change,
not a page tweak.

## Do and don't

| Do | Don't |
| --- | --- |
| Keep one warm paper base and one evergreen brand | Introduce a second competing accent |
| Use the mint glow once, behind the page title | Glow every card |
| Pair sans interface headings with the serif wordmark | Turn the wordmark into a sans label |
| Label every status in words | Rely on color alone |
| Use red only for destructive/error | Use red as a primary CTA |
| Prefer 150ms, named-property transitions | Use `transition: all` or animate layout |
| Say “No children recorded” and explain its limits | Imply the person had no children |
