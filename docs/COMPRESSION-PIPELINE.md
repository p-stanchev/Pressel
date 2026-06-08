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
→ optionally store PNG preservation sections
→ write `.prsl`

## Input Normalization

The encoder decodes the source image and converts it to RGBA8. This normalized raw RGBA byte stream is the canonical signal used by Pressel for hashing, verification, and exact roundtrip testing.

If PNG preservation flags are enabled and the input is a PNG, the encoder can also parse ancillary PNG chunks or preserve the full original file as optional side data. This side data does not change the decoded RGBA correctness rule.

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

### Seeded QOI-Style Pixel-Cache Transform

Transform `7` is a variable-length exact tile transform built around a small seed palette of common tile colors, a 64-entry RGBA cache hash, short runs, small RGB deltas, luma-like deltas, and raw RGB/RGBA escape opcodes.

It is not QOI-compatible at the file level. Instead, it gives Pressel a cache-oriented exact tile representation that can compete with the other reversible tile strategies during search, while the seed palette makes the cache friendlier on repeated local colors.

## Predictors

Pressel v1 currently tries these predictors for every tile:

- None
- Left
- Top
- Average
- Paeth
- JPEG-LS MED-style predictor
- Adaptive 8x8 block predictor map
- Edge-guided deterministic predictor
- Photo-guided RGB predictor
- Weighted gradient predictor

Predictors are applied per byte channel within the transformed tile. The residual is stored as:

`residual = actual - predicted mod 256`

and reconstructed as:

`actual = predicted + residual mod 256`

In the adaptive predictor mode, each 8x8 block inside a tile picks the best base predictor from the implemented set, and the block map is stored alongside the residual stream.

The edge-guided predictor chooses between left-, top-, and clamped-gradient-style prediction per sample using only already-decoded neighbors, so it adds no side data.

The photo-guided predictor stores a compact per-tile prefix that selects green/chroma base predictors and fixed-point green-coupling coefficients for red and blue. It reconstructs green first and then predicts red and blue from the reconstructed green value, which is aimed at photo-like RGB edge correlation.

The weighted gradient predictor blends left/top support with a clamped gradient estimate using only already-decoded neighbors. It is intended as a stronger general natural-image predictor than a plain average while remaining deterministic and exactly reversible.

## Entropy Backends

Implemented backends:

- Raw residual stream
- Zstd residual stream
- Folded residual stream
- Zstd over folded residual stream
- Zstd over channel-separated residual streams
- Zstd over folded, channel-separated residual streams
- Static rANS over folded residual streams
- Zstd over context-adaptive folded residual streams
- Context-adaptive folded rANS residual streams
- Context-adaptive folded arithmetic/range residual streams

For bytewise transformed residual streams, every tile tries all implemented entropy backends. The folded residual variants are exact reversible remaps of modulo-256 residual bytes that cluster small signed errors closer together before optional compression. The channel-separated variants preserve any adaptive predictor-map prefix, split the remaining residuals into exact per-channel streams, optionally fold those channel streams, and then compress them independently. The static rANS backend uses a stored normalized frequency table over folded residual symbols, which is a first step toward a more custom entropy path than generic Zstd. The context-adaptive folded backends preserve prefix bytes, fold the residual body, bin symbols by deterministic channel-and-activity contexts, and then either compress those context streams independently with Zstd or encode them through sparse-table rANS or range-coded payloads. For the structured exact plane transform and seeded QOI-style pixel-cache transform, the encoder stores exact transform payloads through the raw-vs-Zstd choice only, because residual-specific backends are defined for predictor residual streams rather than arbitrary transform payload bytes.

## Tile Strategy Search

For each tile, Pressel enumerates every compatible transform, predictor, and entropy backend combination.

Some transforms and backends are only valid for specific payload types, so invalid combinations are skipped rather than forced through the search.

The current encoder also applies lightweight per-tile classification heuristics before the full exact search. Opaque, flat, low-cardinality, cache-friendly, and photo-like tile signals are used to trim obviously poor transform and predictor combinations without changing the format or weakening exactness.

The smallest exact tile payload is selected and written into the `.prsl` container with its strategy identifiers.

## Optional PNG Preservation

After the required image and tile payloads are selected, Pressel may append optional tagged sections:

- preserved common PNG metadata chunks
- preserved ancillary PNG chunk records
- preserved original source file bytes

These sections are independent of the core RGBA coding path.

- default mode stores none of them
- metadata mode stores only a curated common subset
- chunk mode stores all ancillary chunks and subsumes metadata mode
- source-file mode stores the original PNG byte-for-byte

When exporting PNG from `.prsl`, Pressel regenerates critical PNG structure from decoded RGBA and then reattaches preserved ancillary chunks only when their placement is valid and safe.

## Future Work

Planned research directions include:

- deeper learned or multi-feature arithmetic-coded context modeling beyond the current folded context-range backend
- stronger hybrid palette/cache transforms beyond the current seeded QOI-style tile mode
- richer semantic per-tile image classification and recursive split heuristics
