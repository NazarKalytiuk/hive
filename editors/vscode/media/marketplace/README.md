# Tarn VS Code — Marketplace assets

This folder holds the artwork referenced by the Marketplace listing
(`galleryBanner`, the extension `icon` in `package.json`, and the banner
inlined at the top of `editors/vscode/README.md`).

The PNGs are generated programmatically from the brand mark in
[`media/tarn-icon.svg`](../tarn-icon.svg) so the activity-bar icon and
the marketplace icon read as the same brand. If the brand evolves,
re-run the generator and replace these files in a single commit.

## Brand

- `galleryBanner.color`: `#1E1B4B` (deep indigo).
- `galleryBanner.theme`: `dark`.
- Foreground: pure white.
- Mark: 3 horizontal lines + 1 circle (mirrors `media/tarn-icon.svg`).

## Shipped assets

### `icon.png`

- **Size**: 128×128, PNG, RGBA.
- **Use**: extension icon in the Marketplace listing and the Extensions
  view (`package.json` top-level `"icon"` field).
- **Composition**: rounded-square indigo background with the brand mark
  centered.

### `banner.png`

- **Size**: 1376×400, PNG, sRGB.
- **Use**: hero image at the top of `editors/vscode/README.md`, which is
  also what the Marketplace renders as the gallery header.
- **Composition**: indigo background, centered mark + `tarn` wordmark
  with the tagline *"Run, debug, and iterate on API tests — without
  leaving the editor."* underneath.

## Regenerating

The PNGs are produced by a small Pillow script that draws at 4× and
downscales with LANCZOS for smooth anti-aliased edges (no rsvg/cairo
dependency). To regenerate after a brand change:

1. Edit the brand constants (`INDIGO`, mark proportions, tagline) in the
   generator script.
2. Run it against this directory:
   ```bash
   python3 path/to/gen_marketplace_art.py editors/vscode/media/marketplace
   ```
3. Verify both PNGs render correctly in the Marketplace preview before
   tagging a release: `npx @vscode/vsce package && code --install-extension <vsix>`.

## UI screenshots

Real VS Code screenshots (Test Explorer tree, peek-view diff, CodeLens,
environment picker, streaming output) and the diagnosis-loop demo GIF
require a live editor session against a fixture workspace and are
captured by hand. They are intentionally **not** referenced from the
README at the moment so the listing has zero broken images. Add them
back in the same commit that introduces the captured PNGs.
