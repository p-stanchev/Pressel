# Benchmarking

## What `pressel bench <folder>` Does

The benchmark command recursively scans a folder for decodable images. For each image, it:

1. loads and normalizes the image to RGBA8
2. encodes it as `.prsl`
3. decodes the `.prsl`
4. verifies exact RGBA equality
5. writes one CSV row to `bench.csv`

`pressel bench` accepts an optional `--cores <N>` flag. If omitted, it runs with `1` core.

The command benchmarks source image files that the `image` crate can decode. It does not treat existing `.prsl` files in the folder as benchmark inputs.

## Recorded Metrics

The current benchmark output includes:

- filename
- width
- height
- original file size
- `.prsl` size
- compression ratio
- encode time
- decode time
- selected transform counts
- selected predictor counts
- selected entropy backend counts
- verification result

## How to Read `bench.csv`

Useful interpretations include:

- smaller `.prsl` size indicates better compression for this codec version
- encode and decode time show the computational cost of strategy search
- `--cores` changes wall-clock encode time, not the exact decoded result
- transform/predictor/backend counts show which coding decisions were selected
- verification must remain `true`; any false result indicates a correctness failure

When benchmarking preservation-aware `.prsl` files, it helps to separate:

- core compression progression across codec generations
- preservation overhead from optional PNG metadata, ancillary chunk, or source-file storage

Those preservation modes should be reported explicitly rather than mixed into the main generation-to-generation compression table, because they measure completeness tradeoffs rather than pure image-signal compression.

## Future Cross-Codec Comparisons

Future benchmark studies may compare Pressel against:

- PNG
- ZopfliPNG
- WebP lossless
- JPEG-LS
- JPEG XL lossless
- QOI

A useful comparison table should keep at least these columns aligned:

- input file name
- original PNG size
- ZopfliPNG output size
- WebP lossless output size
- JPEG XL lossless output size
- Pressel `.prsl` size
- exactness notes

Pressel should not be framed as “better PNG” unless those measurements are actually present and reproducible.

Such comparisons should be explicit about:

- input corpus
- encoder settings
- measured output size
- encode/decode time
- memory behavior
- exactness guarantees

No claim that Pressel outperforms those codecs should be made without measured evidence.

## Recommended Benchmark Corpus

A meaningful corpus should include:

- natural photos
- screenshots
- pixel art
- diagrams
- transparent images
- random/noisy images
- gradients
- small images

Different image classes stress different parts of the pipeline. For example, gradients can favor predictive coding, while noisy images expose the limits of spatial predictors and make entropy modeling more important.
