//! Turn parsed config layers into compiled rules.
//!
//! - palette + theme → concrete colors (aliases followed, cycles detected)
//! - color specs (`"f.error b#202020 bold"`) → [`Style`]
//! - `${pattern}` interpolation and `pattern = "name"` → regex source
//! - regex compilation via [`Matcher`] (fast engine first)

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use super::{ColorDef, ColorModeSetting, ConfigErrors, ConfigFile, Layers, Origin, RuleDef};
use crate::color::{Color, ColorMode, Style, flags};
use crate::engine::{Highlighter, Matcher, Rule, RuleStyles};

/// Default wait for the rest of a partial line.
pub const DEFAULT_READ_TIMEOUT_MS: u64 = 2;
const MAX_ALIAS_DEPTH: usize = 32;
const MAX_PATTERN_DEPTH: usize = 16;

/// Overrides that come from the command line / environment.
#[derive(Debug, Clone, Default)]
pub struct ResolveOptions {
    pub theme: Option<String>,
    pub color_mode: Option<ColorMode>,
    pub read_timeout_ms: Option<u64>,
}

/// A rule definition together with where it came from.
#[derive(Debug, Clone)]
pub struct SourcedRule {
    pub def: RuleDef,
    pub origin: Origin,
    /// 0-based index within its layer.
    pub index: usize,
}

impl SourcedRule {
    pub fn label(&self) -> String {
        match &self.def.description {
            Some(d) => format!("rule #{} \"{d}\" ({})", self.index + 1, self.origin),
            None => format!("rule #{} ({})", self.index + 1, self.origin),
        }
    }
}

/// All layers merged, before compilation.
#[derive(Debug, Clone)]
pub struct Merged {
    pub theme: String,
    pub themes: Vec<String>,
    /// Raw palette values with the active theme applied.
    pub palette: BTreeMap<String, String>,
    pub patterns: BTreeMap<String, (String, Origin)>,
    pub rules: Vec<SourcedRule>,
    pub defaults: bool,
    pub settings: super::Settings,
    pub file: Option<PathBuf>,
}

/// The fully resolved, ready-to-run configuration.
#[derive(Debug)]
pub struct Resolved {
    pub merged: Merged,
    pub palette: BTreeMap<String, Color>,
    pub rules: Vec<Rule>,
    pub color_mode: ColorMode,
    pub read_timeout: Duration,
    pub max_line_bytes: usize,
}

impl Resolved {
    pub fn into_highlighter(self) -> Highlighter {
        Highlighter::new(self.rules)
    }
}

