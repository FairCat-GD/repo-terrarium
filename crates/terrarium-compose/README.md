# terrarium-compose

Depiction: from `SceneInput` to pixels.

## Owns

- The native frame, `Rgba640x74`, and the scene's fixed geometry.
- The integer scale policy and the upscaler that applies it with padding, shared by the
  presenter and, later, every export.
- The CPU compositor (`compositor`), which paints a `Frame` of sprite draws into the 640×74
  RGBA image, the one blend rule it and the palette share (`blend`), and the palette
  crossfade it paints with (`palette`).
- Bitmap text and the label plate (`text`), and the nine-slice panel (`nine_slice`).
- Damage: rewriting only what changed, equal to a full redraw (`scale::Damage`).
- The one PNG encoder (`export`), for exports, `cargo xtask render` and the snapshot tests.
- A sample scene assembled by hand (`sample`), shown until the live terrarium reaches the
  strip. It keeps its own bottom-right label, so its pictures never move.
- The asset catalog (`catalog`): every delivered sprite under its semantic ID, with its
  metadata and the atlas its pixels are packed into.
- The variant table (`variants`, `ArtTable`): the art that depicts each `SceneInput` variant,
  each species' depth band, and the filler art.
- The composer (`composer`), with its slots (`slots`), placement (`place`), plant assembly
  (`plant`), tweens (`tween`) and draw order (`order`), and the presets it is shown with
  (`preset`).
- The world around the plants: where each piece stands on an install, the fixed scenery and
  the padding (`world`); the sky's rules, from the moon's phase to the clouds a weather brings
  (`sky`); and the one-message table (`rules`).
- `InstallSalt` (`salt`).
- Animation, frame scheduling hints and GIF encoding, still to come.

## The embedded art

`assets/generated/<biome>/` holds two files that `cargo xtask assets` writes from `art-assets/`
. Both are committed, and `include_bytes!` embeds them:

- `atlas.bin`: the runtime atlas, one byte per pixel, row-major. Each byte is the sprite's raw
  PNG index: 0 is transparent, and 1..=32 is logical palette slot + 1.
- `catalog.bin`: the catalog in postcard, after a magic value and a format version. It holds:
  - every sprite's rect, anchor, layer, blend and opacity, its structured keys, pivots,
    motion and event data;
  - the animation sequences;
  - the fonts' glyph maps and metrics;
  - the UI metrics;
  - the residents' motion budget;
  - the day and night palette.

`cargo xtask verify` regenerates both and fails unless they are byte-identical. After an art
delivery, rewrite them with:

```sh
cargo xtask assets
```

A change to the catalog's types needs a `FORMAT` bump, so a stale file is refused rather than
misread. xtask builds on this crate, and this crate embeds the two files, so they must exist
for anything to compile. If they are ever lost, create them empty, then run `cargo xtask
assets`.

The variant table maps every `SceneInput` variant to its art. `ArtTable::new` resolves every
variant once, so the composer reads maps rather than scanning the catalog, and fails with
every gap. `variants::missing`, those gaps, is check 6 of `cargo xtask assets`, and this
crate's tests run it on the embedded art. The module's docs list where the art and the enums
are not one to one.

## The composer

`Composer::update(&SceneInput, &OverlayInput, now)` moves the depiction toward a scene from
`now`, and `Composer::frame` draws it at any moment and palette mix. The clock is always passed
in. `OverlayInput` is what the app gives beside truth: the label's lines, already worded, and
the local date, for the moon.

- **Slots** (`slots`). Thirty cells cross the strip, ten per third, each in a depth band: far,
  mid or near. The bands step tall, medium, small, and each third has two tree cells, spaced
  differently in each third. The install salt moves each slot up to 7 pixels within its cell,
  and nothing else.
- **Placement** (`place`). A pure function of the previous placement and the scene, over cells
  only, so the same history depicts the same plants on every install. The rules:
  - **bands:** a plant stands in its species' band;
  - **kept:** a plant keeps its cell while truth keeps it;
  - **neighbours:** neighbouring cells of a band never hold one species;
  - **trees:** a third holds at most two mature trees, of different species;
  - **ceiling:** at most 21 plants, mature ones first;
  - **floor:** filler brings the occupied cells to at least 11.

  Filler follows the scene's state and is never a living plant: grass tufts, the terrarium's
  own species dormant or dead, or dead-stem litter. Truth's weeds also take free cells, at most
  four, and above that merge into patches; they count toward the window, real cover before
  invented filler. Litter, fungus and the decorative tufts take no cell.
