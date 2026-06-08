# PRSL Format

## Overview

`.prsl` is the custom Pressel container format for strict lossless image coding.

- Extension: `.prsl`
- Magic bytes: `PRSL1`
- Pixel model in v1: RGBA8 only
- Tiling: rectangular tiles with per-file `tile_size`

Pressel v1 stores enough metadata to decode the image exactly and verify that the reconstructed RGBA byte stream matches the original input.

It does not store the original PNG bitstream itself. As a result, decoding to PNG reproduces the image pixels exactly, but not necessarily the original PNG file bytes, metadata layout, or compression structure.

## Container Header

The v1 header stores:

- magic bytes: `PRSL1`
- width: `u32`
- height: `u32`
- channels: `u8`
- tile size: `u16`
- tile count: `u32`
- original pixel hash: SHA-256 of raw RGBA bytes

The current implementation uses `channels = 4` for RGBA8.

All integer fields in `.prsl` v1 are encoded little-endian unless explicitly stated otherwise.

After the required tile data, v1 may optionally store tagged trailing sections:

- `0x0001`: preserved PNG metadata chunk records
- `0x0002`: preserved ancillary PNG chunk records
- `0x0003`: original source file blob

Each section is encoded as:

- `tag_type: u16`
- `tag_len: u64`
- `tag_payload: [u8; tag_len]`

These sections are optional. Default Pressel files omit them.

After all tile records, the decoder reads optional tagged sections until EOF. Unknown tag types must be skipped using `tag_len`. Duplicate known tags are currently invalid unless a future version explicitly permits them.

## Tile Layout

Each tile stores:

- `x: u32`
- `y: u32`
- `width: u16`
- `height: u16`
- `transform_id: u8`
- `predictor_id: u8`
- `entropy_backend_id: u8`
- `compressed_payload_len: u32`
- `compressed_payload: [u8]`

Tiles are independent coding units. Edge tiles may be smaller than the chosen file tile size.

## Optional PNG Preservation Records

Pressel regenerates critical PNG structure from decoded RGBA by default. It can optionally preserve extra PNG data for later export:

- metadata mode stores a curated subset of useful ancillary metadata chunks
- chunk mode stores all ancillary chunks with placement/order metadata
- source-file mode stores the entire original PNG file byte-for-byte

Chunk mode subsumes metadata mode. If both are enabled, ancillary chunks are stored once in chunk mode and metadata is not duplicated.

Each preserved chunk record stores:

- `chunk_type: [u8; 4]`
- `placement: u8`
- `flags: u8`
- `original_crc: u32`
- `data_len: u32`
- `data: [u8; data_len]`

Current placement values:

- `0`: before `PLTE`
- `1`: before `IDAT`
- `2`: after `IDAT`
- `3`: before `IEND`

Current flags:

- bit `0`: ancillary
- bit `1`: safe-to-copy
- bit `2`: known common metadata
- bit `3`: unsafe to restore without warning

Critical PNG chunks such as `IHDR`, `IDAT`, and `IEND` are not stored in metadata/chunk preservation mode. Exact original-file recovery is only available through the original-source-file tag.

## Transform IDs

Implemented in v1:

- `0`: Raw RGBA
- `1`: Subtract-green
- `2`: Reversible YCoCg-R
- `3`: Alpha-plane separation
- `4`: Green average decorrelation
- `5`: Fixed-width palette/index packed transform for suitable tiles
- `6`: Structured exact plane transform
- `7`: QOI-style pixel-cache transform

Subtract-green uses:

- `R' = R - G mod 256`
- `G' = G`
- `B' = B - G mod 256`
- `A' = A`

The inverse adds green back modulo 256.

Reversible YCoCg-R stores luma/chroma-like channel combinations using wrapping 8-bit arithmetic and reconstructs the original `R`, `G`, and `B` channels exactly.

Alpha-plane separation stores the alpha channel as a separate leading plane followed by grouped color planes. This can improve compression on images whose alpha structure differs from their color structure.

Green average decorrelation keeps `R` and `B` unchanged while storing `G' = G - floor((R + B) / 2) mod 256`.

The fixed-width palette/index transform is only used on tiles where the exact colors fit into a compact palette representation inside the transformed tile buffer.

The structured exact plane transform splits RGBA into separate planes and lets each plane choose an exact reversible submode. Current plane submodes include:

