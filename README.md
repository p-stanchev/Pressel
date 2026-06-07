<div align="center">
  <h1>Pressel</h1>
  <p><strong>Experimental strictly lossless image codec and research platform written in Rust.</strong></p>
  <p>
    <img alt="Rust" src="https://img.shields.io/badge/language-Rust-000000?logo=rust">
    <img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-green">
    <img alt="Status: Experimental" src="https://img.shields.io/badge/status-experimental-orange">
    <img alt="Strict Lossless" src="https://img.shields.io/badge/roundtrip-byte--identical%20RGBA-blue">
  </p>
  <p>
    <code>lossless-compression</code>
    <code>image-codec</code>
    <code>rust</code>
    <code>research-codec</code>
    <code>predictive-coding</code>
    <code>entropy-coding</code>
  </p>
</div>

Pressel is an experimental research codec focused on strict decoded-pixel correctness. It explores tile-adaptive reversible transforms, predictive residual coding, and entropy backends for lossless still-image compression.

Pressel is strictly lossless with respect to decoded RGBA pixel data, not original file bytes.

Pressel preserves byte-identical decoded RGBA pixels. It does not preserve the original PNG bitstream, metadata layout, compression stream, chunk ordering, or file hash. In other words, Pressel is a lossless image codec, not a PNG archiver.

Pressel can optionally preserve PNG metadata, raw ancillary chunks, or the full original PNG source file inside `.prsl`. Exact original PNG file recovery is only available when `--preserve-source-file` is used.

## What Is Pressel?

Pressel is a Rust project with two related goals:

1. Define and test a custom strictly lossless image format, `.prsl`.
2. Build a reproducible research environment for evaluating codec strategies and exact roundtrips.

Pressel is not PNG, JPEG-LS, WebP, QOI, or JPEG XL compatible. It borrows ideas from those codec families, but implements its own pipeline from scratch and does not copy source code from existing codecs.

The current `v0.6.0` prototype prioritizes exactness and compression ratio over encode speed. For larger images, multi-core encoding is available through `--cores <N>`.

## Strict Lossless Guarantee

Pressel is strictly lossless with respect to decoded RGBA pixel data.

- Decoding must reproduce byte-identical RGBA pixels.
- Hidden RGB values inside transparent pixels must be preserved.
- No visually-lossless shortcuts are allowed.
- No approximate reconstruction is accepted.

This project treats decoded RGBA identity as the core correctness rule, rather than original PNG file-byte identity.

Pressel preserves the decoded image signal, not the original PNG file bitstream. A decoded image must return byte-identical RGBA pixels, but a regenerated PNG file may differ in file size, chunk layout, metadata, or compression structure.

## Experimental Status

Pressel is experimental research software, not a production image standard. The codec and documentation are intended to support exploration, benchmarking, and demo workflows. Claims about compression effectiveness should be validated with benchmarks rather than assumed.

## Demo

Build the release binary:

```bash
cargo build --release
```

If you do not already have a test image, generate a synthetic sample first:

```bash
./target/release/pressel make-demo-image examples/sample.png
```

To generate a different but reproducible sample, pass a seed:

```bash
./target/release/pressel make-demo-image examples/sample.png --seed 42
```

Demo flow:

```bash
pressel encode examples/sample.png examples/sample.prsl
pressel decode examples/sample.prsl examples/restored.png
pressel verify examples/sample.png examples/sample.prsl
pressel compare examples/sample.png examples/restored.png
pressel bench examples/ --cores 4
```

If `pressel` is not on your `PATH`, run the same commands through the built binary directly:

```bash
./target/release/pressel encode examples/sample.png examples/sample.prsl
./target/release/pressel decode examples/sample.prsl examples/restored.png
./target/release/pressel verify examples/sample.png examples/sample.prsl
./target/release/pressel compare examples/sample.png examples/restored.png
./target/release/pressel bench examples/
```

The `examples/` folder is included for local demo assets and contains guidance on adding your own non-copyrighted test images.

## How To Test

Use this sequence on a fresh clone:

1. Build the release binary with `cargo build --release`.
2. Generate a demo image with `./target/release/pressel make-demo-image examples/sample.png`.
3. If you want a different repeatable pattern, use a seed such as `./target/release/pressel make-demo-image examples/sample.png --seed 42`.
4. Encode it with `./target/release/pressel encode examples/sample.png examples/sample.prsl`.
5. Decode it with `./target/release/pressel decode examples/sample.prsl examples/restored.png`.
6. Verify strict equality with `./target/release/pressel verify examples/sample.png examples/sample.prsl`.
7. Compare the source and restored images with `./target/release/pressel compare examples/sample.png examples/restored.png`.
8. Run the benchmark with `./target/release/pressel bench examples/ --cores 4`.

Both `encode` and `bench` accept an optional `--cores <N>` flag. If omitted, Pressel uses `1` core by default.

Expected results:

- `verify` should print matching SHA-256 hashes and `exact match: true`.
- `decode` should produce `examples/restored.png`.
- `compare` should report `exact decoded RGBA match: true`.
- `bench` should write `bench.csv` in the repository root.

For the automated test suite, run:

```bash
cargo test
```

That covers strict RGBA roundtrips, transparent hidden RGB preservation, non-64-aligned dimensions, every transform, every predictor, all entropy backends, and the full encode/decode/verify flow.

## Commands

```text
pressel encode <input-image> <output.prsl> [--cores <usize>] [--preserve-png-metadata] [--preserve-png-chunks] [--preserve-source-file]
pressel decode <input.prsl> [<output.png>] [--export-png <path>] [--extract-source-file <path>]
pressel verify <input-image> <input.prsl>
pressel compare <first-image> <second-image>
pressel bench <folder> [--cores <usize>]
pressel make-demo-image <output.png> [--seed <u64>]
```

## Optional PNG Preservation

By default, Pressel stores only what it needs for exact decoded RGBA recovery.

- Default mode: exact RGBA only
- `--preserve-png-metadata`: store a curated set of useful PNG metadata chunks such as `gAMA`, `cHRM`, `sRGB`, `iCCP`, `pHYs`, `tIME`, `tEXt`, `zTXt`, `iTXt`, and `eXIf`
- `--preserve-png-chunks`: store all ancillary PNG chunks with placement/order data
- `--preserve-source-file`: store the original PNG file byte-for-byte for exact extraction later

`--preserve-png-chunks` subsumes `--preserve-png-metadata`. If both are provided, Pressel stores ancillary chunks once in chunk mode and does not duplicate metadata in a second tag.

Examples:

```bash
pressel encode image.png image.prsl --preserve-png-metadata
pressel encode image.png image.prsl --preserve-png-chunks
pressel encode image.png image.prsl --preserve-source-file
pressel decode image.prsl --extract-source-file recovered.png
pressel decode image.prsl --export-png restored-with-safe-chunks.png
```

When exporting PNG, Pressel regenerates `IHDR`/`IDAT`/`IEND` from the decoded image and only reattaches preserved ancillary chunks when their placement is valid and safe. Unsafe-to-copy ancillary chunks may be skipped with a warning.

## PRSL Format

`.prsl` v1 is a custom tile-based container for exact RGBA reconstruction.

- Extension: `.prsl`
- Magic bytes: `PRSL1`
- Channels: RGBA8 only in v1
- Tile size: stored per file, chosen from an exact whole-image search
- Per-tile strategy selection
- SHA-256 hash of the original raw RGBA byte stream
- Optional tagged sections for preserved PNG metadata, ancillary chunks, or the full original source file

Each tile independently tries multiple reversible transform, predictor, and entropy combinations, then stores the smallest exact result. The current search space includes fixed-width bytewise transforms, an exact structured-plane transform, adaptive predictor maps, and raw, Zstd, folded-residual, and channel-split residual payload storage. The encoder also searches a small set of whole-image tile sizes and can parallelize tile encoding when `--cores` is greater than `1`.

When decoding back to PNG, Pressel reconstructs the original RGBA pixels exactly, but it does not attempt to recreate the original PNG file bytes exactly.

More detail:

- [Research Overview](docs/RESEARCH.md)
- [PRSL Format](docs/FORMAT.md)
- [Compression Pipeline](docs/COMPRESSION-PIPELINE.md)
- [Benchmarking](docs/BENCHMARKING.md)

## Benchmarking

