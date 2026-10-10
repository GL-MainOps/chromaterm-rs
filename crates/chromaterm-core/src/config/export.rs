//! Export to Python ChromaTerm's YAML format.
//!
//! The legacy format has no themes, patterns, aliases, settings or `dim`. The
//! export is therefore self-contained: the active theme is applied, palette
//! colors become hex, `pattern`/`${…}` and `ignore_case` are expanded into
//! the regex, and named capture groups become indexes. Everything that cannot
//! be expressed is reported as a note.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use super::resolve::{Merged, resolve_palette, rule_regex};
use super::{ColorDef, ConfigErrors};
use crate::color::{Color, flags};
use crate::engine::Matcher;

/// Python ChromaTerm's styles.
const LEGACY_STYLES: &[&str] = &["blink", "bold", "invert", "italic", "strike", "underline"];

fn hex(c: Color) -> Option<String> {
    c.to_rgb()
        .map(|(r, g, b)| format!("#{r:02x}{g:02x}{b:02x}"))
}

/// A YAML scalar: single-quoted (backslashes stay literal, good for regexes),
/// or a double-quoted JSON string when it contains newlines/control chars.
fn yaml_str(s: &str) -> String {
    if s.chars().any(|c| c.is_control()) {
        serde_json::to_string(s).expect("string serialization cannot fail")
    } else {
        format!("'{}'", s.replace('\'', "''"))
    }
}

struct SpecConverter<'a> {
    palette: &'a BTreeMap<String, Color>,
    used: BTreeMap<String, String>,
    notes: Vec<String>,
}

impl SpecConverter<'_> {
    /// Rewrite one color spec in legacy syntax (`f.name`, `b#rrggbb`, styles).
    fn convert(&mut self, spec: &str, label: &str) -> String {
        let mut out = Vec::new();
        for tok in spec.split_whitespace() {
            if let Some(bit) = flags::by_name(tok) {
                let name = flags::TABLE
                    .iter()
                    .find(|t| t.1 == bit)
                    .map_or(tok, |t| t.0);
                if LEGACY_STYLES.contains(&name) {
                    out.push(name.to_owned());
                } else {
                    self.notes.push(format!(
                        "{label}: style \"{tok}\" is not supported; dropped"
                    ));
                }
                continue;
            }
            // "f.name" / "b.name", "f#hex" / "b#hex", or a bare name / "#hex" (fg).
            let (prefix, name) = if let Some(n) = tok.strip_prefix("f.") {
                ("f", n)
            } else if let Some(n) = tok.strip_prefix("b.") {
                ("b", n)
            } else if tok.starts_with("f#") || tok.starts_with("b#") {
                (&tok[..1], &tok[1..])
            } else {
                ("f", tok)
            };
            let color = match super::resolve::parse_color_literal(name) {
                Ok(Some(c)) => Some(c),
                _ => self.palette.get(name).copied(),
            };
            match color.and_then(hex) {
                Some(h) if name.starts_with('#') || name.starts_with("ansi:") => {
                    out.push(format!("{prefix}{h}"))
                }
                Some(h) => {
                    self.used.insert(name.to_owned(), h);
                    out.push(format!("{prefix}.{name}"));
                }
                None => self.notes.push(format!(
                    "{label}: color \"{tok}\" (terminal default) has no hex form; dropped"
                )),
            }
        }
        out.join(" ")
    }
}

/// Render the merged configuration as a Python ChromaTerm YAML document.
/// Returns the document and notes about lossy conversions.
pub fn to_legacy_yaml(merged: &Merged) -> Result<(String, Vec<String>), ConfigErrors> {
    let mut errors = Vec::new();
    let palette = resolve_palette(&merged.palette, &mut errors);
    let mut conv = SpecConverter {
        palette: &palette,
        used: BTreeMap::new(),
        notes: Vec::new(),
    };
    let mut rules = String::new();
    for sr in merged.rules.iter().filter(|r| r.def.enabled) {
        let label = sr.label();
        let regex = match rule_regex(&sr.def, &merged.patterns) {
            Ok(r) => r,
            Err(e) => {
                errors.push(format!("{label}: {e}"));
                continue;
            }
        };
        let _ = writeln!(
            rules,
            "  - description: {}",
            yaml_str(sr.def.description.as_deref().unwrap_or(""))
        );
        let _ = writeln!(rules, "    regex: {}", yaml_str(&regex));
        match &sr.def.color {
            ColorDef::Spec(spec) => {
                let _ = writeln!(
                    rules,
                    "    color: {}",
                    yaml_str(&conv.convert(spec, &label))
                );
            }
            ColorDef::Groups(groups) => {
                let matcher = Matcher::new(&regex).ok();
                let mut items = Vec::new();
                for (key, spec) in groups {
                    let idx = key
                        .parse::<usize>()
                        .ok()
                        .or_else(|| matcher.as_ref().and_then(|m| m.group_index(key)));
                    match idx {
                        Some(i) => items.push((i, conv.convert(spec, &label))),
                        None => errors.push(format!("{label}: no capture group named \"{key}\"")),
                    }
                }
                items.sort();
                let _ = writeln!(rules, "    color:");
                for (i, spec) in items {
                    let _ = writeln!(rules, "      {i}: {}", yaml_str(&spec));
                }
            }
        }
        if sr.def.exclusive {
            let _ = writeln!(rules, "    exclusive: true");
        }
    }
    if !errors.is_empty() {
        return Err(ConfigErrors(errors));
    }
    let mut out = format!(
        "# Exported by `ct config export --format yaml` (chromaterm-rs {}) for\n\
         # Python ChromaTerm. Self-contained: theme \"{}\" applied, patterns expanded.\n\
         # Note: Python's `re` treats \\w \\d \\s \\b as Unicode-aware.\n",
        env!("CARGO_PKG_VERSION"),
        merged.theme
    );
    out.push_str("palette:\n");
    for (name, hex) in &conv.used {
        let _ = writeln!(out, "  {}: {}", yaml_str(name), yaml_str(hex));
    }
    out.push_str("rules:\n");
    out.push_str(&rules);
    Ok((out, conv.notes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaml_strings() {
        assert_eq!(yaml_str(r"\d+ it's"), r"'\d+ it''s'");
        assert_eq!(yaml_str("a\nb"), "\"a\\nb\"");
    }

    #[test]
    fn spec_conversion() {
        let mut pal = BTreeMap::new();
        pal.insert("error".to_string(), Color::Rgb(255, 0, 0));
        let mut c = SpecConverter {
            palette: &pal,
            used: BTreeMap::new(),
            notes: Vec::new(),
        };
        assert_eq!(c.convert("error bold reverse", "r"), "f.error bold invert");
        assert_eq!(c.convert("b#fff dim f.ansi:1", "r"), "b#ffffff f#cd0000");
        assert_eq!(c.used["error"], "#ff0000");
        assert!(c.notes[0].contains("dim"));
        assert_eq!(c.convert("f.default", "r"), "");
    }
}
