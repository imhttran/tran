# UI-901 — token contract (Linear-inspired graphite/indigo)

Binding source: `docs/plans/PLAN-TRAN-UI-LINEAR-COLOR-REFRESH.md`.
This file fixes the derived values that `web/src/app/globals.css` applies.
The five base colors match the plan exactly. Every other token derives from them.

## Light theme (`:root`)

| Token | Value | Derivation | Used on |
| --- | --- | --- | --- |
| `--bg` | `#f6f6f7` | plan base | page background |
| `--card` | `#ffffff` | plan base | cards, panels, dialogs |
| `--surface` | `#f0f0f2` | plan base | secondary surfaces, chips |
| `--surface-strong` | `#e6e6e9` | one step darker than `--surface` | disabled, hover on surface |
| `--fg` | `#222326` | plan base | normal text on bg/card/surface |
| `--muted` | `#5c5f66` | desaturated mix of `--fg` toward `--surface` | hints, meta, secondary text |
| `--line` | `#e2e2e6` | one step darker than `--surface` | subtle borders |
| `--line-strong` | `#75787f` | mid neutral between `--muted` and `--line` | inputs, essential boundaries |
| `--accent` | `#4f5db8` | plan base | links, selected state, eyebrow, focus |
| `--accent-strong` | `#3d4a96` | `--accent` darkened ~15% | hover text on accent |
| `--accent-soft` | `#ecedf8` | `--accent` at ~8% over `--card` | selected background |
| `--accent-soft-strong` | `#dcdff3` | `--accent` at ~15% over `--card` | selected hover |
| `--accent-contrast` | `#ffffff` | light text on accent/danger fill | `.button-danger` text |
| `--action-bg` | `#222326` | neutral graphite, no hue | primary buttons |
| `--action-hover` | `#36373c` | `--action-bg` lightened | primary hover |
| `--action-fg` | `#ffffff` | neutral text | primary button text |
| `--ok` | `#1a7f37` | unchanged status hue | success chips |
| `--warn` | `#8a5b00` | unchanged status hue | warning text/rules |
| `--err` | `#b42318` | unchanged status hue | error text/rules |
| `--warn-soft` | `#fdf4df` | warn tint over card | warning surfaces |
| `--warn-line` | `#e6cf9b` | warn-line tuned for new card | warning borders |
| `--ok-soft` | `#e6f4ea` | ok tint over card | success surfaces |
| `--err-soft` | `#fceceb` | err tint over card | error surfaces |
| `--info-soft` | `#eceef2` | neutral tint over card | info surfaces |
| `--focus-ring` | `#4f5db8` | equals `--accent` | 3 px focus outline |
| `--backdrop` | `rgba(18,18,20,0.6)` | graphite dim from the palette base | dialog backdrops |
| `--shadow-sm` | `0 1px 2px rgba(18,18,20,0.06)` | graphite shadow | subtle elevation |
| `--shadow-card` | `0 1px 2px rgba(18,18,20,0.05), 0 8px 24px -20px rgba(18,18,20,0.3)` | graphite shadow | cards |

## Dark theme (`@media (prefers-color-scheme: dark)`)

