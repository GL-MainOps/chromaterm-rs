# Color System: Luminance-Targeted Contrast (LTC)

A portable, tool-agnostic specification for terminal and editor color themes
with **high contrast on near-black and white backgrounds**. It ships with
exact palettes, semantic roles, an ANSI-16 mapping, per-tool recipes, and a
stdlib-only generator that reproduces or extends every value.

Origin: the default themes of [chromaterm-rs](../README.md) (v1.0.1+), where a
unit test enforces these rules. You may copy this file into any project (MIT).

---

## 0. Instructions for AI agents

Read this section first. When asked to theme a tool (vim/neovim, tmux,
starship, zellij, a terminal emulator, a prompt, a TUI, a CLI highlighter…)
"using LTC" or "per COLOR-SYSTEM.md":

1. **Pick the theme** that matches the user's terminal: `dark` (background
   `#000000`…`#0E1317`, white text) or `light` (background `#FFFFFF`, black
   text). If unknown, produce both.
2. **Use only colors from §4** (palettes, backgrounds, ANSI-16). Never invent
   or "tweak" hex values by eye. If a new color is needed, derive it with the
   generator (§9) and verify it with `--check`.
3. **Assign colors by semantic role (§5), not by hue preference.** The same
   meaning gets the same color in every tool (an error is `red` in vim,
   starship, and tmux alike).
4. **Obey every MUST in §2.** In particular: text colors ≥ 4.5:1 on the
   background, never a near-text neutral for a highlight, and backgrounds from
   the `bg-*` set only.
5. **Prefer the tool's truecolor support** (`termguicolors`, `#rrggbb`
   styles). Where a tool only speaks ANSI colors, configure the terminal's
   ANSI-16 palette from §6 and reference ANSI names/indexes.
6. **Before finishing, run the checklist in §10** and report any rule you
   could not satisfy.

The key words MUST, MUST NOT, SHOULD and MAY are used as in RFC 2119.

---

## 1. Target environments

| Theme | Background (canvas) | Default text | Use for |
|---|---|---|---|
| `dark` | `#000000` to `#0E1317` (near-black, cool) | `#FFFFFF` / `#F0F3F6` | dark terminals and editors |
| `light` | `#FFFFFF` | `#000000` / `#1F2328` | light terminals and editors |

For a different canvas (e.g. `#1E1E2E` or `#FDF6E3`), re-derive the targets
with §8 instead of reusing these values unchanged.

---

## 2. Rules

### Contrast
- **R1 (MUST).** Every color used for text has a contrast ratio
  **≥ 4.5:1** (WCAG 2.x AA) against every canvas of its theme. Exempt:
  purely decorative, non-text elements (borders, separators, inactive line
  numbers), which SHOULD still be ≥ 3:1.
- **R2 (MUST, light theme).** A highlight color is **≥ 2:1 away from the
  default text color** (target ≈ 4:1). Otherwise it reads as "more black"
  rather than as a color.
- **R3 (MUST, dark theme).** A highlight MUST NOT be a near-white neutral
  (`white`, `silver`) or a desaturated pastel. Highlights read as *color*,
  not as dimmed text. Neutrals are for de-emphasis only.
- **R4 (MUST).** Background highlights use the `bg-*` set: visible on the
  canvas (**≥ 1.4:1**, target ≈ 2:1 dark / ≈ 1.6:1 light) **and** text on
  them ≥ 7:1 (achieved ≥ 9.5:1). Never use a saturated foreground hue as a
  background behind normal text.

### Construction
- **R5.** Colors are defined by **hue + saturation + target relative
  luminance**, and lightness is *solved* so the color hits the target. Do
  not pick lightness by eye; equal-lightness colors differ widely in
  perceived brightness.
- **R6. Per-family luminance targets.** Reds, blues, violets and magentas
  look vivid at lower luminance; yellows, limes and greens need more. Dark
  targets range from 0.25 (scarlet) to 0.58 (yellow), and light targets from
  0.11 to 0.17 (see §9 `HUES`).
- **R7. High saturation** (mostly 0.8–1.0) for hues, so they stay
  distinguishable from neutral text. Earthy/muted hues (`tan`, `brown`,
  `steel`) are the only exceptions (0.45–0.6) and are for low-priority data.
