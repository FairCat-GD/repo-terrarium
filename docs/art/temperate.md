# Temperate: art bible

The temperate biome is a woodland verge under glass: a "botanical cabinet" of trees, ferns and
low plants over dark soil, a pale sky, and small residents living their own hours. It must read
at a glance from 1.5 m, and even a dying terrarium has to look arranged.

## Palette

32 colours, each with a day and a night value. A sprite's PNG index is its slot plus one; index
0 is the only transparent entry. Night swaps every slot to its night colour before anything is
blended. The weather backdrops add 40 colours of their own, paired the same way, for 72 in all.
The source is `art-assets/biome-temperate/palettes/exports/palette-temperate.json`.

| Slot | Name | Day | Night | Used for |
|---|---|---|---|---|
| 0 | soil.deep | `#292720` | `#101518` | the darkest soil; label ink |
| 1 | soil.shadow | `#3A3227` | `#1B2022` | soil; panel centre |
| 2 | soil.body | `#514333` | `#2A2D2C` | soil |
| 3 | soil.light | `#6B5940` | `#3E3D35` | soil highlights |
| 4 | stone.shadow | `#797968` | `#454D4C` | stones, glass edge |
| 5 | stone.body | `#9A9983` | `#63716A` | stones |
| 6 | stone.light | `#BDB9A0` | `#87928A` | stones; panel text; rain |
| 7–10 | green.shadow, body, light, highlight | `#455638` … `#A4AD68` | `#263D35` … `#76865D` | thriving foliage |
| 11–13 | dormant.shadow, body, light | `#626B53` … `#A5A88A` | `#394B44` … `#758175` | dormant foliage |
| 14–15 | pale.body, light | `#ADB16B`, `#C6C685` | `#778455`, `#96A16C` | low-light foliage |
| 16–18 | amber.shadow, body, light | `#836044` … `#C39A64` | `#594535` … `#A08050` | wilting foliage, litter |
| 19–21 | bleached.shadow, body, light | `#748181` … `#C8CBC0` | `#5C7078` … `#ACB4B2` | dead plants, dead stems |
| 22–23 | stem.shadow, light | `#6C5438`, `#9C8054` | `#443F32`, `#726044` | stems and trunks |
| 24 | accent.warm | `#BF805E` | `#96664E` | blooms, berries |
| 25 | accent.cool | `#9B88A5` | `#766E91` | blooms |
| 26–29 | sky.base, cloud-light, cloud-shadow, overcast | `#E5DEC1` … `#C6CDBC` | `#4D6472` … `#405967` | sky |
| 30 | storm.body | `#969F99` | `#53666F` | storm |
| 31 | glow | `#EDF4C9` | `#DDEDB5` | night glows, firefly lantern |

Each plant state keeps to its own group: thriving green, low light pale, poor soil amber,
dormant grey-green, dead bleached blue-grey. Blooms use the accents, stems the stem pair.

## Value bands

Dark soil, mid-value plants, light sky, in every state and by day and night, so the strip reads
as three bands from across a room even when the terrarium is dead. The composer's test holds the
soil at least 8 luma darker than the plants and the sky, and by day the plants at least 8 darker
than open sky. A storm's sky is the one exception: its luma (about 163) sits among pale and
bleached foliage (157 to 200).

## Silhouettes

- **Twelve species, three heights.** Trees at the back (oak, birch, rowan), mid plants (fern,
  tall grass, bramble, foxglove, reed), near plants (moss mound, wildflower cluster, clover,
  thistle). Each terrarium grows 2 trees, 3 mid and 3 near species, chosen by its seed.
- **Every species reads in all five states and three stages by silhouette alone:** seedling,
  young and mature; thriving, pale, wilting, dormant and dead. Low light is tall, thin and leggy;
  poor soil droops; dormant is still; dead is bare.
- **Rhythm.** The strip is 108 cells, 36 to each third, banded tall, medium and small, with no
  two neighbouring cells in the same band. Each third has four tree cells, set as a pair, a grove
  or alone, differently in each third. No two neighbouring plants share a species, and no third
  holds more than four mature trees.
- **Occupancy.** Between about 35% and 70% of the strip holds something. A poor terrarium fills
  its gaps with lower growth, dead stems, litter, fungus and plain soil, never with healthy
  plants that do not exist. Past the ceiling, plants grow fuller rather than more numerous, and
  weeds past their cap merge into a patch.
- **One focal element per third,** and one message at a time: each state has a dominant look and
  the rest stays quiet.

## The states

- **Thriving.** Lush and orderly, plants in leaf and some in bloom, residents about, a clear sky.
  At night the plants and fungi glow faintly.
