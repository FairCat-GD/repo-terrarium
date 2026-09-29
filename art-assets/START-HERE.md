# Repo Terrarium — art assets and usage

Art snapshot: 28 September 2026, including the final creature cleanup. Handoff organized by biome on 29 September 2026.

## Start here

1. Read `biome-temperate/INTEGRATION.md` for loading, placement, palette, animation and text-rendering instructions.
2. Load only the manifests listed in `biome-temperate/active-manifests.json`.
3. Use `biome-temperate/references/` for day/night appearance and scale examples.

The temperate biome contains **778 unique IDs in 14 manifests: 618 art pieces and 160 bitmap glyph cells**. These include 216 base plant pieces and 264 creature pieces across eight families. Exact revisions are in the catalog.

Included: active PNG atlases, individual PNGs, manifests, day/night palette, font metrics and font source/license records. Paths within each asset category are preserved; catalog paths resolve relative to their biome folder. The four reference images show creature lineups and grounded scenes; they are appearance examples rather than fixed layouts.

`PACKAGE-CONTENTS.json` records SHA256 for every supplied file except itself. Pixel assets and rendering values are unchanged from production; one explanatory motion-budget note in the packaged creature manifest has been shortened.

## Biome folders

`biome-temperate/` is the first biome. It contains its own catalog, usage instructions, asset categories and references. Future biomes can be added as sibling `biome-<name>/` folders with independent catalogs. Keep existing semantic IDs unchanged and load assets within the selected biome.

## For integrators

> Read START-HERE.md and biome-temperate/INTEGRATION.md. Use only the assets listed in biome-temperate/active-manifests.json. Preserve IDs, anchors, palette slots and animation contracts. Verify day/night rendering, plant assembly, creature grounding and finite animation transitions when integrating the assets.

## Source files

Editable Aseprite files are kept in the authoring workspace. Manifest `source`, `recipe` and other provenance fields may refer to those omitted files; they are not runtime dependencies. Request the relevant source files if changing the artwork itself.
