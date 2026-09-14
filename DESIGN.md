# Honk Node Control Design System

## 0. Research Log

- Embedded refs: shortlisted Sentry, PostHog, and ClickHouse from the data-dense dashboard index; picked `minimalist-skill` + Sentry because this is an operational console that needs quiet hierarchy, graphite surfaces, and strong probe states.
- Lazyweb: 3 desktop queries (`proxy node dashboard`, `network monitoring dashboard`, `node filter settings`), 3 screens viewed; took the narrow navigation, table-first hierarchy, compact filter drawer, and explicit empty/loading/error states.
- Visual reference: selected the first dark node-inventory concept as the reference-fidelity contract.
- UI-UX DB: `network operations dashboard dark dense` recommended a dark operations pattern with green positive indicators, Fira-like technical typography, and visible focus states; adapted those findings to the Sentry/minimalist palette without loading an external font.
- Skipped lanes: none.

## 1. Atmosphere & Identity

Honk Node Control is a quiet late-night command center: dense when the operator is comparing nodes, spacious when the system is empty or probing. The signature is a soft green probe pulse moving through a graphite inventory surface, with every status backed by a measured value or a clear “not tested” state.

## 2. Color

### Palette

| Role | Token | Value | Usage |
|---|---|---:|---|
| Surface / primary | `--surface-primary` | `#101214` | App canvas |
| Surface / deep | `--surface-deep` | `#0a0c0e` | Sidebar and shell edges |
| Surface / raised | `--surface-raised` | `#191c20` | Cards, forms, drawer |
| Surface / selected | `--surface-selected` | `#242830` | Selected rows and active nav |
| Text / primary | `--text-primary` | `#f3f4f6` | Titles and values |
| Text / on accent | `--text-on-accent` | `#ffffff` | Text on primary buttons and selected nodes |
| Text / secondary | `--text-secondary` | `#c4c9d2` | Body copy and table labels |
| Text / muted | `--text-muted` | `#8e96a3` | Hints and unmeasured values |
| Border / default | `--border-default` | `#303640` | Table and panel outlines |
| Border / subtle | `--border-subtle` | `#242a32` | Row separators |
| Accent / primary | `--accent-primary` | `#5968a8` | Buttons and active controls |
| Accent / highlight | `--accent-highlight` | `#8dbda0` | Healthy and measured |
| Focus ring | `--focus-ring` | `#aab2e3` | Keyboard focus and selected latency |
| Status / warning | `--status-warning` | `#d2a267` | Degraded and slow |
| Status / error | `--status-error` | `#d08095` | Failed probes and errors |

Rules: colors are semantic, not decorative. Accent green never appears without a healthy or measured meaning. No gradients, neon fields, or pure black backgrounds. Every raw color in CSS must be one of these tokens.

### Alternate light theme

The `.theme-light` shell swaps the graphite surfaces for a grayscale white/black treatment while preserving the same semantic accent and status meanings.

| Role | Token value in light theme |
|---|---:|
| Surface / primary | `#f6f6f4` |
| Surface / deep | `#ececea` |
| Surface / raised | `#ffffff` |
| Surface / selected | `#e9ecf3` |
| Text / primary | `#1c1f24` |
| Text / secondary | `#4e555f` |
| Text / muted | `#68717d` |
| Border / default | `#d9dde3` |
| Border / subtle | `#eaecf0` |
| Accent / primary | `#5f6ba3` |
| Accent / highlight | `#2f754f` |
| Focus ring | `#4e5b91` |
| Status / warning | `#a66a20` |
| Status / error | `#b64e68` |

## 3. Typography

| Level | Size | Weight | Line height | Usage |
|---|---:|---:|---:|---|
| Display | 32px | 600 | 1.1 | Page title |
| H2 | 22px | 600 | 1.2 | Section title |
| H3 | 16px | 600 | 1.35 | Card and drawer title |
| Body | 14px | 400 | 1.5 | Default UI copy |
| Body / small | 12px | 400 | 1.4 | Secondary metadata |
| Caption | 11px | 600 | 1.3 | Uppercase labels |
| Metric | 24px | 600 | 1.1 | Summary values |

