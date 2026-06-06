# Research Overview

## Summary

Pressel is an experimental lossless still-image codec. Its primary research goal is to explore whether small, reversible coding decisions selected adaptively on a per-tile basis can improve compression behavior while preserving exact decoded RGBA pixels.

## Research Question

Can adaptive per-tile combinations of reversible color transforms, spatial predictors, and entropy backends improve lossless image compression while preserving exact RGBA pixels?

## Strict Losslessness

Pressel defines correctness in terms of decoded-pixel identity, not visual similarity.

- Approximate reconstruction is not acceptable.
- Hidden RGB values in transparent pixels are part of the preserved signal.
- A successful decode must reproduce byte-identical RGBA output.

This differs from visually-lossless systems, where small reconstruction errors may be tolerated if they are hard to perceive.

## File-Byte Identity vs Decoded-Pixel Identity

Two distinct notions of identity matter in image compression research:

1. File-byte identity:
   The encoded file is byte-for-byte identical to another encoded file.
2. Decoded-pixel identity:
   The decoded image data is byte-for-byte identical in the target pixel representation.

Pressel targets decoded-pixel identity in RGBA8 form. It does not require that the output PNG bitstream match the input PNG bitstream, but it does require that decoding produce identical RGBA bytes.

In practical terms, this means Pressel behaves as a strictly lossless image codec, not as a byte-for-byte PNG archiver. Re-encoded PNG output may differ in file size or binary layout even when the decoded pixels are identical.

## Why Strict Losslessness Matters

Strict losslessness matters for:

- archival pipelines
- sprite and UI assets
- scientific or diagrammatic imagery
- alpha-sensitive compositing workflows
- benchmarking codec behavior without perceptual ambiguity

It is especially important when transparent pixels contain nonzero RGB values, because those values can affect downstream compositing or future re-encoding workflows even when they are visually hidden in a specific viewer.

## Influences

Pressel is inspired by several established codec families:

- PNG reversible filters
- JPEG-LS / LOCO-I predictive residual coding
- WebP lossless reversible transforms
- QOI-style pixel cache ideas
- JPEG XL-style adaptive prediction ideas

These are conceptual influences only. Pressel is not compatible with those formats and does not copy their source code.

## Research Framing

Pressel should be understood as a modular research codec. The project is not claiming state-of-the-art compression performance. Instead, it provides a controlled environment for experimenting with:

- tile-local reversible transforms
- predictor selection
- residual coding strategies
- benchmark-driven comparison across image classes

## Current Scope

The current `v0.2.0` research prototype focuses on:

- `.prsl` v1 container design
- strict RGBA verification
- a reversible tile pipeline
- benchmark generation through `bench.csv`

## Non-Goals

Current non-goals include:

- format compatibility with other codecs
- visually-lossless optimization
- unverified compression claims
- production standardization