- **R8. Separate confusable meanings by hue, not just luminance.** In
  particular, *numbers* (`salmon`) must not look like *errors* (`red`). In
  the light theme `salmon` is shifted to hue 22° (orange-red) for this
  reason.
- **R9. Neutrals are a cool gray ramp** (hue ≈ 215°): `white` > `silver` >
  `gray` > `slate` > `charcoal`. `gray` is the lowest neutral allowed for
  readable secondary text (comments, debug). `charcoal` is decorative only.
- **R10. Light theme swaps ink roles:** `white` is the dark ink (`#1f2328`)
  and `black` is the light color drawn *on* saturated or `bg-*` surfaces.
  Configs written as `f.white b.bg-red` then work in both themes.

### Emphasis
- **R11.** Severity escalates by **adding attributes**, not only by shifting
  hue: `warning` → `error` + **bold** → `critical` = **bold text on
  `bg-red`**. The top severity MUST differ from the one below it by more than
  a hue step.
- **R12.** Do not rely on color alone for meaning that matters (status,
  diff, errors). Pair it with a symbol, bold, or position where the tool allows.
- **R13. 256-color fallback:** map each hex to the nearest xterm-256 entry
  (6×6×6 cube or gray ramp, by Euclidean RGB distance). Re-check R1 after
  mapping, since gray-ramp quantization can lower contrast.

---

## 3. Visual summary

```
DARK  (#0E1317, white text)            LIGHT (#FFFFFF, black text)
status:  error ■ warning ■ success ■    error ■ warning ■ success ■
         saturated, ≥ 5.3:1 on bg        mid-dark, ≥ 4.8:1 on bg, ≈ 4:1 from text
data:    blue keys, green strings,       same roles, same hue families
         orange-pink numbers, steel time
loud:    bold white on bg-red (FATAL)    bold ink on pale bg-red (FATAL)
```

---

## 4. Palettes

### 4.1 Foreground colors (neutrals and hues)

| Name | Dark | on `#0E1317` | vs white text | Light | on `#FFFFFF` | vs black text |
|---|---|---|---|---|---|---|
| `white` | `#f0f3f6` | 16.8:1 | 1.1:1 | `#1f2328` | 15.8:1 | 1.3:1 |
| `silver` | `#b6bfca` | 10.0:1 | 1.9:1 | `#3d444d` | 9.8:1 | 2.1:1 |
| `gray` | `#8f99a5` | 6.5:1 | 2.9:1 | `#59636e` | 6.1:1 | 3.4:1 |
| `slate` | `#7c8794` | 5.1:1 | 3.7:1 | `#6a7480` | 4.7:1 | 4.4:1 |
| `charcoal` | `#5c6570` | 3.2:1 | 5.9:1 | `#8b949f` | 3.1:1 | 6.8:1 |
| `surface` | `#191d22` | 1.1:1 | 16.9:1 | `#f0f2f5` | 1.1:1 | 18.7:1 |
| `black` | `#0b0e12` | 1.0:1 | 19.3:1 | `#ffffff` | 1.0:1 | 21.0:1 |
| `scarlet` | `#ff3b55` | 5.3:1 | 3.5:1 | `#cd001b` | 5.8:1 | 3.6:1 |
| `red` | `#ff594d` | 6.0:1 | 3.1:1 | `#d80e00` | 5.3:1 | 4.0:1 |
| `salmon` | `#fb8256` | 7.5:1 | 2.5:1 | `#c84900` | 4.8:1 | 4.4:1 |
| `orange` | `#ff8f2c` | 8.2:1 | 2.3:1 | `#b45400` | 5.0:1 | 4.2:1 |
| `amber` | `#f8a500` | 9.2:1 | 2.0:1 | `#936200` | 5.3:1 | 4.0:1 |
| `gold` | `#e8b900` | 10.1:1 | 1.8:1 | `#836900` | 5.3:1 | 4.0:1 |
| `yellow` | `#d7cd0b` | 11.2:1 | 1.7:1 | `#746f06` | 5.2:1 | 4.0:1 |
| `lime` | `#88cf1e` | 9.8:1 | 1.9:1 | `#4f7811` | 5.2:1 | 4.0:1 |
| `green` | `#22c34b` | 8.0:1 | 2.3:1 | `#167d30` | 5.2:1 | 4.0:1 |
| `mint` | `#17cc89` | 8.9:1 | 2.1:1 | `#0e7b53` | 5.3:1 | 4.0:1 |
| `teal` | `#05c3aa` | 8.4:1 | 2.2:1 | `#037a6a` | 5.3:1 | 4.0:1 |
| `cyan` | `#0cbce8` | 8.3:1 | 2.2:1 | `#087691` | 5.2:1 | 4.0:1 |
| `sky` | `#40acff` | 7.6:1 | 2.5:1 | `#006ec2` | 5.2:1 | 4.0:1 |
| `blue` | `#6593ff` | 6.4:1 | 2.9:1 | `#0e56ff` | 5.5:1 | 3.8:1 |
| `indigo` | `#8888fc` | 6.2:1 | 3.0:1 | `#4e4efa` | 5.5:1 | 3.8:1 |
| `violet` | `#ad7ffc` | 6.4:1 | 2.9:1 | `#7e36fa` | 5.5:1 | 3.8:1 |
| `purple` | `#cc70f3` | 6.4:1 | 2.9:1 | `#a512e4` | 5.5:1 | 3.8:1 |
| `magenta` | `#ed5cde` | 6.4:1 | 2.9:1 | `#c115b0` | 5.2:1 | 4.0:1 |
| `pink` | `#fb65b0` | 6.8:1 | 2.8:1 | `#d9066f` | 5.0:1 | 4.2:1 |
| `tan` | `#d7a75f` | 8.5:1 | 2.2:1 | `#8f6424` | 5.2:1 | 4.0:1 |
| `brown` | `#c7743d` | 5.3:1 | 3.5:1 | `#894f28` | 6.5:1 | 3.2:1 |
| `steel` | `#88add7` | 8.0:1 | 2.3:1 | `#396eab` | 5.3:1 | 4.0:1 |