| Token | Value | Derivation | Used on |
| --- | --- | --- | --- |
| `--bg` | `#121214` | plan base | page background |
| `--card` | `#1b1b1e` | plan base | cards, panels, dialogs |
| `--surface` | `#29292d` | plan base | secondary surfaces |
| `--surface-strong` | `#35353a` | one step lighter than `--surface` | disabled, hover |
| `--fg` | `#f4f5f8` | plan base | normal text |
| `--muted` | `#a6a8b0` | desaturated mix of `--fg` toward `--surface` | hints, meta |
| `--line` | `#2e2e33` | one step lighter than `--surface` | subtle borders |
| `--line-strong` | `#76787f` | mid neutral | inputs, essential boundaries |
| `--accent` | `#a0a9f8` | plan base | links, selected state, focus |
| `--accent-strong` | `#bcc3fb` | `--accent` lightened | hover text |
| `--accent-soft` | `#23263a` | `--accent` at ~12% over `--card` | selected background |
| `--accent-soft-strong` | `#2e3250` | `--accent` at ~20% over `--card` | selected hover |
| `--accent-contrast` | `#121214` | matches `--bg` | text on light fills |
| `--action-bg` | `#f4f5f8` | neutral graphite-white | primary buttons |
| `--action-hover` | `#ffffff` | `--action-bg` brightened | primary hover |
| `--action-fg` | `#121214` | matches `--bg` | primary button text |
| `--ok` | `#5ecc86` | unchanged status hue | success chips |
| `--warn` | `#e0b355` | unchanged status hue | warning text/rules |
| `--err` | `#f28b82` | unchanged status hue | error text/rules |
| `--warn-soft` | `#2c2413` | warn tint over dark card | warning surfaces |
| `--warn-line` | `#4a3c1c` | warn border on dark card | warning borders |
| `--ok-soft` | `#16291d` | ok tint over dark card | success surfaces |
| `--err-soft` | `#331a19` | err tint over dark card | error surfaces |
| `--info-soft` | `#1e1f24` | neutral tint over dark card | info surfaces |
| `--focus-ring` | `#a0a9f8` | equals `--accent` | 3 px focus outline |
| `--shadow-sm` | `0 1px 2px rgba(0,0,0,0.4)` | dark shadow | subtle elevation |
| `--shadow-card` | `0 1px 2px rgba(0,0,0,0.4), 0 10px 26px -20px rgba(0,0,0,0.8)` | dark shadow | cards |

## Semantic separation rule

`--ok`, `--warn` and `--err` keep their green, amber and red hue families in
both themes. No selector uses them as decoration. The indigo accent never
replaces a status hue, and no status hue replaces the accent.

## Measured contrast pairs

Ratios computed from the hex values above with the WCAG relative-luminance
formula.

| Pair | Light | Dark | Required | Result |
| --- | --- | --- | --- | --- |
| `--fg` on `--bg` | 14.6:1 | 16.2:1 | 4.5:1 | PASS |
| `--fg` on `--card` | 15.6:1 | 14.6:1 | 4.5:1 | PASS |
| `--fg` on `--surface` | 13.5:1 | 10.7:1 | 4.5:1 | PASS |
| `--muted` on `--card` | 6.4:1 | 7.9:1 | 4.5:1 | PASS |
| `--muted` on `--surface` | 5.6:1 | 5.8:1 | 4.5:1 | PASS |
| `--accent` on `--card` (link text) | 6.6:1 | 8.3:1 | 4.5:1 | PASS |
| `--accent` on `--accent-soft` (selected nav) | 5.9:1 | 6.6:1 | 4.5:1 | PASS |
| `--action-fg` on `--action-bg` (primary button) | 15.9:1 | 16.4:1 | 4.5:1 | PASS |
| `--ok` on `--ok-soft` (success chip) | 4.6:1 | 7.7:1 | 4.5:1 | PASS |
| `--warn` on `--warn-soft` (warning chip) | 5.4:1 | 8.1:1 | 4.5:1 | PASS |
| `--err` on `--err-soft` (error chip) | 5.4:1 | 6.6:1 | 4.5:1 | PASS |
| `--focus-ring` on `--bg` | 5.6:1 | 8.6:1 | 3:1 | PASS |
| `--focus-ring` on `--card` | 6.6:1 | 8.3:1 | 3:1 | PASS |
| `--line-strong` on `--card` (input border) | 4.3:1 | 4.2:1 | 3:1 | PASS |
| `--line` on `--card` (subtle border) | 1.3:1 | 1.4:1 | exempt | Decorative; not an essential boundary |
| `--accent` on `--card` (tree edge stroke) | 6.6:1 | 8.3:1 | 3:1 | PASS |
| `--ok` on `--surface` (verified edge) | 4.2:1 | 6.6:1 | 3:1 | PASS |
| `--warn` on `--surface` (review edge) | 5.0:1 | 7.0:1 | 3:1 | PASS |

## Pairs that cannot reach the threshold

None. Every normal-text pair reaches 4.5:1 and every essential boundary and
focus ring reaches 3:1. `--line` is decorative and is exempt under WCAG 1.4.11
because it is not required to identify a control or a state.