- raw plane storage
- constant plane storage
- global affine sparse plane modeling
- row-affine sparse plane modeling
- palette RLE plane modeling
- palette bit-packed plane modeling
- block-pulse plane modeling

These submodes are internal to transform `6` and remain strictly lossless because they reconstruct the original plane bytes exactly.

The QOI-style pixel-cache transform is an exact variable-length tile transform that uses a 64-entry RGBA hash cache plus run, index, small-difference, luma-like, RGB, and RGBA opcodes. It is not QOI bitstream-compatible, but it borrows the same exact cache/update idea inside Pressel's tile search.

## Predictor IDs

Implemented in v1:

- `0`: None
- `1`: Left
- `2`: Top
- `3`: Average
- `4`: Paeth
- `5`: JPEG-LS MED-style predictor
- `6`: Adaptive 8x8 block predictor map
- `7`: Edge-guided deterministic predictor
- `8`: Photo-guided RGB predictor

Predictors operate on transformed per-channel bytes. Residuals are encoded modulo 256.

The adaptive predictor mode stores a compact per-block predictor map at the start of the residual stream, then encodes the tile using the selected predictor for each 8x8 block. This keeps the outer tile container unchanged while allowing more local predictor selection within a tile.

The edge-guided predictor chooses among left-, top-, and clamped-gradient-style behavior per sample using only already-known neighbors, so it needs no side map.

The photo-guided predictor stores a small per-tile prefix inside the residual stream that selects:

- the spatial predictor used for green
- the spatial predictor used as the chroma base
- a fixed-point coupling coefficient for red
- a fixed-point coupling coefficient for blue

It reconstructs green first, then predicts red and blue from already-reconstructed green plus chroma base predictors. This is intended to better follow natural-image RGB edge correlation while remaining strictly reversible.

## Entropy Backend IDs

Implemented in v1:

- `0`: Raw residual stream
- `1`: Zstd-compressed residual or special-transform payload stream
- `2`: Folded residual stream
- `3`: Zstd-compressed folded residual stream
- `4`: Zstd-compressed channel-separated residual stream
- `5`: Zstd-compressed folded channel-separated residual stream
- `6`: Static rANS-compressed folded residual stream
- `7`: Zstd-compressed context-adaptive folded residual stream
- `8`: Context-adaptive folded rANS residual stream

Backends `2` and `3` are only valid for predictor residual streams. They apply an exact reversible residual folding map that brings small signed prediction errors closer together in byte space before optional compression. This is intended to help natural-image residual distributions remain more compressible without changing any decoded pixel values.

Backends `4` and `5` are also valid only for predictor residual streams. They split residual bytes into exact per-channel streams and compress those streams separately after preserving any adaptive predictor-map bytes. Backend `5` also applies the reversible residual folding map to each channel stream before compression.

Backend `6` is also valid only for predictor residual streams. It folds residual bytes exactly and then encodes them with a static order-0 rANS model reconstructed from the stored normalized frequency table.

Backend `7` is also valid only for predictor residual streams. It preserves any residual prefix bytes, folds the remaining residual body exactly, assigns each folded residual byte to a deterministic context based on channel and local activity, and compresses those context streams independently. The decoder reconstructs the same contexts from already-decoded folded residual neighbors.

Backend `8` is also valid only for predictor residual streams. It uses the same deterministic folded residual contexts as backend `7`, but stores each context stream through a sparse-table static rANS payload instead of Zstd. This is a deeper custom entropy step than the context-split Zstd path while remaining exactly reversible.

## Decoding Process

Decoding proceeds as follows:

1. Read and validate the container header.
2. Read tile metadata and payloads.
3. Decode the tile payload with the selected entropy backend.
4. Reconstruct transformed bytes from predictor residuals.
5. Reverse the selected color transform.
6. Write the tile into the output RGBA image buffer.
7. Parse any optional tagged preservation sections.
8. Verify the SHA-256 hash of the full raw RGBA output.

## Verification Process

The `verify` command:

1. Loads the original input image.
2. Normalizes it to RGBA8.
3. Decodes the `.prsl` file.
4. Compares raw RGBA bytes exactly.
5. Prints original hash, decoded hash, and exact-match status.

This is strict decoded-pixel verification, not merely visual inspection.
