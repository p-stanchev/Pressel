# Compression Pipeline

## Pipeline Summary

Pressel encodes images through a reversible, exact pipeline:

input image
→ normalize to RGBA8
→ search tile sizes
→ split into tiles
→ try reversible transforms
→ apply predictors
→ encode residuals
→ compress residual stream
→ choose smallest exact tile strategy
→ write `.prsl`

## Input Normalization

The encoder decodes the source image and converts it to RGBA8. This normalized raw RGBA byte stream is the canonical signal used by Pressel for hashing, verification, and exact roundtrip testing.

## Tiling

The encoder currently searches a small set of whole-image tile sizes starting at `64` and chooses the file that compresses best exactly. For the chosen tile size, each tile is encoded independently so different regions of an image can choose different reversible strategies. Tile encoding can be parallelized through the CLI `--cores` flag without changing the exact decoded result.

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

### Structured Exact Plane Transform

Transform `6` splits the tile into separate `R`, `G`, `B`, and `A` planes and lets each plane choose a compact exact submode. Current submodes include:

- raw plane storage
- constant plane storage
- global affine sparse plane modeling
- row-affine sparse plane modeling
- palette RLE plane modeling
- palette bit-packed plane modeling
- block-pulse plane modeling

This transform is aimed at low-cardinality, patterned, or nearly affine channels that do not compress well when forced through one bytewise tile model.

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
- Folded residual stream
- Zstd over folded residual stream
- Zstd over channel-separated residual streams
- Zstd over folded, channel-separated residual streams

For bytewise transformed residual streams, every tile tries all implemented entropy backends. The folded residual variants are exact reversible remaps of modulo-256 residual bytes that cluster small signed errors closer together before optional compression. The channel-separated variants preserve any adaptive predictor-map prefix, split the remaining residuals into exact per-channel streams, optionally fold those channel streams, and then compress them independently. For the structured exact plane transform, the encoder stores its exact transform payload through the raw-vs-Zstd choice only, because residual-specific backends are defined for predictor residual streams rather than arbitrary transform payload bytes.

## Tile Strategy Search

For each tile, Pressel enumerates:

- every implemented transform
- every implemented predictor
- every implemented entropy backend

The smallest exact tile payload is selected and written into the `.prsl` container with its strategy identifiers.

## Future Work

Planned research directions include:

- rANS entropy coding
- QOI-style pixel cache
- JPEG XL-style weighted predictor
