# terrarium-compose

Depiction: from `SceneInput` to pixels.

## Owns

- The asset catalog.
- The composer, its tweens and animation.
- Frame scheduling hints.
- The CPU compositor, which draws a `Frame` into a 640×74 RGBA buffer.
- Bitmap text, nine-slice panels and GIF encoding.
- `InstallSalt`.

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
