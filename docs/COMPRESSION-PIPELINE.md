# Compression Pipeline

## Pipeline Summary

Pressel encodes images through a reversible, exact pipeline:

input image
→ normalize to RGBA8
→ split into 64x64 tiles
→ try reversible transforms
→ apply predictors
→ encode residuals
→ compress residual stream
→ choose smallest exact tile strategy
→ write `.prsl`

## Input Normalization

The encoder decodes the source image and converts it to RGBA8. This normalized raw RGBA byte stream is the canonical signal used by Pressel for hashing, verification, and exact roundtrip testing.

## Tiling

Images are split into 64x64 tiles. Edge tiles may be smaller. Each tile is encoded independently so different regions of an image can choose different reversible strategies.

## Reversible Transforms

### Raw RGBA

Transform `0` stores channel bytes unchanged before prediction.

### Subtract-Green

Transform `1` applies:

- `R' = R - G mod 256`
- `G' = G`
- `B' = B - G mod 256`
- `A' = A`

This may improve predictability when green carries shared structure with red and blue.

### Reversible YCoCg-R

Transform `2` converts `RGB` into a reversible luma/chroma-like representation using wrapping arithmetic. This can decorrelate natural-image color channels more effectively than raw RGB on some tiles.

### Alpha-Plane Separation

Transform `3` groups alpha values into a separate plane and then stores the color channels in their own planes. This is intended to help when alpha structure and color structure compress differently.

### Green Average Decorrelation

Transform `4` stores `G' = G - floor((R + B) / 2) mod 256` while preserving `R`, `B`, and `A` directly. This is a simple reversible decorrelation variant that can help on tiles where green tracks the average of red and blue.

### Palette/Index Packed Transform

Transform `5` is available only for suitable tiles. It builds an exact local palette and stores palette entries plus per-pixel indices inside the fixed-width transformed tile buffer. If a tile has too many unique colors or the packed representation would not fit, the transform is skipped for that tile.

## Predictors

Pressel v1 currently tries these predictors for every tile:

- None
- Left
- Top
- Average
- Paeth
- JPEG-LS MED-style predictor
- Adaptive 8x8 block predictor map

Predictors are applied per byte channel within the transformed tile. The residual is stored as:

`residual = actual - predicted mod 256`

and reconstructed as:

`actual = predicted + residual mod 256`

In the adaptive predictor mode, each 8x8 block inside a tile picks the best base predictor from the implemented set, and the block map is stored alongside the residual stream.

## Entropy Backends

Implemented backends:

- Raw residual stream
- Zstd residual stream

Every tile tries both backends. The encoder keeps whichever exact representation is smaller once tile metadata is included.

## Tile Strategy Search

For each tile, Pressel enumerates:

- every implemented transform
- every implemented predictor
- every implemented entropy backend

The smallest exact tile payload is selected and written into the `.prsl` container with its strategy identifiers.

## Future Work

Planned research directions include:

- Golomb-Rice residual coding
- rANS entropy coding
- QOI-style pixel cache
- JPEG XL-style weighted predictor
