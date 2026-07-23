# PlayerVox Wordmark Alignment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render the Control Center PlayerVox wordmark exactly like the website:
white `Player`, lime `Vox`, Noto Sans Black Italic, with the existing muted
`OverCrow` subtitle.

**Architecture:** Keep the existing shared `Brand` component and bundled font.
Split the stable brand name into semantic spans and apply one focused accent
class. No asset, layout, or product-identity changes are required.

**Tech Stack:** React, TypeScript, CSS, Vitest, Testing Library

## Global Constraints

- Use the existing bundled `OverCrow Display` Noto Sans Black Italic face.
- Use `--lime` (`#a3e635`) for `Vox`.
- Preserve the accessible name `PlayerVox OverCrow`.
- Keep the existing icon and `OverCrow` subtitle presentation unchanged.

---

### Task 1: Align the shared PlayerVox wordmark

**Files:**
- Modify: `crates/overcrow-control-ui/src/App.test.tsx`
- Modify: `crates/overcrow-control-ui/src/components/Brand.tsx`
- Modify: `crates/overcrow-control-ui/src/i18n/en.ts`
- Modify: `crates/overcrow-control-ui/src/styles.css`

**Interfaces:**
- Consumes: `en.brand.fullName`, `en.brand.product`, and the existing
  `OverCrow Display` font face.
- Produces: `en.brand.player`, `en.brand.vox`, and the CSS class
  `brand__accent`.

- [ ] **Step 1: Write the failing component assertion**

Replace the current combined-name assertion in `App.test.tsx` with:

```tsx
const player = within(brand).getByText('Player');
const vox = within(brand).getByText('Vox');
expect(player).toBeVisible();
expect(vox).toBeVisible();
expect(vox).toHaveClass('brand__accent');
expect(within(brand).getByText('OverCrow')).toBeVisible();
expect(brand).toHaveAccessibleName('PlayerVox OverCrow');
```

- [ ] **Step 2: Run the focused test and verify failure**

Run:

```sh
npm --prefix crates/overcrow-control-ui test -- --run
```

Expected: FAIL because `PlayerVox` is still one text node and
`brand__accent` does not exist.

- [ ] **Step 3: Implement the minimal wordmark split**

Change `en.brand` to expose stable `player` and `vox` strings:

```ts
brand: {
  fullName: 'PlayerVox OverCrow',
  player: 'Player',
  vox: 'Vox',
  product: 'OverCrow',
},
```

Render the shared wordmark as:

```tsx
<strong>
  <span>{en.brand.player}</span>
  <span className="brand__accent">{en.brand.vox}</span>
</strong>
```

Add the focused website-brand rule while retaining the existing display face:

```css
.brand__wordmark strong {
  display: block;
  color: #fff;
  font-family: 'OverCrow Display', sans-serif;
  font-size: 1.4rem;
  font-weight: 900;
  font-style: italic;
  letter-spacing: -0.05em;
  text-transform: uppercase;
}

.brand__accent {
  color: var(--lime);
}
```

- [ ] **Step 4: Verify frontend behavior and production output**

Run:

```sh
npm --prefix crates/overcrow-control-ui test
npm --prefix crates/overcrow-control-ui run build
git diff --check
```

Expected: all 7 frontend tests pass, the Vite production build succeeds, and
the diff check emits no output.

- [ ] **Step 5: Commit the implementation**

```sh
git add crates/overcrow-control-ui/src/App.test.tsx \
  crates/overcrow-control-ui/src/components/Brand.tsx \
  crates/overcrow-control-ui/src/i18n/en.ts \
  crates/overcrow-control-ui/src/styles.css
git commit -m "style(control-ui): align the PlayerVox wordmark"
```
