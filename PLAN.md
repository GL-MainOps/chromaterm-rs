# chromaterm-rs — Project Plan

A Rust rewrite of [ChromaTerm](https://github.com/hSaria/ChromaTerm) (`ct`), the
regex-driven terminal output highlighter. It ships as a single static, portable
musl binary.

> Status legend: ✅ done · 🚧 in progress · 🔜 planned · 💡 idea / open question

---

## 1. Goals

| Goal | How |
|---|---|
| **Fast** | Linear-time `regex` engine first, `fancy-regex` only when a pattern needs look-around/back-references (gated by a cheap linear-time pre-filter). Zero-allocation hot path (reused buffers), fast path for lines with no escape sequences, `poll(2)`-driven I/O. |
| **Small & portable** | Pure Rust dependency tree (no C, no OpenSSL, no PCRE). Built for `x86_64-unknown-linux-musl` / `aarch64-unknown-linux-musl`, statically linked, `opt-level=3`, LTO, `codegen-units=1`, `panic=abort`, stripped. |
| **Secure** | No ReDoS on the default engine (linear-time automata). Back-tracking engine has a hard step limit. Bounded buffers (no unbounded growth on endless lines). YAML importer refuses alias *collections* (no billion-laughs). Strict config schema (`deny_unknown_fields`). No `unsafe` in our code except syscall-level PTY glue via `rustix`. |
| **Modern UX** | TOML config with comments and raw multi-line regex strings; JSON or TOML inline config on the CLI; env-var discovery; themes; named colors and named patterns; generator for a fully commented config; validation with precise, aggregated errors. |
| **Extensible** | Layered config model (built-in → file → inline), schema `version` field, module boundaries matching responsibilities (`ansi`, `color`, `config`, `engine`, `stream`, `pty`, `cli`). |
| **Compatible** | Same rule model as Python ChromaTerm (`regex`, `color`, `exclusive`, group colors, `palette`, `f.`/`b.` color syntax). `ct config import` converts legacy YAML configs. |

## 2. Decisions

### 2.1 Language: Rust ✅
Rust gives static musl binaries with no runtime, a linear-time regex engine
(`regex`, the same family as RE2/ripgrep), memory safety without GC pauses, and
first-class PTY/termios access through `rustix`. The alternatives:

- **Go**: also produces static binaries, but `regexp` (RE2) is slower than Rust
  `regex` and has no look-around fallback. Binaries are usually larger (~2× or more).
- **Zig/C**: smallest binaries, but you need PCRE2/hand-written regex and
  manual memory safety. That's too much risk for a tool that processes untrusted
  terminal output.
- **Python (status quo)**: interpreter startup, per-match Python overhead, and
  back-tracking `re` that is open to ReDoS.

### 2.2 Config language: TOML (primary) + JSON (inline / alt) ✅
Multi-line support **is** needed. Real-world rules are long, and verbose-mode
regexes `(?x)` with comments read far better across lines (see the URL rule in
the legacy sample).

| Need | YAML | JSON | **TOML** |
|---|---|---|---|
| Comments | ✅ | ❌ | ✅ |
| Regex without escaping backslashes | ⚠️ (block scalars only) | ❌ (`\\d`) | ✅ literal strings `'\d+'` / `'''…'''` |
| Multi-line strings | ✅ | ❌ | ✅ |
| No implicit typing traps (`no` → false, `1.10` → 1.1) | ❌ | ✅ | ✅ |
| Well-maintained Rust serde support | ⚠️ (`serde_yaml` archived) | ✅ | ✅ |
| One-line inline on the CLI | ⚠️ | ✅ | ✅ (TOML 1.1 inline tables) |

So TOML is the config file format. JSON is accepted anywhere a config is accepted
(`*.json` files, `-i '{…}'`), because it is the most natural one-liner. YAML is
supported only as **import** of legacy ChromaTerm configs (`ct config import`,
cargo feature `legacy-yaml`).

### 2.3 Rule semantics (compatible with ChromaTerm) ✅
- Rules are evaluated in order on each line, with escape sequences stripped.
- A match is dropped if it overlaps a range already claimed by an **exclusive** match.
- Kept matches of an exclusive rule claim their range.
- Where non-exclusive highlights overlap, the **later** rule wins per attribute
  (fg, bg, and each style flag), so later rules "add styling over" earlier ones.
- Existing colors emitted by the program are preserved. When a highlight ends,
  the program's own color state is restored per attribute (not a blanket `ESC[0m`).

### 2.4 Unicode classes: ASCII by default ✅
`\w \d \s \b` are ASCII-only unless `settings.unicode = true` (or per rule).
Unicode word boundaries force the regex crate off its lazy DFA on any line
with non-ASCII bytes. We measured 3–4× slower matching on such lines, 8× slower
startup and ~7× the memory for configs with `\w{1,63}`-style repetitions.
Terminal tokens (IPs, hashes, levels) are ASCII, so this is the right default.
Imported legacy configs say so in a header comment.

### 2.5 Allocator & threads on musl ✅
musl's malloc serializes threads on one lock and churns `mmap` for large
blocks. Parallel rule compilation is therefore **disabled on musl**: it made
startup 2× slower there and helps on glibc. A pure-Rust `dlmalloc` was tried
and was slower (also a global lock). The hot path is allocation-free, so the
runtime gap vs glibc is small. Only compile-heavy startup is affected.

### 2.6 Line model ✅
Input is split on `\n`, `\r\n`, `\r`. A partial trailing line is held for a
short **read timeout** (default 2 ms) in case more data completes it. After that
it is flushed. Incomplete escape sequences / UTF-8 sequences are never split.
Lines longer than 64 KiB are flushed in chunks (bounded memory).

## 3. Architecture

```
src/
  main.rs            entry point → cli::run()
  lib.rs             library facade (used by tests & benches)
  cli.rs             clap definitions + subcommand dispatch
  ansi.rs            escape-sequence tokenizer + SGR parser (terminal state model)
  color.rs           Color / Style / SGR emission / truecolor→256 down-sampling
  config/
    mod.rs           schema (serde), layering, discovery, validation
    resolve.rs       palette/themes → colors, color-spec parser, pattern interpolation
    legacy.rs        YAML (Python ChromaTerm) importer [feature legacy-yaml]
  engine/
    mod.rs           Highlighter: compiled rules → spans → rendered output
    matcher.rs       Fast(regex) | Fancy(fancy-regex + linear pre-filter)
  stream.rs          line splitting, partial-line hold-back, flush policy
  io.rs              stdin loop (poll + timeout)
  pty.rs             run a program under a PTY (raw mode, SIGWINCH, exit code)
assets/
  builtin.toml       built-in palette, themes, named patterns, default rules
  template.toml      `ct config init` template (commented, with examples)
tests/               integration tests (CLI, config, highlighting, legacy import)
benches/             criterion benchmarks
```

Data flow: `bytes → stream (lines) → ansi::tokenize → engine (matches → spans)
→ render (SGR diffs) → stdout`.

## 4. Milestones

### M0 — Foundations ✅
- [x] `.gitignore`, conventional commits, plan, AGENTS.md / CLAUDE.md / skills
- [x] Cargo project, release profile tuned for size+speed, musl target config

### M1 — Core engine ✅
- [x] ANSI tokenizer (CSI, OSC, DCS/SOS/PM/APC, 2-byte ESC), incomplete-sequence detection
- [x] SGR state model (fg/bg/bold/dim/italic/underline/blink/invert/strike; 16/256/RGB; `:` sub-params)
- [x] Rule matching with exclusivity, group colors, named groups
- [x] Renderer with minimal SGR diffs and per-attribute restoration
- [x] Hybrid matcher (regex → fancy-regex fallback with linear pre-filter)

### M2 — Config ✅
- [x] TOML/JSON schema, `version`, strict unknown-field rejection
- [x] Discovery: `--config`, `$CHROMATERM_CONFIG`, XDG paths, `~/.chromaterm.toml`, `/etc`
- [x] Inline config (`-i`, JSON or TOML, repeatable, layered)
- [x] Built-in palette (dark/light themes), named colors, named patterns, `${pattern}` interpolation
- [x] Aggregated validation errors; `ct config check`
- [x] `ct config init [--full]` commented generator; `ct config show`; `ct config path`
- [x] `ct config import` legacy YAML → TOML

### M3 — Runtime ✅
- [x] stdin mode with poll-based read timeout
- [x] PTY mode: raw mode, window-size propagation, signal forwarding, exit status passthrough
- [x] `--benchmark` per-rule timing report
- [x] `ct patterns`, `ct colors` (with live swatches), `ct completions <shell>`

### M4 — Quality ✅
- [x] Unit tests per module, integration tests via `assert_cmd`
- [x] Legacy sample config import + compile test (when sample is present)
- [x] Criterion benchmarks
- [x] CI workflow (fmt, clippy -D warnings, test, musl release build, size report)

### M5 — Performance ✅
- [x] `RegexSet` pre-pass skips rules that cannot match a line (1.8× on a 141-rule config)
- [x] Parallel rule compilation (glibc). Serial on musl, see §2.5
- [x] ASCII-class default (§2.4)
- [x] Static `aarch64-unknown-linux-musl` build via bundled `rust-lld` (built in CI; not run-tested locally)

Measured (10k lines, 860 KB, static musl binary vs Python ChromaTerm 0.10.7):

| Workload | Python | ct |
|---|---|---|
| default rules | 0.92 s | 0.09 s |
| 141-rule real config | 4.20 s | 0.40 s |
| startup (defaults) | 0.07 s / 18 MB | 0.01 s / 5 MB |

### M6 — Next 🔜
- [ ] 🔜 `--reload` / `SIGUSR1` config reload of running instances (Python ChromaTerm parity: `ct -r`)
- [ ] 🔜 Fuzzing targets (`cargo fuzz`) for the tokenizer, renderer and config parser
- [ ] 🔜 macOS release artifacts (code is portable via `rustix`; untested)
- [ ] 🔜 Run aarch64 smoke tests under QEMU in CI
- [ ] 🔜 Optional faster allocator for musl (e.g. mimalloc behind a feature; needs a musl C compiler)
- [ ] 💡 Per-rule `when`/context filters (e.g. only apply a rule set when the program is `kubectl`)
- [ ] 💡 Rule `include` files / rule-set packs (`include = ["k8s.toml"]`)
- [ ] 💡 16-color mode that maps to the terminal's own ANSI palette
- [ ] 💡 Hot-reload of config on `SIGHUP`

## 5. Conventions
- **Commits:** [Conventional Commits](https://www.conventionalcommits.org/) (`feat:`, `fix:`, `perf:`, `refactor:`, `test:`, `docs:`, `build:`, `ci:`, `chore:`).
- **Quality gate:** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`.
- **Release:** `make release` → `dist/ct-<version>-x86_64-linux-musl`.