`charcoal`, `black` and `surface` are decorative/surface colors (exempt from
R1). `surface` is one step off the canvas, for status bars, cursor lines and
panels. In
dark, `white` is default-text-like and in light it is the ink (R10), so it is
not a highlight.

### 4.2 Background highlight colors

| Name | Dark | on `#0E1317` | white text on it | Light | on `#FFFFFF` | black text on it |
|---|---|---|---|---|---|---|
| `bg-red` | `#82212a` | 1.96:1 | 9.5:1 | `#f0c4c8` | 1.56:1 | 13.4:1 |
| `bg-orange` | `#663b1a` | 1.96:1 | 9.5:1 | `#ebc8ae` | 1.57:1 | 13.4:1 |
| `bg-yellow` | `#524415` | 1.95:1 | 9.6:1 | `#e2ce8b` | 1.56:1 | 13.4:1 |
| `bg-green` | `#1a4f2c` | 1.96:1 | 9.6:1 | `#9addb1` | 1.57:1 | 13.4:1 |
| `bg-teal` | `#174e46` | 1.97:1 | 9.5:1 | `#8bddd2` | 1.57:1 | 13.4:1 |
| `bg-blue` | `#1e4679` | 1.96:1 | 9.5:1 | `#bad1ee` | 1.56:1 | 13.4:1 |
| `bg-purple` | `#5d2d88` | 1.95:1 | 9.6:1 | `#dbc7ec` | 1.57:1 | 13.4:1 |
| `bg-gray` | `#3f4650` | 1.96:1 | 9.5:1 | `#cacfd6` | 1.57:1 | 13.4:1 |

---

## 5. Semantic roles

Use the **role**, and the role maps to a palette name. These are the mappings
used by chromaterm-rs. Keep them when theming other tools so meaning stays
consistent everywhere.

