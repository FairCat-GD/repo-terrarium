# terrarium-compose

Depiction: from `SceneInput` to pixels.

## Owns

- The native frame, `Rgba640x74`, and the scene's fixed geometry.
- The integer scale policy and the upscaler that applies it with padding, shared by the
  presenter and, later, every export.
- A placeholder frame in palette colours, until the compositor draws art.
- The asset catalog (`catalog`): every delivered sprite under its semantic ID, with its
  metadata and the atlas its pixels are packed into.
- The variant table (`variants`): the art that depicts each `SceneInput` variant.
- The composer, its tweens and animation.
- Frame scheduling hints.
- The CPU compositor, which draws a `Frame` into a 640×74 RGBA buffer.
- Bitmap text, nine-slice panels and GIF encoding.
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

The variant table maps every `SceneInput` variant to its art. `variants::missing` is check 6
of `cargo xtask assets`, and this crate's tests run it on the embedded art. The module's docs
list where the art and the enums are not one to one.

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

## Requirements

Invariants I-2, I-7, I-10, I-11 and I-15; budgets N-03 and N-12.