- **Mature.** Established large plants, standing stones, calm residents, very slow motion,
  like an old greenhouse that's well looked after.
- **Struggling, low light.** Tall, thin, washed-out plants in the pale greens.
- **Struggling, poor soil.** Amber, drooping plants with weed patches between them, litter and
  fungus at their base, so the ground carries the message.
- **Dormant.** Grey-green and still, residents asleep, condensation on the glass, thickening at
  30 and 90 days. Nothing rots, because a quiet repository isn't a sick one.
- **Dead.** Bleached plants, dead stems, litter and fungus, a resident or two at most, and after
  a year, dust on the glass, thickening at 18 months and two years. Even then the scene stays tidy.
- **Recovering.** Fresh growth among the litter, seedlings, residents returning.

## Residents

Eight species, five in each terrarium (at least one about by day and one by night, at most one
that flies): snail, ladybird, ground beetle, firefly, woodlouse, springtail, earthworm and
harvestman. Each has three or four life stages (egg, brood or cocoon; juvenile; pupa for the
ladybird, beetle and firefly; adult) and, as an adult, idle, move, feed, sleep, hide and death,
plus a routine of its own: the snail withdraws, the firefly flies and signals, the woodlouse
curls, the springtail hops, the earthworm retreats, the harvestman probes.

- All but the firefly are about by day and by night; the firefly only at night. The snail and
  the earthworm favour the hours after rain.
- A storm sends everyone into hiding.
- At most three move at once, at most one of them flying or signalling, each on its own phase.
- The firefly's lantern is a separate night-only glow, at least eight seconds between signals.

## Events

Each event has art of its own rather than a palette trick, and plays once:

- **Storm:** a dark front rolls in from one side; the **clearing** carries it on out the far side.
- **Lightning:** two frames with no screen flash, then a scorch on a mature plant (one scorched
  foliage per species), which fades back to the plant's own state.
- **Dusk and sunburst:** a multiply and an additive tint over the whole scene.
- **Flowering:** the mature plants' blooms, for a day. The flower sprinkle belongs to the
  **clearing burn**, which burns the weeds off in four frames and leaves fresh soil.
- **Windstorm:** leaves blown across the strip, and stronger sway.
- **Rain after drought:** rain across the strip.
- **Earthquake and sinkhole:** a crack and a sinkhole that settle and stay for up to 30 days; the
  sinkhole leaves a fossil (shell, bone or leaf print).

## The world around them

- **Sky:** four full-width backdrops (clear, overcast, haze, storm), the sun by day, the moon in
  its real phase and the stars by night, clouds only under a clear sky.
- **Ground:** soil tiles and edges, pebbles, tufts, fresh soil.
- **Weeds, litter and fungus:** creeping, tall and burr weeds and their patch; leaf piles, fallen
  leaves and dead stems; cap fungi, glowing at night. The bracket fungi are not drawn yet: they
  wait for dead wood to grow on.
- **Standing stones:** one per year of age, at most three, young, lichened and ancient, with up
  to three carved marks each for major releases.
- **Glass:** the edges and corners, condensation and dust in three levels each, and a faint
  reflection that sweeps across every 20 to 30 minutes.
- **Label and panels:** the 90×9 stone label plate, the 24×7 hover plate, a nine-slice panel,
  trend and freshness icons, and two public-domain bitmap fonts (4×6 and 5×7).

## The technical contract

- **Scene:** 640×74, up to 840 wide; the backdrops are 840×74 with the 640-wide centre at x 100.
  The ground line is y 65; plants stand on row 64. Whole-number scaling, nearest pixel only.
- **Plants:** every piece derives from an 80×64 canvas with its ground point at (40, 59).
  Seedlings are single images; young and mature plants are a stem, a foliage for each state and
  a bloom, drawn in that order. Sway shears the foliage about its pivot, never the stem.
- **Residents:** 32×24 frames anchored at (16, 19), facing right; mirrored for left.
- **Depth, back to front:** sky, far plants, stones, mid plants, weeds and fungus, plant glow,
  near plants, residents on the ground then in the air, litter, soil, ground, event overlays,
  glass, label, panel.
- **Motion:** at most 15 frames a second; 166 ms frames for sky and soil, 167 ms for creatures;
  authored durations set the pace. Every change tweens: state changes over 3 to 10 seconds,
  residents in 1.2 s, day and night over 45 minutes. Nothing cuts and the camera never moves.
- **Delivery:** new art goes into `art-assets/` exactly as it was drawn; `cargo xtask assets`
  checks every file's hash, the 14 active manifests, the palette indices, the animations and
  that every state has its art.