`pressel bench <folder>` recursively scans a folder, encodes supported images to `.prsl`, decodes them, verifies exact RGBA identity, and writes `bench.csv`.

Reported metrics include:

- filename
- width and height
- original file size
- `.prsl` size
- compression ratio
- encode time
- decode time
- selected transform counts
- selected predictor counts
- selected entropy backend counts
- verification result

`encode` and `bench` default to `1` core. Use `--cores <N>` when you want faster encode-time experiments on larger images.

Example local results:

Core compression progression:

| Image | PNG size | Gen 1 PRSL | Gen 2 PRSL | Gen 3 PRSL | Gen 4 PRSL | Gen 5 PRSL | Exact RGBA match |
|---|---:|---:|---:|---:|---:|---:|---|
| synthetic demo (`--seed 42`) | 58,770 bytes | 7,633 bytes | 6,745 bytes | 6,745 bytes | 1,895 bytes | 1,895 bytes | true |
| rural photo | 3,891,380 bytes | 2,908,487 bytes | 2,908,487 bytes | 2,907,368 bytes | 2,840,040 bytes | 2,224,193 bytes | true |

Generation 6 preservation results:

| Image | PNG size | Gen 6 default | Gen 6 + metadata | Gen 6 + chunks | Gen 6 + source file | Exact RGBA match |
|---|---:|---:|---:|---:|---:|---|
| synthetic demo (`--seed 42`) | 58,770 bytes | 1,895 bytes | 1,895 bytes | 1,895 bytes | 60,675 bytes | true |
| rural photo | 3,891,380 bytes | 2,224,772 bytes | 2,224,809 bytes | 2,224,809 bytes | 6,116,162 bytes | true |

Generation 6 is intended to show the size impact of optional PNG preservation modes:

- `Gen 6 default`: exact RGBA only
- `Gen 6 + metadata`: exact RGBA plus curated PNG metadata chunks
- `Gen 6 + chunks`: exact RGBA plus preserved ancillary PNG chunks
- `Gen 6 + source file`: exact RGBA plus the original PNG stored byte-for-byte

The generated synthetic sample does not contain meaningful preserved PNG metadata or ancillary chunks, so its `Gen 6 default`, `Gen 6 + metadata`, and `Gen 6 + chunks` sizes are identical. The rural photo is a better example of preservation overhead on a real PNG input.

To measure those four generation 6 variants on the same PNG:

```bash
pressel encode examples/rural.png rural-default.prsl
pressel encode examples/rural.png rural-meta.prsl --preserve-png-metadata
pressel encode examples/rural.png rural-chunks.prsl --preserve-png-chunks
pressel encode examples/rural.png rural-source.prsl --preserve-source-file
```

Then inspect the resulting sizes:

```bash
ls -l rural-default.prsl rural-meta.prsl rural-chunks.prsl rural-source.prsl
```

For the rural photo example, `verify` reported matching decoded RGBA hashes:

```text
original SHA-256: 2d4ad8c726cb2a9ef105e683544b07234c669dc71d976ec4a7767c925bfce05a
decoded SHA-256: 2d4ad8c726cb2a9ef105e683544b07234c669dc71d976ec4a7767c925bfce05a
exact match: true
```

## Research Notes

Pressel is designed as a research codec, not just a file converter.

- [Research Overview](docs/RESEARCH.md)
- [PRSL Format](docs/FORMAT.md)
- [Compression Pipeline](docs/COMPRESSION-PIPELINE.md)
- [Benchmarking](docs/BENCHMARKING.md)

## Version Goal

This project is currently positioned as `v0.6.0`: a more capable research prototype with `encode`, `decode`, `verify`, `compare`, `bench`, and demo-image commands, documentation, strict roundtrip tests, CI, safer decode validation, an expanded reversible transform set, adaptive tile-size search, structured exact plane modeling, residual folding experiments, channel-separated residual coding, photo-oriented predictor experiments, aggressive exact compression experiments over raw, folded, and Zstd-backed residual payloads, and optional PNG metadata/chunk/source-file preservation.

## Roadmap

- rANS entropy coding
- QOI-style pixel cache mode
- JPEG XL-style weighted predictor
- per-tile image classifier

## License

Pressel is released under the MIT License.

- Copyright (c) 2026 Petar Stanchev
- See [LICENSE](LICENSE)