| Group | Role → palette name |
|---|---|
| Status | `critical` → **bold** on `bg-red` (text `white`); `error` → `red` **bold**; `warning` → `amber`; `success` → `green`; `info` → `sky`; `notice` → `teal`; `debug` → `gray`; `muted` → `slate` |
| Values | `number` → `salmon`; `string` → `green`; `boolean` → `yellow`; `null` → `gray` *italic*; `version` → `gold`; `size` → `teal`; `duration` → `mint` |
| Identifiers | `url` → `sky` *underline*; `email` → `pink`; `uuid` → `violet`; `hash` → `gray`; `checksum` → `purple`; `pointer` → `gray`; `mac` → `indigo`; `ipv4` → `cyan`; `ipv6` → `magenta` |
| System | `timestamp` → `steel`; `path` → `steel`; `process` → `indigo`; `pid` → `cyan`; `cloud-id` → `teal`; `k8s-id` → `blue`; `method` → `indigo` **bold**; `protocol` → `sky` |
| Structure | `config-key` → `blue`; `operator` → `silver` (punctuation may be subtle) |

Editor/UI roles derived from the same groups:

| UI element | Color |
|---|---|
| Normal text | theme default text |
| Comments, secondary text | `gray` (R9: never `charcoal`/`slate` for text you must read) |
| Keywords / control flow | `violet` |
| Functions / commands | `blue` |
| Types / classes | `teal` |
| Constants / booleans | `yellow` |
| Strings | `green` |
| Numbers | `salmon` |
| Errors / diagnostics | `red` (error), `amber` (warning), `sky` (info), `teal` (hint) |
| Diff added / removed / changed (background) | `bg-green` / `bg-red` / `bg-blue` |
| Selection / visual mode | `bg-blue` |
| Search match | `bg-yellow`; current match `bg-orange` |
| Cursor line, status bar, panels | `surface` (dark `#191d22` = ANSI 0; light `#f0f2f5`) |
| Borders, separators, inactive line numbers | `charcoal` (decorative) |
| Active border / focused element | `blue` |

---

## 6. ANSI-16 terminal palette

For terminal emulators, and for tools that only emit ANSI colors (tmux and
zellij defaults, many CLIs, shells). Derived by the generator (§9): normal
colors reuse the palette, and bright colors use the same hue at higher
luminance (dark) or a slightly lighter, still-AA tone (light).

| # | Name | Dark | Light |
|---|---|---|---|
| 0 | black | `#191d22` | `#1f2328` |
| 1 | red | `#ff594d` | `#d80e00` |
| 2 | green | `#22c34b` | `#167d30` |
| 3 | yellow | `#e8b900` | `#936200` |
| 4 | blue | `#6593ff` | `#0e56ff` |
| 5 | magenta | `#ed5cde` | `#c115b0` |
| 6 | cyan | `#0cbce8` | `#087691` |
| 7 | white | `#b6bfca` | `#6a7480` |
| 8 | bright black | `#8f99a5` | `#59636e` |
| 9 | bright red | `#ff8d85` | `#eb1000` |
| 10 | bright green | `#5ae27c` | `#188834` |
| 11 | bright yellow | `#ffd736` | `#a06b00` |
| 12 | bright blue | `#91b2ff` | `#2a6aff` |
| 13 | bright magenta | `#f390e9` | `#d117bf` |
| 14 | bright cyan | `#6ddcf7` | `#08809e` |
| 15 | bright white | `#f0f3f6` | `#3d444d` |

Conventions:
- **Dark** `0 black` (`#191d22`) is a *surface* color (status bars,
  selections), not text. `8 bright black` is readable (6.5:1). Many themes
  make it too dim, which breaks comments, autosuggestions and `ls` output.
- **Light** `7 white` is `slate` and `15 bright white` is `silver` (dark
  grays). Programs that print "white" or "bright white" text stay visible on
  a white background. That is deliberate.
- Terminal `foreground`/`background`/`cursor`: dark `#F0F3F6` on `#0E1317`
  (or `#000000`), cursor `#6593ff`. Light `#1F2328` on `#FFFFFF`, cursor
  `#0e56ff`. Selection background is `bg-blue` (dark `#1e4679`, light
  `#bad1ee`).

---

## 7. Applying it to tools

General recipe: (1) set the terminal ANSI-16 palette (§6), (2) in each tool,
use truecolor hex values by role (§5), (3) fall back to ANSI indexes only
where the tool has no truecolor support.

