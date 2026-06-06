# Security Policy

Pressel is experimental research software. Thank you for helping keep the project safe and reliable.

## Supported Versions

Pressel is currently pre-1.0 software. Security fixes are only planned for the latest version on the `main` branch.

| Version | Supported |
|---|---|
| `main` | Yes |
| older commits / releases | No |

## Reporting a Vulnerability

Please do **not** report security vulnerabilities through public GitHub issues.

If you believe you found a security issue, please report it privately by using GitHub’s private vulnerability reporting/security advisory feature, if available, or by emailing `petar@stanchev.dev`.

When reporting a vulnerability, please include:

- A clear description of the issue
- Steps to reproduce it
- The affected command, for example `encode`, `decode`, `verify`, `bench`, or `compare`
- A minimal input file if possible
- Expected behavior
- Actual behavior
- Your operating system and Rust version

## What Counts as a Security Issue?

Examples of security-relevant issues include:

- Crafted `.prsl` files causing crashes, panics, or excessive memory allocation
- Malformed image files causing unsafe behavior
- Decompression bombs or extremely large allocations
- Path handling issues
- Unexpected file overwrite behavior
- Any issue that could affect users processing untrusted images

## Non-Security Bugs

Regular bugs, compression ratio issues, benchmark problems, documentation errors, and feature requests can be reported through normal GitHub issues.

## Disclosure

Please give the maintainer reasonable time to investigate and fix confirmed vulnerabilities before publishing details publicly.

Pressel is experimental and not intended for processing untrusted files in production environments yet.
