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

## What Is Pressel?

Pressel is a Rust project with two related goals:

1. Define and test a custom strictly lossless image format, `.prsl`.
2. Build a reproducible research environment for evaluating codec strategies and exact roundtrips.

Pressel is not PNG, JPEG-LS, WebP, QOI, or JPEG XL compatible. It borrows ideas from those codec families, but implements its own pipeline from scratch and does not copy source code from existing codecs.

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
pressel bench examples/
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
8. Run the benchmark with `./target/release/pressel bench examples/`.

Expected results:

- `verify` should print matching SHA-256 hashes and `exact match: true`.
- `decode` should produce `examples/restored.png`.
- `compare` should report `exact decoded RGBA match: true`.
- `bench` should write `bench.csv` in the repository root.

For the automated test suite, run:

```bash
cargo test
```

That covers strict RGBA roundtrips, transparent hidden RGB preservation, non-64-aligned dimensions, every transform, every predictor, both entropy backends, and the full encode/decode/verify flow.

## Commands

```text
pressel encode <input-image> <output.prsl>
pressel decode <input.prsl> <output.png>
pressel verify <input-image> <input.prsl>
pressel compare <first-image> <second-image>
pressel bench <folder>
pressel make-demo-image <output.png> [--seed <u64>]
```

## PRSL Format

`.prsl` v1 is a custom tile-based container for exact RGBA reconstruction.

- Extension: `.prsl`
- Magic bytes: `PRSL1`
- Channels: RGBA8 only in v1
- Tile size: 64x64 in v1
- Per-tile strategy selection
- SHA-256 hash of the original raw RGBA byte stream

Each tile independently tries multiple reversible transform, predictor, and entropy combinations, then stores the smallest exact result.

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

Example local results:

| Image | PNG size | PRSL size | Exact RGBA match |
|---|---:|---:|---|
| synthetic demo (`--seed 42`) | 58,770 bytes | 6,745 bytes | true |
| rural photo | 3,891,380 bytes | 2,908,487 bytes | true |

## Research Notes

Pressel is designed as a research codec, not just a file converter.

- [Research Overview](docs/RESEARCH.md)
- [PRSL Format](docs/FORMAT.md)
- [Compression Pipeline](docs/COMPRESSION-PIPELINE.md)
- [Benchmarking](docs/BENCHMARKING.md)

## Version Goal

This project is currently positioned as `v0.1.0`: a working research prototype with `encode`, `decode`, `verify`, `compare`, `bench`, and demo-image commands, documentation, and strict roundtrip tests.

## Roadmap

- Golomb-Rice residual coding
- rANS entropy coding
- adaptive block predictor maps
- QOI-style pixel cache mode
- palette/index transform for `.prsl`
- alpha-plane separation
- reversible YCoCg-R transform
- JPEG XL-style weighted predictor
- per-tile image classifier
- multithreaded tile encoding

## License

Pressel is released under the MIT License.

- Copyright (c) 2026 Petar Stanchev
- See [LICENSE](LICENSE)
