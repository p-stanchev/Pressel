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

## Transform IDs

Implemented in v1:

- `0`: Raw RGBA
- `1`: Subtract-green
- `2`: Reversible YCoCg-R
- `3`: Alpha-plane separation
- `4`: Green average decorrelation
- `5`: Fixed-width palette/index packed transform for suitable tiles
- `6`: Structured exact plane transform

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

## Predictor IDs

Implemented in v1:

- `0`: None
- `1`: Left
- `2`: Top
- `3`: Average
- `4`: Paeth
- `5`: JPEG-LS MED-style predictor
- `6`: Adaptive 8x8 block predictor map

Predictors operate on transformed per-channel bytes. Residuals are encoded modulo 256.

The adaptive predictor mode stores a compact per-block predictor map at the start of the residual stream, then encodes the tile using the selected predictor for each 8x8 block. This keeps the outer tile container unchanged while allowing more local predictor selection within a tile.

## Entropy Backend IDs

Implemented in v1:

- `0`: Raw residual stream
- `1`: Zstd-compressed residual or special-transform payload stream
- `2`: Folded residual stream
- `3`: Zstd-compressed folded residual stream
- `4`: Zstd-compressed channel-separated residual stream
- `5`: Zstd-compressed folded channel-separated residual stream

Backends `2` and `3` are only valid for predictor residual streams. They apply an exact reversible residual folding map that brings small signed prediction errors closer together in byte space before optional compression. This is intended to help natural-image residual distributions remain more compressible without changing any decoded pixel values.

Backends `4` and `5` are also valid only for predictor residual streams. They split residual bytes into exact per-channel streams and compress those streams separately after preserving any adaptive predictor-map bytes. Backend `5` also applies the reversible residual folding map to each channel stream before compression.

## Decoding Process

Decoding proceeds as follows:

1. Read and validate the container header.
2. Read tile metadata and payloads.
3. Decode the tile payload with the selected entropy backend.
4. Reconstruct transformed bytes from predictor residuals.
5. Reverse the selected color transform.
6. Write the tile into the output RGBA image buffer.
7. Verify the SHA-256 hash of the full raw RGBA output.

## Verification Process

The `verify` command:

1. Loads the original input image.
2. Normalizes it to RGBA8.
3. Decodes the `.prsl` file.
4. Compares raw RGBA bytes exactly.
5. Prints original hash, decoded hash, and exact-match status.

This is strict decoded-pixel verification, not merely visual inspection.
