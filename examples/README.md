# Examples

Use this folder for demo inputs and outputs such as:

- `sample.png`
- `sample.prsl`
- `restored.png`

No copyrighted images are included here. For a fresh clone, generate a synthetic demo image with:

```bash
./target/release/pressel make-demo-image examples/sample.png
```

To generate a different reproducible sample, provide a seed:

```bash
./target/release/pressel make-demo-image examples/sample.png --seed 42
```

You can also place your own test images here if you have the right to use them.

Generated `.png`, `.prsl`, and restored output files in this folder are local demo artifacts and should not be committed unless you intentionally want them as fixtures.

If you want faster experiments on larger files, use the optional `--cores <N>` flag with `encode` or `bench`, for example:

```bash
./target/release/pressel encode examples/sample.png examples/sample.prsl --cores 4
```
