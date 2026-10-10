# AGENTS.md — guide for AI agents working on chromaterm-rs

This is the canonical, tool-agnostic briefing for AI coding agents (Claude Code,
Codex, Copilot, Cursor, Aider, …). `CLAUDE.md` imports this file. Read it fully
before changing code. Read `docs/PLAN.md` for design rationale and the roadmap,
`docs/CICD-SETUP.md` for CI/CD, and `docs/COLOR-SYSTEM.md` before touching
colors. Documentation lives in `docs/`. Only files that conventionally sit at
the root stay there (README, AGENTS, CLAUDE, LICENSE).

## What this project is
`ct` highlights terminal output with regex rules, in a single static
musl binary. It is a Rust rewrite of Python [ChromaTerm](https://github.com/hSaria/ChromaTerm).
It runs either as a filter (`cmd | ct`) or as a PTY wrapper (`ct ssh host`).

## Commands you will need
```sh
cargo build                                   # debug build
cargo test                                    # all unit + integration tests
cargo fmt --check                             # formatting gate
cargo clippy --all-targets -- -D warnings     # lint gate (must be clean)
cargo bench                                   # criterion benchmarks
make check                                    # fmt + clippy + tests, both feature sets
make link                                     # native release build → ~/.local/bin/ct
make release-all                              # 4 release binaries (gnu/musl × x86_64/aarch64)
./target/debug/ct config check                # validate effective config
printf 'GET http://x 10.0.0.1 ERROR\n' | ./target/debug/ct   # smoke test
```
The musl target needs `rustup target add x86_64-unknown-linux-musl`. No C
toolchain is needed: the dependency tree is pure Rust, and **must stay that way**.

## Repository map
The engine, config and color code are the library crate `crates/chromaterm-core` (no terminal
I/O; embeddable). The root crate is the `ct` binary and re-exports the core modules under
their old paths (`chromaterm::engine`, …).

| Path | Responsibility |
|---|---|
| `crates/chromaterm-core/src/ansi.rs` | Escape-sequence tokenizer, SGR parsing, terminal `Attrs` state |
| `crates/chromaterm-core/src/color.rs` | `Color`, `Style`, SGR emission, RGB→xterm-256 mapping, `ColorMode` |
| `crates/chromaterm-core/src/config/mod.rs` | Serde schema, layering (built-in → file → inline), discovery, validation |
| `crates/chromaterm-core/src/config/resolve.rs` | Palette/theme resolution, color-spec parser, `${pattern}` interpolation, rule compilation |
| `crates/chromaterm-core/src/config/legacy.rs` | Python ChromaTerm YAML importer (feature `legacy-yaml`) |
| `crates/chromaterm-core/src/engine/mod.rs` | `Highlighter`: matching, exclusivity, span rendering |
| `crates/chromaterm-core/src/engine/matcher.rs` | `Matcher::{Fast, Fancy}` — regex vs fancy-regex with pre-filter |
| `src/stream.rs` | Line splitting, partial-line hold-back, timeout flush |
| `src/io.rs` / `src/pty.rs` | stdin loop / PTY child runner (both handle the reload signal) |
| `src/signals.rs` | Signal flags + self-pipe (reload, winch, child, forwarded signals) |
| `src/instances.rs` | Instance registry in `$XDG_RUNTIME_DIR/chromaterm`, `ct --reload` |
| `src/appearance.rs` | `auto` theme: OSC 11/10 + DA1 terminal query, `$COLORFGBG` (classification is in `crates/chromaterm-core/src/appearance.rs`) |
| `crates/chromaterm-core/src/config/export.rs` | Python ChromaTerm YAML exporter (`ct config export -F yaml`) |
| `ci/*.sh` | Build/release scripts shared by `.gitlab-ci.yml` and `.github/workflows/release.yml` |
| `src/cli.rs` | clap CLI + subcommands |
| `crates/chromaterm-core/assets/builtin.toml` | **Built-in palette, themes, named patterns, default rules** |
| `crates/chromaterm-core/assets/template.toml` | Commented template written by `ct config init` |
| `tests/` | Integration tests (`assert_cmd`) |

## Invariants — do not break
1. **Pure Rust deps only.** No `-sys` crates or C builds. Static musl must build with plain `cargo build --target x86_64-unknown-linux-musl`.
2. **Hot path is allocation-free in steady state.** `Highlighter::highlight_line` reuses internal buffers. Don't add per-line `Vec`/`String` allocations.
3. **Never emit `ESC[0m` to end a highlight.** Restore the program's own state per attribute (see `ansi::Attrs` + `engine::render`).
4. **Never insert bytes inside an escape sequence or a UTF-8 code point.** The tokenizer and `stream` hold back incomplete sequences.
5. **Rule semantics** (see docs/PLAN.md §2.3): in-order evaluation; exclusive matches claim ranges; overlapping non-exclusive highlights → later rule wins per attribute.
6. **Config schema is strict** (`deny_unknown_fields`). New keys must be added to the schema, `crates/chromaterm-core/assets/template.toml`, README config reference, and tests.
7. **Built-in patterns prefer the linear-time `regex` syntax** (no look-around). Use `fancy-regex` features only when unavoidable.
8. **Errors are aggregated and human-readable**: point to the rule index/description and the field.
9. **ASCII classes by default** (`settings.unicode = false`). This only changes what
   `\w \d \s \b` and `(?i)` mean in regexes. UTF-8 text always passes through
   and literal non-ASCII characters always match (README → "Unicode matching").
   Never enable Unicode word boundaries in built-in patterns: they knock the regex
   crate off its fast DFA (≈4.6× slower matching, ≈10× slower startup measured).
10. **Measure before/after for perf changes**: `cargo bench --bench highlight` and
    `ct -b` on a corpus. Note that musl and glibc behave differently (docs/PLAN.md §2.5).

## How to …
- **Add a built-in named pattern or default rule** → see `.claude/skills/add-builtin-pattern/SKILL.md`.
- **Add a config key** → schema struct in `config/mod.rs` → resolution in `config/resolve.rs` → template + README → tests in `tests/config.rs`.
- **CI runs only on `vX.Y.Z` tags.** Nothing checks branch pushes, so run `make check`
  yourself before every commit you would want released.
- **Change CI/CD** → edit `ci/*.sh` (shared), and only orchestration in `.gitlab-ci.yml` /
  `.github/workflows/release.yml`. Keep both pipelines equivalent. Run `shellcheck ci/*.sh`.
- **Add a CLI flag/subcommand** → `cli.rs` (clap derive) → integration test in `tests/cli.rs` → README usage section.
- **Cut a release / check binary size** → see `.claude/skills/release-build/SKILL.md`.
- **Change built-in colors** → follow `docs/COLOR-SYSTEM.md`. Derive new values with
  its generator (§9), never by eye, and keep the generator's tables and
  `crates/chromaterm-core/assets/builtin.toml` identical (it must reproduce every shipped value). The
  `builtin_themes_meet_contrast_targets` test enforces the floors.

## Gotchas
- **Never signal processes found by name.** SIGUSR1 kills programs without a
  handler. Reload must only target processes in the instance registry, with a
  matching start time.
- The `--no-default-features` build has no YAML *loader* (export still works).
  Tests that load YAML must be gated with `cfg!(feature = "legacy-yaml")`.
- **Terminal queries race with other readers of the keyboard.** Only query in
  the places `cli.rs` allows: before spawning the child (program mode), after
  the first input (filter mode), never mid-session or in reloads. Keep the
  guards in `appearance.rs` (stdout is a tty, foreground process group,
  `TERM`). `tests/common::run_in_terminal` is a fake terminal emulator for
  end-to-end tests of this.
- Tests must remove `COLORFGBG` (as `tests/common` does), or a developer's
  light terminal changes expected colors.
- Integration tests must isolate `HOME`/`XDG_CONFIG_HOME` (see `tests/common`).
  Otherwise the developer's own `~/.chromaterm.yml` gets loaded.
- `[profile.dev] opt-level = 1` is deliberate: regex generics are monomorphized
  into this crate, and unoptimized test runs are ~100× slower.
- Program mode must reap the child **before** closing the PTY master. Otherwise
  a child that already closed its fds but hasn't exited gets SIGHUP.

## Testing expectations
- Every bug fix comes with a regression test.
- Engine changes: add a case to `engine` unit tests asserting exact output bytes.
- Config changes: valid and invalid examples (error messages are asserted with `contains`).
- If `sample-config-file-for-current-python-implementation.yaml` exists locally,
  `tests/legacy.rs` imports and compiles it. Keep that passing.

## Style
- `rustfmt` defaults. Clippy clean with `-D warnings`.
- Doc-comment public items. Comment the *why*, not the *what*.
- Prefer small, pure functions that are easy to unit-test. I/O stays at the edges (`io.rs`, `pty.rs`, `cli.rs`).

## Git
- Conventional Commits, always: `type(scope): summary` with types
  `feat|fix|perf|refactor|test|docs|build|ci|chore|style|revert`.
- One logical change per commit. Run the quality gate before committing.
