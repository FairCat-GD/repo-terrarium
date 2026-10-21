# The DMG's picture

The picture behind the Mac installer's window: the app on the left, the Applications folder on
the right, and the person drags one onto the other. `cargo xtask package` checks the two files
here and combines them, as delivered, into the one TIFF Finder shows.

Today's picture is warm ivory, a woodland verge along the bottom and an arrow in the middle
("Repo Terrarium — Installer background", CC BY-SA 4.0).

## What to deliver

- **Two PNGs,** opaque, sRGB colour:
  - `background.png`, 660×400 pixels: the window's size in points;
  - `background@2x.png`, 1320×800 pixels: the same picture for Retina displays.
- **No words.** One picture serves every language.
- **The art is licensed CC BY-SA 4.0,** like the rest of `art-assets/`.

## Where things fall

In window points, from the top left.

- **The app's icon:** centred at (180, 200), 80 points square.
- **The Applications icon:** centred at (480, 200), 80 points square.
- **Their names:** Finder writes "Repo Terrarium" and "Applications" just under each icon,
  centred. It draws them in black when the Mac is in light mode and in white in dark mode, and
  the picture cannot change that, so whatever sits under the names reads best in a mid tone,
  with a contrast of at least 4.5:1 against black and against white (such as `#777766`).
- **Everything else is free:** the icons sit on top of the picture.

The places and the size are `dmg::APP_AT`, `LINK_AT` and `ICON_SIZE` in
`xtask/src/package/dmg/mod.rs`; a new picture drawn for other places changes them there.

It goes here as delivered, never repainted.

## What the check refuses

- A file of another size.
- A pixel that is not fully opaque: Finder would show its own white through it.
