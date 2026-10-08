---
name: release-build
description: Build, size-check and verify the static musl release binary of chromaterm-rs. Use when asked to release, package, check binary size, or verify portability.
---

# Release build

Releases are produced by CI (GitLab origin + GitHub mirror) from tags. Both
pipelines call the same scripts in `ci/`.

1. Quality gate: `make check` (fmt, clippy -D warnings, tests for both feature sets).
2. Local release binaries: `make release-all` (or `ci/build-release.sh <target> dist`
   for one target). Targets: `{x86_64,aarch64}-unknown-linux-{gnu,musl}`.
   - musl: plain cargo, static. The script fails if not statically linked.
   - gnu: cargo-zigbuild against glibc 2.28 (pinned zig/cargo-zigbuild get
     installed into a venv). The script fails if a newer glibc symbol is needed.
   - On the host architecture the script also runs a smoke test.
3. `ci/checksums.sh dist` writes `SHA256SUMS`.
4. Size: report `ls -l dist/`. Regressions above ~10% need a justification.
5. Cut a release: bump `version` in Cargo.toml, commit `chore(release): vX.Y.Z`,
   `git tag -a vX.Y.Z -m vX.Y.Z && git push origin main --follow-tags`.
   `ci/check-version.sh` fails the pipeline if tag and Cargo.toml disagree.
   Release notes: `ci/release-notes.sh vX.Y.Z` (from Conventional Commits).
6. glibc vs musl speed numbers in README must be re-measured if the engine or
   allocator strategy changes (best of 5, 50k-line corpus).