**Neovim / Vim** (`set termguicolors`), dark example:
```vim
hi Normal       guifg=#f0f3f6 guibg=NONE
hi Comment      guifg=#8f99a5 gui=italic
hi String       guifg=#22c34b
hi Number       guifg=#fb8256
hi Boolean      guifg=#d7cd0b
hi Keyword      guifg=#ad7ffc
hi Function     guifg=#6593ff
hi Type         guifg=#05c3aa
hi Error        guifg=#ff594d gui=bold
hi WarningMsg   guifg=#f8a500
hi Visual       guibg=#1e4679
hi Search       guibg=#524415 guifg=#f0f3f6
hi DiffAdd      guibg=#1a4f2c
hi DiffDelete   guibg=#82212a
hi DiffChange   guibg=#1e4679
hi CursorLine   guibg=#191d22
hi LineNr       guifg=#5c6570
hi CursorLineNr guifg=#6593ff gui=bold
hi StatusLine   guifg=#f0f3f6 guibg=#191d22
```

**tmux**, dark:
```tmux
set -g status-style "bg=#191d22,fg=#b6bfca"
set -g window-status-current-style "fg=#6593ff,bold"
set -g pane-border-style "fg=#5c6570"
set -g pane-active-border-style "fg=#6593ff"
set -g message-style "bg=#1e4679,fg=#f0f3f6"
set -g mode-style "bg=#1e4679,fg=#f0f3f6"
```

**starship** (`starship.toml`), dark:
```toml
[directory]
style = "bold #40acff"          # sky: location
[git_branch]
style = "#ad7ffc"               # violet
[git_status]
style = "#f8a500"               # amber: warning-ish state
[character]
success_symbol = "[❯](bold #22c34b)"
error_symbol = "[❯](bold #ff594d)"
[cmd_duration]
style = "#17cc89"               # mint: duration
[hostname]
style = "#05c3aa"               # teal
[username]
style = "#8888fc"               # indigo: process/identity
```

**zellij** (theme file, classic color keys), dark:
```kdl
themes {
    ltc-dark {
        fg "#f0f3f6"
        bg "#191d22"
        black "#0e1317"
        red "#ff594d"
        green "#22c34b"
        yellow "#e8b900"
        blue "#6593ff"
        magenta "#ed5cde"
        cyan "#0cbce8"
        white "#b6bfca"
        orange "#ff8f2c"
    }
}
```

For the **light** theme, use the light column of §4 and §6 in the same slots.
Diff/selection/search backgrounds use the light `bg-*` values. Keep the
same role → name mapping.

---

## 8. Re-targeting to another background

Relative luminance `L` and contrast use WCAG 2.x:

```
lin(c) = c/12.92                  if c ≤ 0.03928   (c = channel/255)
       = ((c+0.055)/1.055)^2.4    otherwise
L      = 0.2126·lin(R) + 0.7152·lin(G) + 0.0722·lin(B)
ratio  = (L_lighter + 0.05) / (L_darker + 0.05)
```

For a canvas with luminance `Lc` and a required ratio `R`:
- dark canvas (text lighter): `L_min = R·(Lc + 0.05) − 0.05`
- light canvas (text darker): `L_max = (Lc + 0.05)/R − 0.05`

Procedure: change `THEMES[...]["canvas"]` in the generator, raise or lower
each hue's luminance target until `--check` passes with margin (≥ 5:1 is a
good working floor), and keep the R2/R3 distinctness from the new default
text color. Neutrals are hand-picked and must be re-checked too.

---

## 9. Reference generator

Stdlib-only Python 3. It reproduces every value in §4 and §6 exactly.
`python3 palette.py` prints JSON; `python3 palette.py --check` verifies all
floors (exit code 1 on failure). To add a hue, add one `HUES` entry
`(hue°, saturation, dark_lum, light_lum[, light_hue])`. To add a theme or
canvas, edit `THEMES`/`NEUTRALS`.

