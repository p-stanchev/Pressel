# PRSL Format

## Overview

`.prsl` is the custom Pressel container format for strict lossless image coding.

- Extension: `.prsl`
- Magic bytes: `PRSL1`
- Pixel model in v1: RGBA8 only
- Tiling: 64x64 tiles in v1

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

Tiles are independent coding units. Edge tiles may be smaller than 64x64.

## Transform IDs

Implemented in v1:

- `0`: Raw RGBA
- `1`: Subtract-green
- `2`: Reversible YCoCg-R
- `3`: Alpha-plane separation
- `4`: Green average decorrelation
- `5`: Palette/index packed transform for suitable tiles

Subtract-green uses:

- `R' = R - G mod 256`
- `G' = G`
- `B' = B - G mod 256`
- `A' = A`

The inverse adds green back modulo 256.

Reversible YCoCg-R stores luma/chroma-like channel combinations using wrapping 8-bit arithmetic and reconstructs the original `R`, `G`, and `B` channels exactly.

Alpha-plane separation stores the alpha channel as a separate leading plane followed by grouped color planes. This can improve compression on images whose alpha structure differs from their color structure.

Green average decorrelation keeps `R` and `B` unchanged while storing `G' = G - floor((R + B) / 2) mod 256`.

The palette/index packed transform is only used on tiles where the exact colors fit into a compact palette representation inside the fixed-width transformed tile buffer.

## Predictor IDs

Implemented in v1:

- `0`: None
- `1`: Left
- `2`: Top
- `3`: Average
- `4`: Paeth
- `5`: JPEG-LS MED-style predictor

Predictors operate on transformed per-channel bytes. Residuals are encoded modulo 256.

## Entropy Backend IDs

Implemented in v1:

- `0`: Raw residual stream
- `1`: Zstd-compressed residual stream

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
