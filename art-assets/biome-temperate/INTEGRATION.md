# Temperate biome — integration contracts

## Import and rendering

- Resolve catalog manifest paths relative to the directory containing `biome-temperate/active-manifests.json`. Resolve each manifest's `atlas` and sprite `file` relative to that manifest. Load the 14 listed manifests only; never recursively merge every JSON file.
- Atlas and individual PNGs contain the same pixels; choose either import route. If repacking into a runtime atlas, retain semantic IDs, metadata and pixel-exact art. The supplied section atlases are delivery units, not a mandated GPU layout.
- Schema shapes vary by batch: atlas size may be `width`/`height` or `canvas`; animations may be top-level sequences or per-sprite `animation`, `frame`, `durationMs`, `loop`. Parse the actual fields rather than assuming one Aseprite-export schema. Missing opacity/blend means opaque/normal; explicit values take precedence.
- Use nearest-neighbour sampling and integer pixel placement/scaling. Native scene is **640×74**; exact 3× is **1920×222**. Preserve these proportions without distortion or cropping.
- Palette data is `biome-temperate/palettes/exports/palette-temperate.json`. There are 32 opaque semantic slots plus transparency. Source masks use binary alpha. For night, map day colours to corresponding night slots before compositing; do not quantize a composited scene back into the source palette.
- Apply `opacity / 255` at draw time, with each sprite's `blend`. Dusk is multiply; sunburst and designated fungus/lichen glows use additive blending. Firefly lantern masks explicitly use normal blending. Do not flatten translucent overlays into opaque atlas art.
- Follow this depth order and each sprite’s `layer`: sky, far plants, monuments, mid plants, weeds/fungus, near plants, creatures, ground litter, soil band, event overlays, glass, label, soil panel. Scorch belongs at its host plant's depth rather than the general event-overlay layer. Preserve negative space; the reference scenes are diagnostic compositions.
- Place anchored images at `(worldX - anchor.x, worldY - anchor.y)`. Sky/glass/full-width overlays use local top-left anchors. Respect `tileX`, paired effects, attachment points, pivots and per-sprite placement rules. Trimmed anchors/pivots may legitimately lie outside opaque bounds.

## Plants and events

- Seedlings are combined images. Young/mature plants use shared stem first, then the chosen state foliage at the same ground point. Dead foliage includes its authored bleached stem override. Use the manifest's shear pivot, not the centre of a trimmed rectangle.
- Mature reproductive overlays go over mature thriving plants when authorized by the simulation/event. Rowan berries are already in mature thriving; foxglove bells are in the bloom overlay. Do not treat every plant as a generic flower.
- Scorch is a temporary mature-foliage replacement over the existing shared stem, not a sixth health state. Eligibility, replacement family, transition envelope and bloom suppression are in the event manifest. Return to the current simulation-selected foliage, never automatically to thriving.
- Burn-off plays once for 940 ms, then removes its final ash frame. Preserve the per-frame weed/fresh-soil opacity envelope and transition metadata. Flower sprinkles are finite separate overlays, not persistent simulated plants.
- Earthquake and sinkhole retain settled terminal poses; a generic rule to remove every one-shot would be wrong. Read each sequence's terminal contract.
- Rain streaks belong to rain-after-drought. Condensation indicates dormancy; dust requires the dead state for a year. Storm or inactivity alone must not trigger these substitutes. Arbitrate one foreground event; unresolved priorities/cooldowns belong to application design.

## Residents

The delivered working pool is snail, ladybird, ground beetle, firefly, woodlouse, springtail, earthworm and harvestman.

- All frames have fixed 32×24 registration and ground anchor `(16,19)`, facing right. Mirror anatomy and attached effects together for left facing, use x anchor 15, and negate `rootDelta.x`.
- Apply each frame's `rootDelta` **after its hold**, exactly once per elapsed frame, including skipped render frames. It is world movement, not anchor movement. Do not slide resting/feeding/idle poses.
- Use authored durations, frame order, `loop`, `playCount` and terminal behavior. Looping routines repeat only while selected. Finite hatch, eclosion, emergence, landing and hop hand off to their declared destinations; death holds until simulation removal. Do not infer birth, healing or removal from the editor animation itself.
- Juveniles have independent artwork. Their baseline routines are idle/move/feed; shelter or sleep can hold juvenile idle. Never substitute an adult sprite for a juvenile.
- Ladybird, beetle and firefly have larva/pupa/adult stages. Woodlouse uses protected brood, earthworm a cocoon, and springtail/harvestman eggs followed by juveniles/adults. These four have no pupa.
- The firefly lantern uses separate night-only masks synchronized to signal, with per-frame opacity 0/64/96/0 and normal blending. Attach it to the same registration origin; remove after its zero-opacity terminal frame. A flight origin may travel above the substrate but remains the landing reference.
- Both batches share a **proposed** maximum of three moving residents, including at most one flying/signalling firefly. This is not three per batch. They also share the application's eventual global motion cap with plants/events; the final global number remains a product decision. Phase offsets and 1200 ms external transition proposals are in the manifests.
- The manifests specify a 15 fps render ceiling. Authored frame durations still govern animation time; avoid synchronized constant motion and unnecessary redraws of static scenes.

## UI and fonts

- Nine-slice panel tiles are 3×3, with tiled edges/centre and minimum 9×9 panel size. Use the UI manifest's insets.
- The small hover plate is 24×7 with a 20×5 safe text area: up to five short uppercase body-font characters without descenders. The separate main label is 90×9; use its text inset for mixed-case labels.
- Body font: 4×6 cells, 95 printable ASCII characters. Heading: 5×7 cells, 65 uppercase/digit/punctuation cells; lowercase may map to uppercase. There are 160 glyph assets across the two fonts, not 160 distinct characters.
- Read per-font advance, ascent, descent, line height and fallback from `fonts-batch-07.json`. `.fnt` files are supplied too; their exporter serializes some integral fields as `4.0`, so consumers requiring integer-only BMFont tokens should normalize those numbers or use JSON metrics.
- Glyph ink is a mask: slot 7 on dark panels; slot 1 for the stone main label. No smoothing or resampling. Non-ASCII repository names/localization require a separate fallback policy. Font sources and public-domain notices are included.

## Verification to perform in the application

Asset validation already covered source/export agreement, binary alpha, palette, bounds, anchors, exact integer scaling, animation timing and lifecycle destinations. Application integration must still verify the actual palette swap, blending/depth, plant assembly, contact during movement, root motion, one-shot termination and global motion/event arbitration.

Inspect day/night at 1× and exact integer display scale. Tiny night forms and monitor-distance readability remain human acceptance work. Two coarse moss state-silhouette metrics remain diagnostic notes; the authored contours were reviewed, and the 2-pixel dead moss seedling is an intentional broken remnant. Do not alter art simply to satisfy a generic size/connectivity heuristic.