```python
#!/usr/bin/env python3
"""Reference generator for the "Luminance-Targeted Contrast" color system.

Stdlib only. `python3 palette.py`         -> JSON palette (dark + light)
             `python3 palette.py --check` -> contrast report, exit 1 on failure
"""
import colorsys, json, sys

# ---- 1. Targets ---------------------------------------------------------
THEMES = {
    # canvas = backgrounds the theme must work on; text = default foreground
    "dark":  {"canvas": ["#000000", "#0E1317"], "text": "#FFFFFF"},
    "light": {"canvas": ["#FFFFFF"],            "text": "#000000"},
}
# ---- 2. Hues: name -> (hue°, saturation, dark lum, light lum[, light hue]) ---
HUES = {
    "scarlet": (352, 1.00, 0.25, 0.13), "red":     (4,   1.00, 0.29, 0.15),
    "salmon":  (16,  0.95, 0.37, 0.17, 22),  # light: shifted toward orange, away from red
    "orange":  (28,  1.00, 0.41, 0.16), "amber":   (40,  1.00, 0.47, 0.15),
    "gold":    (48,  1.00, 0.52, 0.15), "yellow":  (57,  0.90, 0.58, 0.15),
    "lime":    (84,  0.75, 0.50, 0.15), "green":   (135, 0.70, 0.40, 0.15),
    "mint":    (158, 0.80, 0.45, 0.15), "teal":    (172, 0.95, 0.42, 0.15),
    "cyan":    (192, 0.90, 0.42, 0.15), "sky":     (206, 1.00, 0.38, 0.15),
    "blue":    (222, 1.00, 0.31, 0.14), "indigo":  (240, 0.95, 0.30, 0.14),
    "violet":  (262, 0.95, 0.31, 0.14), "purple":  (282, 0.85, 0.31, 0.14),
    "magenta": (306, 0.80, 0.31, 0.15), "pink":    (330, 0.95, 0.33, 0.16),
    "tan":     (36,  0.60, 0.43, 0.15), "brown":   (24,  0.55, 0.25, 0.11),
    "steel":   (212, 0.50, 0.40, 0.15),
}
LIGHT_SAT_OVERRIDE = {"salmon": 1.00}
# ---- 3. Backgrounds for "loud" highlights: name -> (hue°, saturation) --------
BGS = {"bg-red": (354, .60), "bg-orange": (26, .60), "bg-yellow": (46, .60),
       "bg-green": (140, .50), "bg-teal": (172, .55), "bg-blue": (214, .60),
       "bg-purple": (272, .50), "bg-gray": (215, .12)}
BG_LUM = {"dark": 0.06, "light": 0.62}
# ---- 4. Neutrals (hand-picked, cool gray ramp) ---------------------------------
NEUTRALS = {
    "dark":  {"white": "#f0f3f6", "silver": "#b6bfca", "gray": "#8f99a5",
              "slate": "#7c8794", "charcoal": "#5c6570", "black": "#0b0e12",
              "surface": "#191d22"},  # status bars, cursor line, panels
    "light": {"white": "#1f2328", "silver": "#3d444d", "gray": "#59636e",
              "slate": "#6a7480", "charcoal": "#8b949f", "black": "#ffffff",
              "surface": "#f0f2f5"},
}
DECORATIVE = {"charcoal", "black", "white", "surface"}   # exempt from the 4.5:1 floor

# ---- math ------------------------------------------------------------------------
def _lin(c): return c / 12.92 if c <= 0.03928 else ((c + 0.055) / 1.055) ** 2.4
def lum_rgb(r, g, b): return 0.2126 * _lin(r) + 0.7152 * _lin(g) + 0.0722 * _lin(b)
def rgb(h): h = h.lstrip("#"); return [int(h[i:i + 2], 16) / 255 for i in (0, 2, 4)]
def hexa(c): return "#" + "".join(f"{round(min(1, max(0, x)) * 255):02x}" for x in c)
def lum(h): return lum_rgb(*rgb(h))
def contrast(a, b):
    x, y = sorted((lum(a) + .05, lum(b) + .05), reverse=True); return x / y

def solve(hue, sat, target):
    """HSL color with this hue/saturation whose relative luminance == target."""
    lo, hi = 0.0, 1.0
    for _ in range(60):
        mid = (lo + hi) / 2
        if lum_rgb(*colorsys.hls_to_rgb(hue / 360, mid, sat)) < target: lo = mid
        else: hi = mid
    return hexa(colorsys.hls_to_rgb(hue / 360, (lo + hi) / 2, sat))

def build():
    out = {}
    for theme in THEMES:
        p = dict(NEUTRALS[theme])
        for name, spec in HUES.items():
            h, s, dl, ll = spec[:4]
            if theme == "dark": p[name] = solve(h, s, dl)
            else: p[name] = solve(spec[4] if len(spec) > 4 else h, LIGHT_SAT_OVERRIDE.get(name, s), ll)
        for name, (h, s) in BGS.items(): p[name] = solve(h, s, BG_LUM[theme])
        out[theme] = p
    return out

def ansi16(pal, theme):
    """Terminal palette (color0..15) derived from the same rules."""
    if theme == "dark":
        bright = lambda n: solve(HUES[n][0], HUES[n][1], min(0.70, HUES[n][2] * 1.45))
        return [pal["surface"], pal["red"], pal["green"], pal["gold"], pal["blue"],
                pal["magenta"], pal["cyan"], pal["silver"],
                pal["gray"], bright("red"), bright("green"), bright("gold"), bright("blue"),
                bright("magenta"), bright("cyan"), pal["white"]]
    bright = lambda n: solve(HUES[n][0], HUES[n][1], 0.18)
    return [pal["white"], pal["red"], pal["green"], pal["amber"], pal["blue"],
            pal["magenta"], pal["cyan"], pal["slate"],
            pal["gray"], bright("red"), bright("green"), bright("amber"), bright("blue"),
            bright("magenta"), bright("cyan"), pal["silver"]]

def check(pals):
    bad = []
    for theme, spec in THEMES.items():
        for name, c in pals[theme].items():
            if name in DECORATIVE: continue
            for canvas in spec["canvas"]:
                cr = contrast(c, canvas)
                if name.startswith("bg-"):
                    on = contrast(c, spec["text"])
                    if cr < 1.4 or on < 7: bad.append(f"{theme}/{name}: canvas {cr:.2f}, text-on {on:.1f}")
                elif cr < 4.5: bad.append(f"{theme}/{name}: {cr:.2f}:1 on {canvas}")
            if theme == "light" and not name.startswith("bg-") and contrast(c, spec["text"]) < 2.0:
                bad.append(f"light/{name}: too close to text")
        a = ansi16(pals[theme], theme)
        for i, c in enumerate(a):
            if i in (0,) or (theme == "light" and i in (7, 15)) or (theme == "dark" and i in ()):
                continue
            for canvas in spec["canvas"]:
                if contrast(c, canvas) < 4.5: bad.append(f"{theme}/ansi{i} {c}: {contrast(c, canvas):.2f}")
    return bad

if __name__ == "__main__":
    pals = build()
    if "--check" in sys.argv:
        bad = check(pals); print("\n".join(bad) or "OK: all contrast floors met"); sys.exit(1 if bad else 0)
    print(json.dumps({t: {"palette": pals[t], "ansi16": ansi16(pals[t], t)} for t in pals}, indent=2))
```

---

## 10. Checklist (run before calling a theme done)

- [ ] Every text color ≥ 4.5:1 on each background (R1). Decorative ≥ 3:1.
- [ ] Light: every highlight ≥ 2:1 from black text (R2).
- [ ] Dark: no highlight uses `white`/`silver` or a pastel (R3).
- [ ] Backgrounds come from `bg-*`, are visible on the canvas, and text on them is ≥ 7:1 (R4).
- [ ] Same role → same color across all tools (§5).
- [ ] Errors vs numbers, critical vs error, and warning vs success are distinguishable at a glance (R8, R11).
- [ ] ANSI `bright black` is readable and light-theme "white" ANSI slots are visible (§6).
- [ ] Any new color came from the generator, and `--check` passes (§9).

### Common mistakes this system prevents
- Pastel "Nord-like" highlights on dark that read as dim text.
- Solarized-style low-contrast accents (≈ 3:1) on light backgrounds.
- Bright-black (`color8`) so dim that comments and suggestions disappear.
- Yellow or white text that vanishes on a white background.
- Tinted backgrounds (`#2a1a1a` on `#0E1317`) that are invisible.
- Red numbers that look like errors.
