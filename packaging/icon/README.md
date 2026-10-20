# The app icon

The three PNGs here are the app's icon. `cargo xtask package` makes the Mac's `.icns` and the
Windows `.ico` from them, scaling by whole numbers only, and checks them first.

Today's files are a **placeholder**, composed from the delivered sprites. The real icon replaces
them before the first release.

## What to deliver

- **Three PNGs, each drawn at its own size:** `icon-16.png` (16×16), `icon-32.png` (32×32) and
  `icon-64.png` (64×64). `icon-48.png` (48×48), for Windows' large icons, is optional; without
  it Windows shows the 16 px icon at three times its size.
- **A transparent background,** sRGB colour. The art is licensed CC BY-SA 4.0 like the rest of
  `art-assets/`.
- **A rounded square, centred:** the art fills it, every pixel inside it opaque, none outside
  it. macOS 26 and later puts an icon of any other shape on a grey tile of its own.
  - The margin on every side: 7 px at 64, 5 px at 48, 3 px at 32, 1 px at 16.
  - The corners are rounded, with a radius of about a fifth of the square's side (11 px at 64,
    6 px at 32, 3 px at 16), in pixel steps. Pixels on the edge itself may go either way: the
    check gives the outline about a pixel of play.
- **Windows** uses the same images.

They go here as delivered, never repainted.

## How they are used

| Made | From | Scale |
|---|---|---|
| Mac 16 | `icon-16.png` | ×1 |
| Mac 32, 16@2x | `icon-32.png` | ×1 |
| Mac 32@2x | `icon-64.png` | ×1 |
| Mac 128, 256, 512, 1024 (and the @2x of 64 to 512) | `icon-64.png` | ×2, ×4, ×8, ×16 |
| Windows 16, 32, 64 | `icon-16.png`, `icon-32.png`, `icon-64.png` | ×1 |
| Windows 48 | `icon-48.png`, else `icon-16.png` | ×1, else ×3 |
| Windows 256 | `icon-64.png` | ×4 |

The check fails on a file of the wrong size, a pixel outside the rounded square that is not
transparent, or one inside it that is not opaque.
