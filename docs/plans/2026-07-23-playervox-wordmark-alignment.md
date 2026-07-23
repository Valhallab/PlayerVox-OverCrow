# PlayerVox Wordmark Alignment

## Objective

Match the PlayerVox OverCrow wordmark to the established PlayerVox website
branding without changing the Control Center layout or product hierarchy.

## Design

- Render `Player` and `Vox` as separate text spans inside the existing brand
  component.
- Use the bundled Noto Sans Black Italic face already exposed as
  `OverCrow Display`.
- Keep `Player` white and render `Vox` with the canonical PlayerVox lime
  (`#a3e635`, exposed through `--lime`).
- Preserve uppercase rendering and tight tracking from the website wordmark.
- Keep `OverCrow` below the PlayerVox name in its current small, light, muted
  presentation.
- Preserve the existing full accessible label: `PlayerVox OverCrow`.

## Scope

Only the shared Control Center brand component and its focused presentation
test and styles are affected. The icon, global color system, overlay renderer,
packaging identity, and product strings remain unchanged.

## Verification

- A component test verifies the accessible full name and distinct Player/Vox
  wordmark segments.
- The frontend test suite and production build must pass.
- The final rendering is checked in the real Control Center.
