---
name: release-build
description: Build, size-check and verify the static musl release binary of chromaterm-rs. Use when asked to release, package, check binary size, or verify portability.
---

# Release build

1. Quality gate: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`.
2. Build: `make release` (≙ `cargo build --profile release --target x86_64-unknown-linux-musl`).
   The release profile (Cargo.toml) uses LTO=fat, codegen-units=1, panic=abort, strip.
3. Verify static linkage: `file dist/ct-*` must say *statically linked*;
   `ldd dist/ct-*` must say *not a dynamic executable*.
4. Size: `ls -l dist/` — report the size. Regressions above ~10% need a justification
   (`cargo bloat --release --target x86_64-unknown-linux-musl --crates` if installed).
5. Smoke test: `printf 'ERROR 10.0.0.1 https://x.io\n' | dist/ct-*` and `dist/ct-* config check`.
6. Optional size-only build without the YAML importer:
   `cargo build --profile release --target x86_64-unknown-linux-musl --no-default-features`.
