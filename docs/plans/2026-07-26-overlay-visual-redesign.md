# Overlay Visual Redesign

## Goal

Give the interactive overlay and every built-in widget a coherent PlayerVox
identity while preserving provider activity, transactional persistence,
input safety, passive click-through, and supported display integrations.

## Direction

Use a lightweight “tactical dashboard” language built entirely with egui
primitives:

- zinc-black surfaces and elevated cards;
- PlayerVox lime for active state, focus, and small structural accents;
- high-contrast white content and restrained zinc metadata;
- compact uppercase eyebrow labels and clear numeric hierarchy;
- simple geometric marks drawn by egui instead of image assets or icon fonts;
- one subtle border and shadow layer, with no blur, shader, animation loop, or
  texture dependency.

This intentionally mirrors the Control Center and PlayerVox website while
remaining readable over varied game imagery.

## Widget library

Replace the long settings list with a centered library surface:

- **General**: Session, Clock, Performance, Stopwatch, Media, Notes, Twitch.
- **Warframe**: Status, Fissures, Market, Sortie & Archon, Invasions.
- Each widget is a compact card with a geometric mark, name, short purpose, and
  clear enabled state.
- Clicking anywhere on a card toggles that widget. Cards contain no secondary
  controls, which keeps the catalog dense and usable as the recovery path for a
  disabled widget.
- Categories and card metadata come from the widget registry so future games
  add a category without rewriting the library.

All fine-grained settings belong to the widget itself. The catalog does not
duplicate passive visibility, transparency, scale, position, or provider
options.

## Widget chrome

All widgets share:

- a layered translucent panel frame;
- a lime leading accent and consistent header hierarchy;
- branded controls, inputs, tabs, checkboxes, status pills, separators, and
  resize affordance;
- content-specific cards and rows where they improve scanning.

In interactive mode, hovering a widget exposes one fixed-size foreground
toolbar with three custom-drawn actions:

1. an eye, or a struck eye, toggles passive visibility;
2. an ellipsis opens the widget options;
3. a cross disables the widget immediately.

The toolbar floats above the top-right edge without participating in content
measurement. It is clamped or folded inside the panel near the top viewport
edge, remains active while the pointer moves between the panel and toolbar, and
is never rendered in passive mode. Its controls have accessible labels and do
not depend on icon fonts.

The options popup always contains transparent background, a content-scale
slider from 75% to 175%, reset scale, reset position, and the widget-specific
options. Widget-specific painters remain close to their widget modules and emit
typed actions. The application routes actions through the existing validated,
transactional general, Twitch, or Warframe settings authority.

Compact widgets retain their natural content size. Resizable widgets retain
their configured interactive size and content-driven passive height.
Transparent-background mode removes the panel surface while retaining readable
content and necessary controls.

## General widget refinement

This feedback pass refines common-widget presentation and returns fine-grained
settings to a shared floating widget surface. Warframe content remains outside
the visual redesign, but uses the same toolbar and local options contract.

- **Session** renders only the elapsed session value.
- **Clock** renders time and, by default, date. A persisted `Show date` option
  can remove the date.
- **Performance** renders four consistently aligned metrics in either one
  horizontal row or one vertical stack. Viewports too narrow for four readable
  columns use a bounded 2x2 fallback. Horizontal mode has a persisted,
  width-only resize affordance from 300 to 900 logical pixels; available width
  is distributed across KPI cells. The selected orientation is persisted. Game
  CPU is displayed as a normalized share of the logical CPU capacity, clamped
  to 0–100%, while the existing raw multi-core sample remains unchanged in the
  protocol.
- **Manual stopwatch** renders elapsed time and interactive controls without a
  redundant icon, title, or running/paused label.
- **Media** renders track information without a widget header or text status.
  Interactive controls use conventional previous, play/pause, and next
  symbols. The passive metadata line separates the artist from the current
  play/pause state symbol. Its content-driven width follows the active title
  between 240 and 560 logical pixels and wraps only at the maximum.
- **Notes** renders the note workspace without a widget icon or title. The add
  control participates in the horizontally scrolling tab row directly after
  the final note. Checklist removal uses a dedicated 22-pixel inline action
  rather than a full-size control.
- **Twitch** moves favorite mutation into its options. A non-interactive star
  beside the channel name indicates that the active channel is a favorite.

The clock and performance settings are additive and defaulted during
deserialization so existing profiles remain valid. No stable widget ID,
position, size, or provider preference changes.

## Shared controls

Shared chrome provides the visual and interaction contract for recurring
elements:

- badges use the compact height and padding of the catalog's `CUSTOMIZE MODE`
  indicator;
- single-line inputs, ordinary buttons, primary buttons, tab buttons, and icon
  buttons use one control height;
- media and other icon-only actions use a shared bounded square button with an
  accessible hover label;
- destructive inline row actions use a separate 22-pixel primitive;
- metric rows/cells allocate explicit label and value regions so differing text
  widths cannot shift alignment.

These helpers remain presentation-only. They do not own widget state or dispatch
commands.

## Geometry and scale

The existing persisted per-widget `scale` value becomes user-facing and applies
to every widget. It scales content typography and spacing but never the floating
toolbar. Resizable panels keep their configured outer size while compact panels
re-measure naturally within their explicit bounds.

The last visible absolute top-left is the runtime anchor for content changes and
interactive/passive transitions. Auto-height, media-title, scale, and mode
changes therefore grow or shrink from that corner. The anchor is clamped only
when the new bounds would leave the safe viewport. Transient anchoring is kept
inside the manager and does not rewrite persisted normalized positions; a real
user drag, resize, or reset remains the only geometry mutation.

## Behavior invariants

The redesign must preserve:

- stable widget IDs, profile schema, positions, sizes, and settings actions;
- interactive-only dragging, resizing, input, and settings mutation;
- passive click-through and read-only behavior;
- content-driven passive height and viewport bounds;
- Twitch, media, notes, stopwatch, and Warframe provider behavior;
- the existing bottom control bar behavior and close paths;
- X11, KWin Wayland, and Hyprland placement/input contracts.

Widget-surface controls must additionally preserve:

- interactive-only mutation and the existing reliable close path;
- the Notes rule that at least one of note or checklist remains visible;
- Twitch authentication, favorite, channel, and passive-lifetime validation;
- Warframe preference gating and its dedicated durable store;
- transactional rollback for eye, disable, transparency, scale, layout, and
  position actions, with one bounded non-modal settings notice;
- failure logging without user content, provider payloads, account names, or
  other sensitive data.

## Performance

The UI remains immediate-mode and event-driven. Paint helpers perform bounded
work over already bounded widget data. There are no new background tasks,
repaint timers, network requests, allocations proportional to unbounded input,
or external assets.

## Validation

Automated tests cover category membership/order, card toggles, toolbar
visibility and routing, settings rollback, settings backward compatibility,
top-left anchoring, bounded dynamic media width, scale propagation, performance
resize/distribution, CPU normalization and bounds, control size contracts,
passive sizing, and existing widget behaviors. Final checks are format, Clippy,
the full workspace test suite, and diff hygiene. Live visual, click-through,
drag, resize, scaling, and compositor acceptance remains a short real-machine
test for the user.