/// Merge layers (see module docs of [`super`] for precedence).
pub fn merge(layers: &Layers, opts: &ResolveOptions) -> Result<Merged, ConfigErrors> {
    let mut palette = BTreeMap::new();
    let mut themes: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut patterns = BTreeMap::new();
    let mut settings = super::Settings::default();
    let mut theme = None;
    let mut defaults = None;
    let mut builtin_rules = Vec::new();
    let mut file_rules = Vec::new();
    let mut inline_rules = Vec::new();

    for (origin, cfg) in &layers.0 {
        palette.extend(cfg.palette.iter().map(|(k, v)| (k.clone(), v.clone())));
        for (name, overlay) in &cfg.themes {
            themes
                .entry(name.clone())
                .or_default()
                .extend(overlay.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        patterns.extend(
            cfg.patterns
                .iter()
                .map(|(k, v)| (k.clone(), (v.clone(), origin.clone()))),
        );
        settings.merge(&cfg.settings);
        let sourced = cfg
            .rules
            .iter()
            .enumerate()
            .map(|(index, def)| SourcedRule {
                def: def.clone(),
                origin: origin.clone(),
                index,
            });
        match origin {
            Origin::Builtin => builtin_rules.extend(sourced),
            Origin::File(_) => file_rules.extend(sourced),
            Origin::Inline(_) => inline_rules.extend(sourced),
        }
        if *origin != Origin::Builtin {
            theme = cfg.theme.clone().or(theme);
            defaults = cfg.defaults.or(defaults);
        }
    }

    let theme = opts
        .theme
        .clone()
        .or(theme)
        .unwrap_or_else(|| "dark".to_owned());
    let Some(overlay) = themes.get(&theme) else {
        return Err(ConfigErrors(vec![format!(
            "unknown theme \"{theme}\" (available: {})",
            themes.keys().cloned().collect::<Vec<_>>().join(", ")
        )]));
    };
    palette.extend(overlay.iter().map(|(k, v)| (k.clone(), v.clone())));

    let defaults = defaults.unwrap_or(true);
    let mut rules = inline_rules;
    rules.extend(file_rules);
    if defaults {
        rules.extend(builtin_rules);
    }
    Ok(Merged {
        theme,
        themes: themes.into_keys().collect(),
        palette,
        patterns,
        rules,
        defaults,
        settings,
        file: layers.file().map(PathBuf::from),
    })
}

/// Merge, validate and compile everything. All problems are reported together.
pub fn resolve(layers: &Layers, opts: &ResolveOptions) -> Result<Resolved, ConfigErrors> {
    let merged = merge(layers, opts)?;
    let mut errors = Vec::new();

    let palette = resolve_palette(&merged.palette, &mut errors);

    let color_mode = opts
        .color_mode
        .or(match merged.settings.color_mode {
            Some(ColorModeSetting::Truecolor) => Some(ColorMode::TrueColor),
            Some(ColorModeSetting::Ansi256) => Some(ColorMode::Ansi256),
            Some(ColorModeSetting::Auto) | None => None,
        })
        .unwrap_or_else(ColorMode::detect);

    for name in merged.patterns.keys() {
        if !valid_name(name) {
            errors.push(format!(
                "pattern name \"{name}\" is invalid (use letters, digits, '-' and '_')"
            ));
        }
    }

    let unicode = merged.settings.unicode.unwrap_or(false);
    let enabled: Vec<&SourcedRule> = merged.rules.iter().filter(|r| r.def.enabled).collect();
    let compiled = compile_all(&enabled, |sr| {
        compile_rule(sr, &palette, &merged.patterns, color_mode, unicode)
    });
    let mut rules = Vec::with_capacity(enabled.len());
    for (sr, result) in enabled.iter().zip(compiled) {
        match result {
            Ok(rule) => rules.push(rule),
            Err(e) => errors.push(format!("{}: {e}", sr.label())),
        }
    }

    if !errors.is_empty() {
        return Err(ConfigErrors(errors));
    }
    let read_timeout = Duration::from_millis(
        opts.read_timeout_ms
            .or(merged.settings.read_timeout_ms)
            .unwrap_or(DEFAULT_READ_TIMEOUT_MS),
    );
    let max_line_bytes = merged
        .settings
        .max_line_bytes
        .unwrap_or(crate::stream::DEFAULT_MAX_PENDING);
    Ok(Resolved {
        merged,
        palette,
        rules,
        color_mode,
        read_timeout,
        max_line_bytes,
    })
}

/// Compile rules, in parallel for large rule sets (regex compilation dominates
/// startup for big configs). Results keep the input order.
fn compile_all<F>(rules: &[&SourcedRule], compile: F) -> Vec<Result<Rule, String>>
where
    F: Fn(&SourcedRule) -> Result<Rule, String> + Sync,
{
    const PER_THREAD: usize = 12;
    let threads = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(rules.len() / PER_THREAD)
        .min(16);
    if threads <= 1 {
        return rules.iter().map(|r| compile(r)).collect();
    }
    let mut slots: Vec<Option<Result<Rule, String>>> = Vec::with_capacity(rules.len());
    slots.resize_with(rules.len(), || None);
    std::thread::scope(|scope| {
        let compile = &compile;
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                scope.spawn(move || {
                    // Interleaved assignment spreads expensive rules across threads.
                    (t..rules.len())
                        .step_by(threads)
                        .map(|i| (i, compile(rules[i])))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        for h in handles {
            for (i, r) in h.join().expect("rule compilation thread panicked") {
                slots[i] = Some(r);
            }
        }
    });
    slots
        .into_iter()
        .map(|s| s.expect("every rule compiled"))
        .collect()
}

/// Build a single, self-contained config equivalent to the merged layers
/// (used by `ct config show`).
pub fn effective_config(m: &Merged) -> ConfigFile {
    ConfigFile {
        version: Some(super::SCHEMA_VERSION),
        defaults: Some(false),
        theme: None,
        settings: m.settings.clone(),
        palette: m.palette.clone(),
        themes: BTreeMap::new(),
        patterns: m
            .patterns
            .iter()
            .map(|(k, (v, _))| (k.clone(), v.clone()))
            .collect(),
        rules: m.rules.iter().map(|r| r.def.clone()).collect(),
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Parse a palette value that is a literal color (not an alias).
/// `Ok(None)` means "this is an alias name".
pub fn parse_color_literal(v: &str) -> Result<Option<Color>, String> {
    let v = v.trim();
    if v.starts_with('#') {
        return Color::parse_hex(v)
            .map(Some)
            .ok_or_else(|| format!("invalid hex color \"{v}\" (expected #rrggbb or #rgb)"));
    }
    if let Some(n) = v.strip_prefix("ansi:") {
        return n
            .parse::<u8>()
            .map(|n| {
                Some(if n < 16 {
                    Color::Ansi(n)
                } else {
                    Color::Indexed(n)
                })
            })
            .map_err(|_| format!("invalid ANSI color \"{v}\" (expected ansi:0 … ansi:255)"));
    }
    if v == "default" {
        return Ok(Some(Color::Default));
    }
    Ok(None)
}

fn resolve_palette(
    raw: &BTreeMap<String, String>,
    errors: &mut Vec<String>,
) -> BTreeMap<String, Color> {
    let mut out = BTreeMap::new();
    for name in raw.keys() {
        if !valid_name(name) {
            errors.push(format!(
                "palette name \"{name}\" is invalid (use letters, digits, '-' and '_')"
            ));
            continue;
        }
        let mut cur = name.as_str();
        let mut resolved = None;
        let mut failed = false;
        for _ in 0..MAX_ALIAS_DEPTH {
            let Some(value) = raw.get(cur) else {
                errors.push(format!(
                    "palette \"{name}\": refers to unknown color \"{cur}\""
                ));
                failed = true;
                break;
            };
            match parse_color_literal(value) {
                Ok(Some(c)) => {
                    resolved = Some(c);
                    break;
                }
                Ok(None) => cur = value.trim(),
                Err(e) => {
                    errors.push(format!("palette \"{name}\": {e}"));
                    failed = true;
                    break;
                }
            }
        }
        match resolved {
            Some(c) => {
                out.insert(name.clone(), c);
            }
            None if !failed => {
                errors.push(format!("palette \"{name}\": alias cycle"));
            }
            None => {}
        }
    }
    out
}

/// Parse a color spec such as `"f.error b#202020 bold"` or `"warn underline"`.
pub fn parse_spec(spec: &str, palette: &BTreeMap<String, Color>) -> Result<Style, String> {
    let mut style = Style::default();
    let lookup = |name: &str| -> Result<Color, String> {
        if let Some(c) = parse_color_literal(name)? {
            return Ok(c);
        }
        palette
            .get(name)
            .copied()
            .ok_or_else(|| format!("unknown color \"{name}\" (see `ct colors`)"))
    };
    for tok in spec.split_whitespace() {
        if let Some(bit) = flags::by_name(tok) {
            style.flags |= bit;
        } else if let Some(rest) = tok
            .strip_prefix("f.")
            .or_else(|| tok.strip_prefix("f#").map(|_| &tok[1..]))
        {
            style.fg = Some(lookup(rest)?);
        } else if let Some(rest) = tok
            .strip_prefix("b.")
            .or_else(|| tok.strip_prefix("b#").map(|_| &tok[1..]))
        {
            style.bg = Some(lookup(rest)?);
        } else if tok.starts_with('#') || palette.contains_key(tok) {
            style.fg = Some(lookup(tok)?);
        } else {
            return Err(format!(
                "unknown color or style \"{tok}\" (styles: bold, dim, italic, underline, blink, invert, strike; colors: see `ct colors`)"
            ));
        }
    }
    if style.is_empty() {
        return Err(format!("empty color spec \"{spec}\""));
    }
    Ok(style)
}

/// Expand `${name}` references to named patterns (recursively).
/// `\${…}` (an odd number of preceding backslashes) is left alone.
pub fn expand_patterns(
    src: &str,
    patterns: &BTreeMap<String, (String, Origin)>,
) -> Result<String, String> {
    fn go(
        src: &str,
        patterns: &BTreeMap<String, (String, Origin)>,
        stack: &mut Vec<String>,
    ) -> Result<String, String> {
        if stack.len() > MAX_PATTERN_DEPTH {
            return Err(format!(
                "patterns nested too deeply ({})",
                stack.join(" → ")
            ));
        }
        let mut out = String::with_capacity(src.len());
        let mut rest = src;
        while let Some(i) = rest.find("${") {
            out.push_str(&rest[..i]);
            let after = &rest[i + 2..];
            let escaped = out.bytes().rev().take_while(|&b| b == b'\\').count() % 2 == 1;
            let name = after
                .find('}')
                .map(|j| &after[..j])
                .filter(|n| valid_name(n));
            match name {
                Some(name) if !escaped => {
                    let Some((pat, _)) = patterns.get(name) else {
                        return Err(format!(
                            "unknown pattern \"${{{name}}}\" (see `ct patterns`)"
                        ));
                    };
                    if stack.iter().any(|s| s == name) {
                        return Err(format!("pattern cycle: {} → {name}", stack.join(" → ")));
                    }
                    stack.push(name.to_owned());
                    let inner = go(pat, patterns, stack)?;
                    stack.pop();
                    out.push_str("(?:");
                    out.push_str(&inner);
                    out.push(')');
                    rest = &after[name.len() + 1..];
                }
                _ => {
                    out.push_str("${");
                    rest = after;
                }
            }
        }
        out.push_str(rest);
        Ok(out)
    }
    go(src, patterns, &mut Vec::new())
}

/// The final regex source for a rule (pattern lookup, interpolation, flags).
pub fn rule_regex(
    def: &RuleDef,
    patterns: &BTreeMap<String, (String, Origin)>,
) -> Result<String, String> {
    let src = match (&def.regex, &def.pattern) {
        (Some(_), Some(_)) => return Err("set either `regex` or `pattern`, not both".into()),
        (None, None) => return Err("missing `regex` (or `pattern`)".into()),
        (Some(re), None) => expand_patterns(re, patterns)?,
        (None, Some(name)) => {
            let Some((pat, _)) = patterns.get(name.as_str()) else {
                return Err(format!("unknown pattern \"{name}\" (see `ct patterns`)"));
            };
            expand_patterns(pat, patterns)?
        }
    };
    if src.trim().is_empty() {
        return Err("empty regex".into());
    }
    Ok(if def.ignore_case {
        format!("(?i){src}")
    } else {
        src
    })
}

fn compile_rule(
    sr: &SourcedRule,
    palette: &BTreeMap<String, Color>,
    patterns: &BTreeMap<String, (String, Origin)>,
    mode: ColorMode,
    unicode: bool,
) -> Result<Rule, String> {
    let def = &sr.def;
    let src = rule_regex(def, patterns)?;
    let matcher = Matcher::with_unicode(&src, def.unicode.unwrap_or(unicode))
        .map_err(|e| format!("invalid regex: {e}"))?;
    let styles = match &def.color {
        ColorDef::Spec(spec) => RuleStyles::Whole(parse_spec(spec, palette)?.for_mode(mode)),
        ColorDef::Groups(map) => {
            if map.is_empty() {
                return Err("`color` table is empty".into());
            }
            let groups = matcher.group_count();
            let mut styles = Vec::with_capacity(map.len());
            for (key, spec) in map {
                let idx = match key.parse::<usize>() {
                    Ok(i) => i,
                    Err(_) => matcher
                        .group_index(key)
                        .ok_or_else(|| format!("no capture group named \"{key}\""))?,
                };
                if idx >= groups {
                    return Err(format!(
                        "capture group {idx} does not exist (the regex has {} group{})",
                        groups - 1,
                        if groups == 2 { "" } else { "s" }
                    ));
                }
                let style = parse_spec(spec, palette)
                    .map_err(|e| format!("group {key}: {e}"))?
                    .for_mode(mode);
                styles.push((idx, style));
            }
            styles.sort_by_key(|(i, _)| *i);
            styles.dedup_by_key(|(i, _)| *i);
            match styles.as_slice() {
                [(0, s)] => RuleStyles::Whole(*s),
                _ => RuleStyles::Groups(styles),
            }
        }
    };
    Ok(Rule {
        description: def
            .description
            .clone()
            .unwrap_or_else(|| def.pattern.clone().unwrap_or_else(|| src.clone())),
        matcher,
        styles,
        exclusive: def.exclusive,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Sources, parse_inline};

    fn layers(inline: &[&str]) -> Layers {
        Sources {
            file: None,
            inline: inline.iter().map(|s| s.to_string()).collect(),
            no_config: true,
        }
        .load()
        .unwrap()
    }

    fn opts() -> ResolveOptions {
        ResolveOptions {
            color_mode: Some(ColorMode::TrueColor),
            ..Default::default()
        }
    }

    #[test]
    fn builtin_resolves_cleanly_in_all_themes() {
        for theme in ["dark", "light"] {
            let r = resolve(
                &layers(&[]),
                &ResolveOptions {
                    theme: Some(theme.into()),
                    ..opts()
                },
            )
            .unwrap_or_else(|e| panic!("{theme}: {e}"));
            assert!(r.rules.len() > 20);
        }
    }

    #[test]
    fn palette_aliases_and_errors() {
        let mut raw = BTreeMap::new();
        raw.insert("a".to_string(), "b".to_string());
        raw.insert("b".to_string(), "#010203".to_string());
        raw.insert("c".to_string(), "d".to_string());
        raw.insert("d".to_string(), "c".to_string());
        raw.insert("e".to_string(), "#zz".to_string());
        raw.insert("bad name".to_string(), "#000".to_string());
        // Style names are allowed as palette names (usable as `f.bold`).
        raw.insert("faint".to_string(), "#111".to_string());
        let mut errs = Vec::new();
        let p = resolve_palette(&raw, &mut errs);
        assert_eq!(p["a"], Color::Rgb(1, 2, 3));
        assert!(
            errs.iter().any(|e| e.contains("\"c\": alias cycle")),
            "{errs:?}"
        );
        assert!(errs.iter().any(|e| e.contains("invalid hex")), "{errs:?}");
        assert!(
            errs.iter().any(|e| e.contains("\"bad name\" is invalid")),
            "{errs:?}"
        );
        assert_eq!(p["faint"], Color::Rgb(0x11, 0x11, 0x11));
    }

    #[test]
    fn color_specs() {
        let mut pal = BTreeMap::new();
        pal.insert("error".to_string(), Color::Rgb(255, 0, 0));
        let s = parse_spec("f.error b#000 bold underline", &pal).unwrap();
        assert_eq!(s.fg, Some(Color::Rgb(255, 0, 0)));
        assert_eq!(s.bg, Some(Color::Rgb(0, 0, 0)));
        assert_eq!(s.flags, flags::BOLD | flags::UNDERLINE);
        assert_eq!(
            parse_spec("error", &pal).unwrap().fg,
            Some(Color::Rgb(255, 0, 0))
        );
        assert_eq!(
            parse_spec("#fff", &pal).unwrap().fg,
            Some(Color::Rgb(255, 255, 255))
        );
        assert_eq!(
            parse_spec("b.ansi:3", &pal).unwrap().bg,
            Some(Color::Ansi(3))
        );
        assert!(
            parse_spec("f.nope", &pal)
                .unwrap_err()
                .contains("unknown color")
        );
        assert!(
            parse_spec("sparkly", &pal)
                .unwrap_err()
                .contains("unknown color or style")
        );
        assert!(parse_spec("  ", &pal).unwrap_err().contains("empty"));
    }

    #[test]
    fn pattern_interpolation() {
        let mut p = BTreeMap::new();
        p.insert("oct".to_string(), (r"\d{1,3}".to_string(), Origin::Builtin));
        p.insert(
            "ip".to_string(),
            (r"${oct}(?:\.${oct}){3}".to_string(), Origin::Builtin),
        );
        p.insert("loop".to_string(), ("${loop}".to_string(), Origin::Builtin));
        assert_eq!(
            expand_patterns("src=${ip}", &p).unwrap(),
            r"src=(?:(?:\d{1,3})(?:\.(?:\d{1,3})){3})"
        );
        assert_eq!(expand_patterns(r"a\${ip}", &p).unwrap(), r"a\${ip}");
        assert_eq!(expand_patterns(r"x${1,2}", &p).unwrap(), r"x${1,2}");
        assert!(
            expand_patterns("${nope}", &p)
                .unwrap_err()
                .contains("unknown pattern")
        );
        assert!(
            expand_patterns("${loop}", &p)
                .unwrap_err()
                .contains("cycle")
        );
    }

    #[test]
    fn rule_errors_are_aggregated_with_labels() {
        let l = layers(&[r#"{"defaults": false, "rules": [
            {"description": "bad re", "regex": "(", "color": "red"},
            {"regex": "x", "color": "f.nope"},
            {"regex": "(a)", "color": {"2": "red"}},
            {"regex": "a", "pattern": "ipv4", "color": "red"},
            {"color": "red"}
        ]}"#]);
        let e = resolve(&l, &opts()).unwrap_err();
        let msg = e.to_string();
        assert_eq!(e.0.len(), 5, "{msg}");
        assert!(
            msg.contains("rule #1 \"bad re\" (inline config #1): invalid regex"),
            "{msg}"
        );
        assert!(
            msg.contains("rule #2 (inline config #1): unknown color \"nope\""),
            "{msg}"
        );
        assert!(msg.contains("capture group 2 does not exist"), "{msg}");
        assert!(msg.contains("not both"), "{msg}");
        assert!(msg.contains("missing `regex`"), "{msg}");
    }

    #[test]
    fn layering_order_and_defaults() {
        let l = layers(&[
            r#"rules = [{ regex = 'a', color = 'red' }]"#,
            r##"{"rules": [{"regex": "b", "color": "red"}], "palette": {"red": "#110000"}}"##,
        ]);
        let m = merge(&l, &opts()).unwrap();
        assert_eq!(m.rules[0].def.regex.as_deref(), Some("a"));
        assert_eq!(m.rules[1].def.regex.as_deref(), Some("b"));
        assert!(m.defaults && m.rules.len() > 2);
        assert_eq!(m.palette["red"], "#110000");

        let l = layers(&[r#"{"defaults": false, "rules": [{"regex": "b", "color": "red"}]}"#]);
        assert_eq!(merge(&l, &opts()).unwrap().rules.len(), 1);
    }

    #[test]
    fn themes_override_palette() {
        let l = layers(&[r##"{"theme": "mine", "themes": {"mine": {"error": "#123456"}}}"##]);
        let r = resolve(&l, &opts()).unwrap();
        assert_eq!(r.palette["error"], Color::Rgb(0x12, 0x34, 0x56));
        let e = resolve(
            &l,
            &ResolveOptions {
                theme: Some("nope".into()),
                ..opts()
            },
        )
        .unwrap_err();
        assert!(e.to_string().contains("unknown theme \"nope\""));
    }

    #[test]
    fn named_group_colors_and_ignore_case() {
        let cfg = parse_inline(
            r#"{"defaults": false, "rules": [{"regex": "(?P<k>\\w+)=(\\w+)", "color": {"k": "red", "2": "blue"}, "ignore_case": true}]}"#,
        )
        .unwrap();
        let l = Layers(vec![
            (Origin::Builtin, crate::config::builtin()),
            (Origin::Inline(0), cfg),
        ]);
        let r = resolve(&l, &opts()).unwrap();
        assert!(r.rules[0].matcher.as_str().starts_with("(?i)"));
        match &r.rules[0].styles {
            RuleStyles::Groups(g) => assert_eq!(g.iter().map(|x| x.0).collect::<Vec<_>>(), [1, 2]),
            other => panic!("{other:?}"),
        }
    }
}
