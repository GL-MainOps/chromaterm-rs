---
name: add-builtin-pattern
description: Add or change a built-in named regex pattern, named color, or default highlighting rule in chromaterm-rs (assets/builtin.toml). Use when asked to support a new log format, token type, or color role out of the box.
---

# Adding a built-in pattern / default rule

All built-ins live in **`assets/builtin.toml`** (embedded with `include_str!`).
Nothing is hard-coded in Rust.

1. **Pattern** — add to `[patterns]`:
   - Name: kebab-case noun (`ipv4`, `k8s-resource`). It's referenced as
     `pattern = "name"` or interpolated as `${name}` inside a rule `regex`.
   - Use TOML literal strings (`'…'` / `'''…'''`) so backslashes are not escaped.
   - **Linear-time syntax only** (no `(?=`, `(?!`, `(?<=`, `(?<!`, backrefs).
     Express boundaries with `\b`, character classes, or capture groups + group colors.
   - Anchor with `\b` on both ends where possible (it avoids partial-word hits).
2. **Color** — if a new semantic role is needed, add it to `[palette]` as an
   *alias of a raw hue* (`my-role = "teal"`), not as a hex. A genuinely new
   hue must be derived with the generator in `docs/COLOR-SYSTEM.md` (§9) and
   added to both its `HUES` table and `assets/builtin.toml`. Themes only override
   raw hues, so the role follows every theme automatically.
3. **Rule** — add a `[[rules]]` entry in the right layer:
   - Structural tokens (URLs, IPs, IDs, timestamps): `exclusive = true`, near the top.
   - Generic lexical tokens (numbers, booleans): non-exclusive, near the bottom.
   - Order matters: earlier exclusive rules win, later non-exclusive rules override styling.
4. **Test** — add a case in `src/engine/mod.rs` tests or `tests/highlight.rs`
   asserting the exact highlighted bytes, plus false-positive cases.
5. **Verify**: `cargo test && cargo run -- patterns` and benchmark with
   `cargo bench --bench highlight` when the rule could be hot.
