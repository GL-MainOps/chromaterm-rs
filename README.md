# chromaterm-rs — `ct`

**Fast, static, regex-based terminal output highlighter.** A Rust rewrite of
[ChromaTerm](https://github.com/hSaria/ChromaTerm) shipped as a single ~3 MB
static binary: no Python, no runtime, no dependencies.

```sh
tail -f /var/log/syslog | ct           # highlight a stream
ct ssh router1                         # run a program under a highlighting PTY
ct kubectl get pods -A                 # …works with interactive programs too
```

- **~10× faster than Python ChromaTerm** on the same config (see [Performance](#performance)).
- **Sane defaults with zero config**: URLs, IPs (v4/v6/CIDR), MACs, UUIDs, hashes,
  timestamps, durations, sizes, versions, paths, PIDs, HTTP methods and status
  codes, log levels, JSON keys, key=value, booleans, numbers. Dark and light themes.
- **Readable config**: TOML with comments and raw regex strings (`'\d+'`, no `\\d`),
  **named colors** (`f.error`, `f.ipv4`) and **named patterns** (`pattern = "ipv4"`,
  `regex = 'from ${ipv4}'`). Raw regexes and `#hex` colors always work too.
- **Config from anywhere**: file, `$CHROMATERM_CONFIG`, XDG paths, or **inline on
  the command line** as one-line JSON or TOML.
- **Drop-in for existing users**: loads Python ChromaTerm YAML configs as they
  are, and `ct config import` converts them to TOML.
- **Safe on untrusted output**: linear-time regex engine by default (no ReDoS),
  step-limited backtracking only where a pattern needs it, bounded buffers,
  and the text itself is never altered (property-tested).

---

## Contents

- [Install](#install)
- [Usage](#usage)
- [Configuration](#configuration)
  - [Where config comes from](#where-config-comes-from)
  - [Why TOML (and JSON inline)?](#why-toml-and-json-inline)
  - [Schema reference](#schema-reference)
  - [Colors](#colors)
  - [Patterns](#patterns)
  - [Rules and how they combine](#rules-and-how-they-combine)
  - [Inline config on the command line](#inline-config-on-the-command-line)
  - [Export and convert](#export-and-convert)
  - [Reloading running instances](#reloading-running-instances)
- [Migrating from Python ChromaTerm](#migrating-from-python-chromaterm)
- [Performance](#performance)
  - [glibc vs musl builds](#glibc-vs-musl-builds)
- [Security](#security)
- [Building](#building)
- [CI/CD and releases](#cicd-and-releases)
- [Development](#development)

---

## Install

### Release binaries

Every release (GitLab Releases / GitHub Releases) ships four binaries plus
`SHA256SUMS`:

| File | Runs on | Pick it when |
|---|---|---|
| `ct-<v>-x86_64-linux-gnu` | x86_64 Linux with glibc ≥ 2.28 (RHEL 8+, Debian 10+, Ubuntu 20.04+, Fedora, Arch…) | **Default choice: fastest** ([numbers](#glibc-vs-musl-builds)) |
| `ct-<v>-aarch64-linux-gnu` | arm64 Linux with glibc ≥ 2.28 | arm64 servers, Raspberry Pi OS 64-bit |
| `ct-<v>-x86_64-linux-musl` | **any** x86_64 Linux (fully static) | Alpine, scratch/distroless containers, very old or unknown systems |
| `ct-<v>-aarch64-linux-musl` | **any** arm64 Linux (fully static) | same, on arm64 |

```sh
sha256sum -c --ignore-missing SHA256SUMS
install -Dm755 ct-*-x86_64-linux-gnu ~/.local/bin/ct
```

### Build locally
```sh
make link           # optimized native build, symlinked as ~/.local/bin/ct (see Development)
make release        # static musl binary → dist/
make release-all    # all four release binaries + SHA256SUMS → dist/
```

### From source
```sh
cargo install --path .            # installs `ct` into ~/.cargo/bin
```

### Shell completions
```sh
ct completions bash > ~/.local/share/bash-completion/completions/ct
ct completions zsh  > ~/.zfunc/_ct
ct completions fish > ~/.config/fish/completions/ct.fish
```

---

## Usage

```
ct [OPTIONS] [PROGRAM [ARGS]...]
ct [OPTIONS] <COMMAND>
```

| Mode | Example | Notes |
|---|---|---|
| Filter | `journalctl -f \| ct` | Highlights stdin → stdout. |
| Program | `ct ssh host`, `ct make -j8` | Runs PROGRAM in a pseudo-terminal: colors, prompts, `vim`, `top`, window resizing and Ctrl-C all work. Exits with PROGRAM's exit code. |
| Explicit | `ct run -- config` | For programs whose name clashes with a subcommand. |

### Options

| Option | Env | Description |
|---|---|---|
| `-c, --config PATH` | `CHROMATERM_CONFIG` | Config file (TOML, JSON, or legacy YAML). |
| `-i, --inline CONFIG` | | Inline config, JSON (`{…}`) or TOML. Repeatable; layered in order. |
| `-N, --no-config` | | Ignore config files (built-ins + `--inline` only). |
| `-t, --theme NAME` | `CHROMATERM_THEME` | Theme: `dark` (default), `light`, or your own. |
| `--color-mode MODE` | | `auto` (default), `truecolor`, `256`. |
| `-R, --rgb` | | Force truecolor (same as Python ChromaTerm's `-R`). |
| `--read-timeout MS` | | Wait for the rest of a partial line (default 2 ms). |
| `-b, --benchmark` | | On exit, print per-rule time and match counts to stderr. |
| `-r, --reload` | | Make all running `ct` instances reload their config, then exit (Python ChromaTerm's `-r`). |
| `-v, -V, --version` | | Print version. |

### Commands

| Command | What it does |
|---|---|
| `ct config init [--full] [-o PATH\|-] [--force]` | Write a fully commented config with examples before every section. `--full` writes an editable copy of all built-in rules, patterns and colors. |
| `ct config check [PATH]` | Validate. Reports **every** problem at once with rule number, description and file. Exit code 1 on error. |
| `ct config show [--json]` | Print the effective, merged config (self-contained; usable as a config file). |
| `ct config path` | Show the search order and which file is active. |
| `ct config export [FILE] [-F toml\|json\|yaml] [--oneline] [--shell] [--effective] [-o PATH]` | Convert or export a config to any supported format (alias: `ct config convert`). [Details](#export-and-convert). |
| `ct config reload` | Make all running `ct` instances reload their config (same as `ct -r`). |
| `ct config import OLD.yml [-o NEW.toml]` | Convert a Python ChromaTerm YAML config to commented TOML. |
| `ct patterns [NAME]` | List named patterns, or show one fully expanded. |
| `ct colors` | List named colors with live swatches (theme-aware). |
| `ct explain TEXT…` | Show which rule colors which part of a line (reads stdin if no TEXT). |
| `ct completions SHELL` | bash, zsh, fish, elvish, powershell. |

```console
$ ct explain 'Oct  9 00:28:01 web sshd[12]: Failed password from 10.0.0.5 port 22'
(the highlighted line)
    0..15  "Oct  9 00:28:01"        rule #12 Timestamp (ISO 8601, syslog, access log) [exclusive]
   20..24  "sshd"                   rule #18 Program with PID, e.g. sshd[1234] [exclusive]
   25..27  "12"                     rule #18 Program with PID, e.g. sshd[1234] [exclusive]
   30..36  "Failed"                 rule #30 Error / failure [exclusive]
   51..59  "10.0.0.5"               rule #17 IPv4 address / CIDR [exclusive]
   65..67  "22"                     rule #37 Number
```

---

## Configuration

**No config is needed.** The built-in defaults are designed for logs, DevOps
tools and general terminal work. Create a config to customize:

```sh
ct config init          # → ~/.config/chromaterm/config.toml, fully commented
$EDITOR ~/.config/chromaterm/config.toml
ct config check
```

### Where config comes from

Layers, lowest → highest precedence:

1. **Built-in defaults** (always present): palette, themes, named patterns, default rules.
2. **One config file**, the first found of:
   1. `--config PATH` or `$CHROMATERM_CONFIG`
   2. `$XDG_CONFIG_HOME/chromaterm/config.toml` (default `~/.config/chromaterm/config.toml`)
   3. `$XDG_CONFIG_HOME/chromaterm/config.json`
   4. `~/.chromaterm.toml`, `~/.chromaterm.json`
   5. Legacy YAML: `$XDG_CONFIG_HOME/chromaterm/chromaterm.yml`, `~/.chromaterm.yml`, `~/.chromaterm.yaml`
   6. `/etc/chromaterm/config.toml`, `/etc/chromaterm/chromaterm.yml`
3. **Inline configs** (`-i`), in command-line order.

Merging: `palette`, `themes`, `patterns` and `settings` merge **by key** (later
wins). **Rules** run in this order: inline rules → file rules → built-in rules
(when `defaults = true`, the default).

### Why TOML (and JSON inline)?

Multi-line support matters: long rules read far better split across lines
with comments (`(?x)` verbose regexes). We compared the options:

| | YAML (Python ChromaTerm) | JSON | **TOML** |
|---|---|---|---|
| Comments | ✅ | ❌ | ✅ |
| Regex without doubling backslashes | ⚠️ block scalars only | ❌ `"\\d+"` | ✅ `'\d+'` |
| Multi-line strings | ✅ | ❌ | ✅ `'''…'''` |
| No implicit-typing traps (`no`→false, `1.10`→1.1) | ❌ | ✅ | ✅ |
| One-liner on a command line | ⚠️ | ✅ | ✅ (one key per `-i`) |

So **files are TOML** and **JSON is accepted everywhere** (`*.json` files and
`-i '{…}'`), because JSON is the most natural one-liner. YAML remains readable for
migration only.

### Schema reference

```toml
version  = 1          # schema version (optional; currently 1)
defaults = true       # append the built-in rules after yours (default true)
theme    = "dark"     # active theme (also --theme / $CHROMATERM_THEME)

[settings]
read_timeout_ms = 2       # wait for the rest of a partial line (ms)
color_mode      = "auto"  # "auto" | "truecolor" | "256"
max_line_bytes  = 65536   # partial lines longer than this are flushed in pieces
unicode         = false   # Unicode-aware \w \d \s \b (see Performance)

[palette]                 # name = "#rrggbb" | "#rgb" | "ansi:N" | "default" | "<other name>"
brand = "#ff6600"
error = "brand"           # aliases resolve recursively (cycles are reported)

[themes.light]            # overlay applied when theme = "light"
brand = "#b34700"

[patterns]                # reusable regex fragments
ticket = '\b(?:OPS|INC)-[0-9]+\b'

[[rules]]
description = "Tickets"   # shown in errors, `ct explain`, `--benchmark`
regex       = '${ticket}' # OR: pattern = "ticket"
color       = "f.black b.amber bold"   # OR a table: { 1 = "f.key", name = "f.value" }
exclusive   = true        # later rules can't color inside this match
ignore_case = false       # same as a leading (?i)
unicode     = false       # per-rule override of settings.unicode
enabled     = true        # false keeps the rule but switches it off
```

Unknown keys are **errors** (typos like `colour` are caught). All problems are
reported together, for example:

```
ct: invalid configuration (2 problems):
  - rule #1 "bad re" (inline config #1): invalid regex: regex parse error: …
  - rule #2 (~/.config/chromaterm/config.toml): unknown color "nope" (see `ct colors`)
```

### Colors

A color spec is a space-separated list of tokens:

| Token | Meaning |
|---|---|
| `f.NAME` / `b.NAME` | Foreground / background from the palette (`f.error`, `b.bg-red`) |
| `f#rrggbb` / `b#rrggbb` (or `#rgb`) | Hex foreground / background |
| `NAME` / `#rrggbb` | Shorthand for a foreground color |
| `bold` `dim` `italic` `underline` `blink` `invert` `strike` | Styles (`reverse`, `strikethrough` are aliases) |

The built-in palette has **raw hues** and **semantic roles** that alias them, so a
theme only needs to change hues:

- Hues: `white silver gray slate charcoal black scarlet red salmon orange amber
  gold yellow lime green mint teal cyan sky blue indigo violet purple magenta pink
  tan brown steel`, plus backgrounds `bg-red bg-orange bg-yellow bg-green bg-teal
  bg-blue bg-purple bg-gray`
- Roles: `critical error warning success info notice debug muted number string
  boolean null timestamp duration size version url email ipv4 ipv6 mac uuid hash
  checksum pointer path process pid cloud-id k8s-id method protocol config-key operator`

Run `ct colors` to see them all with swatches. `ansi:N` colors follow your
terminal's own palette. With `--color-mode 256` (or auto-detected when
`$COLORTERM` isn't `truecolor`), hex colors map to the nearest xterm-256 color.

### Patterns

Named regex fragments, built-in or your own. Use one as a whole rule with
`pattern = "ipv4"`, or embed it with `${ipv4}` (expands to `(?:…)`; write `\${`
for a literal `${`):

`url email ipv4 ipv4-octet ipv6 ipv6-group mac uuid hash digest pointer aws-arn
k8s-resource path datetime date time month syslog-time clf-time duration size
semver number boolean null quoted http-method http-version log-critical log-error
log-warn log-ok log-info log-debug`

```toml
[[rules]]
regex = 'from (${ipv4}) port (${number})'
color = { 1 = "f.ipv4 bold", 2 = "f.number" }
```

`ct patterns` lists them. `ct patterns ipv6` shows one fully expanded.

### Rules and how they combine

Same model as Python ChromaTerm:

1. Rules run **in order** on each line. Existing escape sequences are stripped
   before matching, so a program's own colors never break a match.
2. A match that overlaps text **claimed by an earlier exclusive** match is dropped.
   Exclusive matches claim their text.
3. Overlapping non-exclusive highlights: the **later rule wins per attribute**
   (fg, bg, each style). So put broad rules (strings, key=value) first and
   precise ones after.
4. When a highlight ends, `ct` restores the **program's own** color for that
   attribute rather than resetting everything.

Regex syntax is [Rust `regex`](https://docs.rs/regex/latest/regex/#syntax)
(RE2-like, close to Python's `re`). Look-around (`(?=…)`, `(?<!…)`) and
back-references (`\1`) also work. Those patterns transparently use a backtracking
engine with a step limit. `ct config check` reports how many rules use it.

### Inline config on the command line

Any config can be given inline: JSON (starts with `{`) or TOML. Repeat `-i` to layer.

```sh
# One-line JSON: add a rule on top of the defaults
ct -i '{"rules":[{"regex":"\\bTODO\\b","color":"f.black b.amber bold"}]}' make

# Only your rules (no built-ins), as TOML
ct -i 'defaults = false' -i "rules = [{ regex = '\d+ms', color = 'duration' }]" ./bench

# Quick theme / settings overrides
ct -i 'theme = "light"' -i 'settings = { color_mode = "256" }' tail -f app.log
```

Turn any config file into a one-liner with `ct config export --oneline` (see below).

### Export and convert

`ct config export` (alias `ct config convert`) writes a config in any supported
format:

| Format | `-F` | Notes |
|---|---|---|
| TOML | `toml` (default) | The native format. |
| JSON | `json` | Pretty, or single-line with `--oneline`. |
| YAML | `yaml` | **Python ChromaTerm format.** Self-contained: the active theme is applied, palette colors become hex, `pattern`/`${…}`/`ignore_case` are expanded into the regex, and named groups become indexes. Anything the old format can't express (e.g. `dim`) is reported. |

What gets exported:

| Invocation | Content |
|---|---|
| `ct config export FILE` | That file, converted as-is (TOML, JSON or legacy YAML in). |
| `ct config export` | Your layers (config file + `-i` values) merged into one document, without built-ins. |
| `ct config export --effective` | Everything, built-ins included, as one self-contained config (`defaults = false`). |

The format is inferred from `-o`'s extension when `-F` is omitted.

```sh
ct config convert ~/.chromaterm.yml -o ~/.config/chromaterm/config.toml   # YAML → TOML
ct config export -o team.json                                              # your config → JSON
ct config export -F yaml --effective -o python-chromaterm.yml              # for Python ChromaTerm

# One line of JSON, ready for the command line:
ct config export ./rules.toml --oneline
#  → {"defaults":false,"rules":[{"regex":"\\bTODO\\b","color":"f.black b.amber bold"}]}
ct -N -i "$(ct config export ./rules.toml --oneline)" make               # use it directly

# …or as a shell-quoted argument to paste into a command or alias:
ct config export ./rules.toml --shell
#  → --inline '{"defaults":false,"rules":[…]}'
```

### Reloading running instances

Edit your config, then apply it to every `ct` already running (in any
terminal) without restarting them:

```sh
ct -r            # or: ct config reload
# Reloaded 3 running ct instances.
```

- The config is validated first. If it is broken, nothing is signalled and the
  errors are printed.
- Each instance re-reads its **own** sources (its `--config`/`-i` options and
  the search paths, so a config file created after start is picked up). Held-back
  output and the program's own colors carry over seamlessly. If the new config
  fails to load in an instance, that instance keeps its previous config and
  prints why.
- How it works: running instances register in `$XDG_RUNTIME_DIR/chromaterm/`
  (a private, per-user directory) with their PID and process start time.
  `ct -r` sends `SIGUSR1` only to registered, live instances whose start time
  still matches, so PID reuse or other programs called `ct` are never
  signalled. You can also send `SIGUSR1` to one instance yourself.

---

## Migrating from Python ChromaTerm

- Existing `~/.chromaterm.yml` files are **found and loaded automatically**.
- Convert once for the full feature set (themes, patterns, comments):
  ```sh
  ct config import ~/.chromaterm.yml -o ~/.config/chromaterm/config.toml
  ```
  Regexes are written as TOML literal strings (no escaping), `defaults = false`
  keeps your rule set exactly as it was, and YAML anchors/aliases are resolved.
- Rule semantics (`regex`, `color`, `exclusive`, group colors, `f.`/`b.` specs,
  palette) are the same. Python's `\Z` is translated to `\z`.
- Imported configs use `unicode = false` (ASCII `\w \d \s \b`): about 8× faster
  startup and 5× faster matching on a large real-world config. Set `unicode = true` under `[settings]` for
  Python's exact Unicode semantics.
- CLI: `-c`, `-b`, `-r`, `-R`, `-v` behave as before. `--pcre` isn't needed:
  look-around and back-references work out of the box.
- Going back is possible too: `ct config export -F yaml` writes a config that
  Python ChromaTerm loads (verified against Python ChromaTerm 0.10.7).

---

## Performance

Measured on the same machine with 10,000 log lines (860 KB, 20% with UTF-8
text). Python ChromaTerm is 0.10.7. `ct` is the static musl release binary.

| Workload | Python ChromaTerm | `ct` | Speed-up |
|---|---|---|---|
| Startup, default config | 0.07 s / 18 MB | **0.01 s / 5 MB** | 7× |
| 10k lines, each tool's default rules | 0.92 s | **0.09 s** | ~10× |
| 10k lines, 141-rule real-world config (37 with look-around) | 4.20 s | **0.40 s** | ~10× |
| … same, `unicode = true` (Python-identical semantics) | 4.20 s | 1.93 s | 2.2× |
| Passthrough, no rules | — | 2.4 GiB/s | — |

How:
- Patterns compile to the **linear-time `regex` automata** whenever possible.
- One **`RegexSet` pass per line** finds which rules can match at all; the rest are skipped.
- **ASCII classes by default** keep the fast DFA engine active on non-ASCII lines.
- **Allocation-free hot path** (reused buffers), with a fast path for lines with no escapes.
- **Minimal SGR output**: only attributes that change are emitted.
- `ct -b …` prints which of *your* rules cost the most.

Binary size: **~3.1 MB** (x86_64), 2.5–2.7 MB (aarch64). `make release-small`
builds a size-optimized variant without the YAML importer.

### glibc vs musl builds

Same code, two C libraries. The **glibc (`-gnu`) build is faster**: musl's
`malloc` is slower and takes one global lock (regex compilation allocates a
lot, so `ct` compiles rules serially on musl), and its `memcpy`/string
routines are less optimized. Best of 5 runs, 50,000 lines (4.3 MB), x86_64:

| Workload | `-gnu` | `-musl` | gnu advantage |
|---|---|---|---|
| Startup, built-in rules | < 0.01 s | 0.01 s | — |
| Startup, 141-rule config | **0.02 s** | 0.07 s | 3.5× |
| 50k lines, built-in rules | **0.26 s** | 0.38 s | 1.45× |
| 50k lines, 141-rule config | **1.32 s** | 1.51 s | 1.15× |
| 50k lines, 141-rule config, `unicode = true` | **6.13 s** | 6.83 s | 1.1× |

Both are far ahead of Python ChromaTerm. Use **`-gnu` by default** and
**`-musl` where glibc ≥ 2.28 is missing** (Alpine, scratch images, old
distros). The glibc binaries are linked with
[cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild) against
glibc 2.28 symbols, so they run on old distros even though they are built
on new ones.

---

## Security

- **No ReDoS by default**: the `regex` crate guarantees linear-time matching. Only
  patterns that need look-around/back-references use the backtracking engine,
  and it has a hard step limit per search. Results found before the limit are
  kept, and `--benchmark` reports when the limit is hit.
- **Bounded memory**: partial lines are capped (`max_line_bytes`), config files
  are capped at 4 MiB, and compiled regexes have a size limit.
- **YAML import is hardened**: aliases to collections are rejected (no "billion
  laughs"), and the node count is capped.
- **Output integrity**: highlights never split UTF-8 code points or escape
  sequences. Property tests verify that, for random byte streams, the output
  minus SGR codes equals the input minus SGR codes.
- **Strict config**: unknown keys are errors, and there is no code execution or
  include mechanism.
- No network access, no shell-outs, pure Rust dependencies. The only `unsafe`
  is the PTY `pre_exec` hook (async-signal-safe syscalls only).

---

## Building

Requirements: Rust stable (via [rustup](https://rustup.rs); `rust-toolchain.toml`
pins the channel and targets). **No C toolchain is needed**: every dependency
is pure Rust. musl targets link with the bundled linker. For glibc release
builds, `ci/build-release.sh` installs pinned `cargo-zigbuild` + `ziglang`
into a private Python venv (python3 required).

```sh
make link                                   # native optimized build → ~/.local/bin/ct
make release                                # static x86_64 musl → dist/
make release-gnu                            # fast x86_64 glibc ≥ 2.28 → dist/
make release-all                            # all four + SHA256SUMS → dist/
ci/build-release.sh aarch64-unknown-linux-gnu dist   # any single target
make release-small                          # opt-level=s, no YAML importer
```

| Cargo feature | Default | Purpose |
|---|---|---|
| `legacy-yaml` | on | Load/import Python ChromaTerm YAML configs |

---

## CI/CD and releases

GitLab is the origin and GitHub is a push mirror. Both run the **same pipeline**,
and all logic lives in `ci/*.sh`. **Pipelines run only when a version tag
(`vX.Y.Z`) is pushed.** Pushing a tag is the one and only release trigger.
Branch pushes, merge requests and pull requests start nothing, so run
`make check` locally before tagging.

| Stage | GitLab (`.gitlab-ci.yml`) | GitHub (`.github/workflows/release.yml`) |
|---|---|---|
| check | `lint` (rustfmt, clippy for both feature sets), `test` (both feature sets) | `check` |
| build | `build` matrix: x86_64/aarch64 × gnu/musl via `ci/build-release.sh` (verifies static linkage / glibc ≤ 2.28, smoke-tests) | `build` matrix (same script) |
| release | `version` (tag == Cargo.toml), then `release`: binaries + `SHA256SUMS` uploaded to the Generic Package Registry, GitLab Release created with links and a changelog | `version`, then `release`: GitHub Release with the same assets and changelog |

Cutting a release:

```sh
make check                                   # CI only runs on tags: verify first
# bump `version` in Cargo.toml (Cargo.lock follows on the next build), then:
git commit -am "chore(release): v1.1.0"
git tag -a v1.1.0 -m "ct v1.1.0"
git push origin main                         # code only: no pipeline
git push origin v1.1.0                       # tag → GitLab pipeline → mirror → GitHub run
```

First-time setup of runners, the GitHub mirror and permissions:
**[docs/CICD-SETUP.md](docs/CICD-SETUP.md)**.

Release notes come from the Conventional Commit subjects since the previous
tag (`ci/release-notes.sh`), grouped into Features, Fixes, Performance and
Other.

## Development

```sh
make check          # rustfmt --check + clippy -D warnings + tests (both feature sets)
make bench          # criterion; CT_BENCH_CONFIG=~/.my.toml adds your config
make link           # build the current tree and symlink it as ~/.local/bin/ct
```

Handy alias: rebuild the working tree and (re)link the binary:

```bash
# ~/.bashrc
alias ct-build='make -C ~/storageHub/.YOS/chromaterm-rs link'
```

`make link` builds with `cargo build --release --locked` for the native
target and symlinks `target/release/ct` to `~/.local/bin/ct` (`BINDIR=…` to
change). An existing `ct` that isn't this build, such as a pipx-installed
Python ChromaTerm, is moved aside to `ct.bak-<timestamp>`, never deleted.

- Architecture, decisions and roadmap: **[PLAN.md](PLAN.md)**
- Guide for AI coding agents (and humans): **[AGENTS.md](AGENTS.md)**
- Commits follow [Conventional Commits](https://www.conventionalcommits.org/).

Project layout:

```
src/ansi.rs           escape-sequence scanner, SGR state model
src/signals.rs        signal flags + self-pipe for the event loops
src/instances.rs      registry of running instances (for --reload)
src/color.rs          colors, styles, 256-color mapping
src/engine/           matcher (regex / fancy-regex) + highlighter/renderer
src/stream.rs         line framing, partial-line hold-back
src/config/           schema, layering, resolution, export, legacy YAML import
src/io.rs, pty.rs     filter mode / PTY program mode
src/cli.rs            command-line interface
assets/builtin.toml   built-in palette, themes, patterns, default rules
assets/template.toml  `ct config init` template
tests/                integration + property tests
benches/              criterion benchmarks
ci/                   build/release scripts shared by GitLab CI and GitHub Actions
```
