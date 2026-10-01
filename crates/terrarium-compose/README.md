# terrarium-compose

Depiction: from `SceneInput` to pixels.

## Owns

- The native frame, `Rgba640x74`, and the scene's fixed geometry.
- The integer scale policy and the upscaler that applies it with padding, shared by the
  presenter and, later, every export.
- The CPU compositor (`compositor`), which paints a `Frame` of sprite draws into the 640×74
  RGBA image, and the palette crossfade it paints with (`palette`).
- Bitmap text and the label plate (`text`), and the nine-slice panel (`nine_slice`).
- Damage: rewriting only what changed, equal to a full redraw (`scale::Damage`).
- The one PNG encoder (`export`), for exports, `cargo xtask render` and the snapshot tests.
- A sample scene assembled by hand (`sample`), shown until the composer draws the terrarium.
- The asset catalog (`catalog`): every delivered sprite under its semantic ID, with its
  metadata and the atlas its pixels are packed into.
- The variant table (`variants`, `ArtTable`): the art that depicts each `SceneInput` variant.
- The composer, its tweens and animation.
- Frame scheduling hints.
- GIF encoding.
- `InstallSalt`.

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
- Every visible change tweens, and the camera never moves (I-15). At most 15 frames a second,
  and none while nothing changes (I-10).
- Art is used as delivered (I-11).
- It draws the words it is given and authors none; they live in the app's copy module (I-8).

## Requirements

Invariants I-2, I-7, I-10, I-11 and I-15; budgets N-03 and N-12.