Primary: `Avenir Next`, `SF Pro Display`, `system-ui`, sans-serif. Mono: `SF Mono`, `JetBrains Mono`, monospace. The display and body use one sans family; mono is reserved for addresses, protocols, and measured values.

## 4. Spacing & Layout

Base unit is 4px. Tokens: 4, 8, 12, 16, 20, 24, 32, 40, 48, 64px. The shell is a fixed-sidenav layout with a 224px sidebar and a fluid main region capped at 1440px. At 900px the sidebar becomes a top strip; at 560px controls stack; at 375px the table becomes stacked node cards.

Scroll ownership is explicit: the app shell is bounded by `100dvh`; the main region owns vertical scroll; the sidebar and top metric strip stay fixed within the shell. Filter and add-node drawers are overlays and do not create a second page scrollbar.

## 5. Components

### App shell

- Structure: fixed sidebar + `scroll-body-shell` main content.
- States: normal, loading, database error.
- Accessibility: landmark `nav` and `main`, visible focus ring, keyboard-reachable navigation.

### Metric strip

- Structure: three compact metric cards in an intrinsic grid.
- Variants: total nodes, healthy, median latency.
- Responsive: three columns on desktop, two columns below 900px.
- States: measured value, empty `—`, loading pulse.

### Node inventory

- Structure: toolbar + provider/region aggregate cards in two columns on desktop, one column on mobile; each group contains compact selectable node chips.
- Variants: subscription source, manual source, healthy, degraded, untested.
- States: hover, selected, focus, probing, empty, filtered-empty, error.
- Controls: inline search plus region, provider, protocol filters and a region/provider/latency/protocol sort selector.
- Group header: flag, provider/region label, protocol set, alive/total count, and median latency; node chips expose protocol, latency, and health state.
- Accessibility: filter controls use native inputs/selects; the health dot has an accessible status label and is never the only information exposed to assistive technology.

### Filter drawer

- Structure: overlay drawer with search, region, latency, stability, AI, and Netflix controls.
- States: closed, open, applied, validation error, clear.
- Motion: 200ms opacity/transform only; reduced motion removes movement.

### Action button

- Variants: primary slate-blue, quiet outline, destructive coral.
- States: default, hover, active, focus, disabled, loading, success/error copy swap.
- Motion: 150ms opacity/transform press response; async actions show an inline loading label.

### Status badge

- Variants: healthy, warning, error, untested, reachable.
- States: measured and unknown; always includes text for non-color identification.
- Node health variant: compact 8px semantic dot with a restrained opacity pulse; reduced motion disables the pulse.

## 6. Motion & Interaction

Micro interactions use 150ms ease-out. Drawer and filter transitions use 220ms ease-in-out. Probe state uses a single opacity pulse on the status marker; no decorative motion. `prefers-reduced-motion: reduce` disables transforms and keeps only an immediate opacity/state change. The interaction reference was the beui.dev `drawer`, `button`, `checkbox`, `table`, and `animated-badge` mechanisms, adapted to vanilla HTML controls.

## 7. Depth & Surface

Strategy: mixed, but restrained. Use 1px slate-tinted borders for tables and drawers, tonal shifts between `--surface-primary`, `--surface-deep`, and `--surface-raised`, and only the Sentry-inspired inset button shadow `inset 0 1px 3px rgba(0,0,0,.1)`. No large card shadows and no glass blur.

## 8. Accessibility Constraints & Accepted Debt

### Constraints

WCAG 2.2 AA target. Body contrast is at least 4.5:1, focus indicators are visible on every interactive control, status meaning is written as text, forms have labels and error copy, and the layout reflows to one readable column at 375px without horizontal overflow.

### Accepted Debt

| Item | Location | Why accepted | Exit |
|---|---|---|---|
| AI/Netflix unlock is an HTTP reachability heuristic | Probe service | The result indicates that the target responded through the selected node; it is not a legal or region-authoritative guarantee | Add provider-specific response classifiers and fixtures |
| No authentication layer in local panel | GraphQL and static shell | This MVP is intended for a trusted local control plane | Add secret/token middleware before network exposure |