- **The world** (`world`, `sky`).
  - The sky: a 13-row weather band, drifting clouds, the sun by day, and the moon (in the
    local date's phase) and stars by night, with dusk and sunburst light over everything.
  - The ground: the soil band, pebbles, fossils, monuments with lichen glow and carved marks,
    weeds, litter, fungus and decorative tufts.
  - The glass: edges, corners, condensation and dust.
  - The label plate at the top left, cycling its lines every 20 s.

  Each kind of piece has a fixed set of places on an install, and piece `i` always stands in
  place `i`, so a count that changes never moves another piece. Night-only art fades with the
  palette mix, day-only art the other way.
- **One message** (`rules`). Each presentation state quiets the composer's own dressing
  (clouds, sun, stars, decorative tufts) so its dominant visual stands out. Truth's signals are
  never quieted.
- **Looks** (`composer`). Each subject has up to five parts: a plant's stem, foliage, bloom and
  night glow, or a monument's stone, three marks and lichen glow. A part that changes look
  follows its art: opaque art raises the new look over the old one before the old one falls,
  so a change never shows the sky through it; translucent and glowing art crossfades. An update
  never changes the frame at its own moment.
- **Order** (`order`). Within a layer, draws are sorted by their world point (where the anchor
  lands), then by subject, so a foliage always follows its stem. A subject whose parts must
  sort together, like a stone and its marks, is keyed at one point, and a look of several draws
  (the weather tiles, the label's glyphs) numbers them.

The constraints are property-tested over generated histories in `tests/composer.rs`:
occupancy, rhythm, palette discipline, one message, the healthy subset, no cuts, and salt
independence. A second property renders settled frames by day and by night: the soil band is
always the darkest, by day the plants sit between it and the sky, and with every translucent
draw left out each pixel is a palette colour. The crate is built at `opt-level = 1` in dev
builds, so these run in seconds.

## The compositor

A `Frame` is a list of `Draw`s, each a sprite with its position, layer, flip, opacity, blend,
shear, tint and clip, plus the frame's `palette_mix` and `dim`. The compositor paints them in
order over a sky of flat `sky.base`, in gamma-encoded sRGB with nothing linearized:

```text
out = (dst × (255 − a) × 255 + B × a + 32512) / 65025
B   = src × 255 (normal) | dst × src (multiply) | min(dst + src, 255) × 255 (add)
```

That is `dst + (blend − dst) × a / 255` rounded once, to nearest; 65025 is odd, so there is
never a tie. Each frame first looks every slot up in the day and night palettes and blends the
two by `palette_mix`, so night is swapped in before compositing. `dim` darkens the finished
frame toward black by the same rule.

The pictures are pinned by exact snapshots in `tests/pixels.rs`. Each is kept as a PNG to look
at and as the SHA-256 of its raw pixels, so a new `png` release that encodes differently shows
apart from a pixel that changed. After a deliberate change, re-bless them and look at every PNG
that moved:

```sh
INSTA_UPDATE=always cargo nextest run -p terrarium-compose --test pixels
```

## Why it is a crate of its own

Every pixel is composed on the CPU at native size, so the strip, the time-lapse, the exports and
the snapshot tests share one byte stream. Depending on truth alone keeps the composer from
reaching past `SceneInput` into live data.

## Rules

- It depends on no workspace crate except `terrarium-truth`, and reads only `SceneInput` and its
  enums from it.
- Floats are allowed, but the output is deterministic for a given input, salt and clock.
- It never fabricates health: the healthy plants drawn are a subset of those in `SceneInput`
  (I-7).
- It reads no clock and iterates no hash map; `clippy.toml` holds that.
- Every visible change tweens, and the camera never moves (I-15). At most 15 frames a second,
  and none while nothing changes (I-10).
- Art is used as delivered (I-11).
- It draws the words it is given and authors none; they live in the app's copy module (I-8).

## Requirements

Invariants I-2, I-7, I-10, I-11 and I-15; budgets N-03 and N-12.
