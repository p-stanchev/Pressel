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

## Predictors

Pressel v1 currently tries these predictors for every tile:

- None
- Left
- Top
- Average
- Paeth
- JPEG-LS MED-style predictor

Predictors are applied per byte channel within the transformed tile. The residual is stored as:

`residual = actual - predicted mod 256`

and reconstructed as:

`actual = predicted + residual mod 256`

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
- adaptive block predictor maps
- palette/index transforms
- QOI-style pixel cache
- reversible YCoCg-R
- JPEG XL-style weighted predictor
