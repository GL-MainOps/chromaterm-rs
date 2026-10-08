//! Configuration: schema, discovery, layering and validation.
//!
//! Configs are layered (lowest → highest precedence):
//!
//! 1. **built-in** (`assets/builtin.toml`): palette, themes, named patterns, default rules
//! 2. **file**: `--config` / `$CHROMATERM_CONFIG` / the first discovered path
//! 3. **inline**: each `-i/--inline` value (JSON or TOML), in order
//!
//! Maps (`palette`, `themes`, `patterns`) merge key by key. Rules from inline
//! layers come first, then file rules, then the built-in rules if `defaults`
//! is true (the default).

pub mod export;
pub mod resolve;

#[cfg(feature = "legacy-yaml")]
pub mod legacy;

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

pub use resolve::{Resolved, resolve};

/// The built-in base layer (palette, themes, patterns, default rules).
pub const BUILTIN_TOML: &str = include_str!("../../assets/builtin.toml");
/// Template written by `ct config init`.
pub const TEMPLATE_TOML: &str = include_str!("../../assets/template.toml");

/// Highest config schema version this build understands.
pub const SCHEMA_VERSION: u32 = 1;
/// Refuse to read config files larger than this (sanity / DoS guard).
pub const MAX_CONFIG_BYTES: u64 = 4 * 1024 * 1024;

fn is_false(b: &bool) -> bool {
    !*b
}
fn is_true(b: &bool) -> bool {
    *b
}
fn default_true() -> bool {
    true
}

/// One configuration document (file or inline).
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigFile {
    /// Schema version (currently 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
    /// Append the built-in default rules after this config's rules.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defaults: Option<bool>,
    /// Active theme name (a key of `themes`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(default, skip_serializing_if = "Settings::is_empty")]
    pub settings: Settings,
    /// Named colors: `name = "#rrggbb" | "#rgb" | "ansi:N" | "<other name>"`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub palette: BTreeMap<String, String>,
    /// Theme overlays: `themes.<name>.<palette key> = <color>`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub themes: BTreeMap<String, BTreeMap<String, String>>,
    /// Named regex fragments, usable as `pattern = "name"` or `${name}` in a regex.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub patterns: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<RuleDef>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// How long to wait for the rest of a partial line before flushing it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_timeout_ms: Option<u64>,
    /// `auto`, `truecolor` or `256`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_mode: Option<ColorModeSetting>,
    /// Longest partial line held back before it is flushed in pieces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_line_bytes: Option<usize>,
    /// Unicode-aware `\w \d \s \b` in rule regexes (default false: ASCII, faster).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unicode: Option<bool>,
}

impl Settings {
    pub fn is_empty(&self) -> bool {
        *self == Settings::default()
    }

    fn merge(&mut self, other: &Settings) {
        if other.read_timeout_ms.is_some() {
            self.read_timeout_ms = other.read_timeout_ms;
        }
        if other.color_mode.is_some() {
            self.color_mode = other.color_mode;
        }
        if other.max_line_bytes.is_some() {
            self.max_line_bytes = other.max_line_bytes;
        }
        if other.unicode.is_some() {
            self.unicode = other.unicode;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorModeSetting {
    Auto,
    Truecolor,
    #[serde(rename = "256")]
    Ansi256,
}

/// A highlighting rule as written in a config file.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuleDef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Regular expression (mutually exclusive with `pattern`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regex: Option<String>,
    /// Name of a built-in or user pattern (mutually exclusive with `regex`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    pub color: ColorDef,
    /// Matches of this rule cannot be highlighted by later rules.
    #[serde(default, skip_serializing_if = "is_false")]
    pub exclusive: bool,
    /// Case-insensitive matching (same as a leading `(?i)`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub ignore_case: bool,
    /// Per-rule override of `settings.unicode`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unicode: Option<bool>,
    /// Set to false to keep a rule in the file but switch it off.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub enabled: bool,
}

/// `color = "f.red bold"` or `color = { 1 = "f.key", value = "f.string" }`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ColorDef {
    Spec(String),
    Groups(BTreeMap<String, String>),
}

impl<'de> Deserialize<'de> for ColorDef {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = ColorDef;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str(
                    "a color spec string (e.g. \"f.error bold\") or a table mapping \
                     capture groups to color specs (e.g. { 1 = \"f.key\" })",
                )
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<ColorDef, E> {
                Ok(ColorDef::Spec(v.to_owned()))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<ColorDef, A::Error> {
                let mut groups = BTreeMap::new();
                while let Some((k, v)) = map.next_entry::<GroupKey, String>()? {
                    groups.insert(k.0, v);
                }
                Ok(ColorDef::Groups(groups))
            }
        }
        d.deserialize_any(V)
    }
}

/// Group keys may be written as strings ("1", "name") or integers (JSON/YAML).
struct GroupKey(String);

impl<'de> Deserialize<'de> for GroupKey {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = GroupKey;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a capture group index or name")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<GroupKey, E> {
                Ok(GroupKey(v.to_owned()))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<GroupKey, E> {
                Ok(GroupKey(v.to_string()))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<GroupKey, E> {
                Ok(GroupKey(v.to_string()))
            }
        }
        d.deserialize_any(V)
    }
}

/// Where a config layer came from (for messages).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    Builtin,
    File(PathBuf),
    Inline(usize),
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Origin::Builtin => f.write_str("built-in defaults"),
            Origin::File(p) => write!(f, "{}", p.display()),
            Origin::Inline(i) => write!(f, "inline config #{}", i + 1),
        }
    }
}

/// A list of human-readable problems found while loading or validating.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigErrors(pub Vec<String>);

impl fmt::Display for ConfigErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let n = self.0.len();
        writeln!(
            f,
            "invalid configuration ({n} problem{}):",
            if n == 1 { "" } else { "s" }
        )?;
        for e in &self.0 {
            writeln!(f, "  - {e}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ConfigErrors {}

/// Supported document formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Toml,
    Json,
    LegacyYaml,
}

impl Format {
    pub fn from_path(path: &Path) -> Format {
        match path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("json") => Format::Json,
            Some("yml" | "yaml") => Format::LegacyYaml,
            _ => Format::Toml,
        }
    }
}

/// Parse a config document.
pub fn parse(text: &str, format: Format) -> Result<ConfigFile, String> {
    let cfg: ConfigFile = match format {
        Format::Toml => toml::from_str(text).map_err(|e| e.to_string().trim_end().to_owned())?,
        Format::Json => serde_json::from_str(text).map_err(|e| e.to_string())?,
        Format::LegacyYaml => {
            #[cfg(feature = "legacy-yaml")]
            {
                legacy::parse(text)?
            }
            #[cfg(not(feature = "legacy-yaml"))]
            {
                return Err(
                    "legacy YAML configs are not supported by this build (feature `legacy-yaml`)"
                        .into(),
                );
            }
        }
    };
    if let Some(v) = cfg.version {
        if v == 0 || v > SCHEMA_VERSION {
            return Err(format!(
                "unsupported config version {v} (this build supports version {SCHEMA_VERSION})"
            ));
        }
    }
    Ok(cfg)
}

/// Parse an inline config: JSON if it starts with `{`, TOML otherwise.
pub fn parse_inline(text: &str) -> Result<ConfigFile, String> {
    if text.trim_start().starts_with('{') {
        parse(text, Format::Json)
    } else {
        parse(text, Format::Toml)
    }
}

/// Read and parse a config file (size-limited).
pub fn load_file(path: &Path) -> Result<ConfigFile, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.len() > MAX_CONFIG_BYTES {
        return Err(format!(
            "{}: file is larger than {} MiB",
            path.display(),
            MAX_CONFIG_BYTES >> 20
        ));
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse(&text, Format::from_path(path)).map_err(|e| format!("{}: {e}", path.display()))
}

/// The parsed built-in layer (panics only if the embedded asset is broken,
/// which is covered by tests).
pub fn builtin() -> ConfigFile {
    parse(BUILTIN_TOML, Format::Toml).expect("embedded builtin.toml must be valid")
}

/// Default config search paths, highest priority first.
pub fn search_paths() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let xdg = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| home.as_ref().map(|h| h.join(".config")));
    let mut paths = Vec::new();
    if let Some(x) = &xdg {
        paths.push(x.join("chromaterm/config.toml"));
        paths.push(x.join("chromaterm/config.json"));
    }
    if let Some(h) = &home {
        paths.push(h.join(".chromaterm.toml"));
        paths.push(h.join(".chromaterm.json"));
    }
    if cfg!(feature = "legacy-yaml") {
        if let Some(x) = &xdg {
            paths.push(x.join("chromaterm/chromaterm.yml"));
        }
        if let Some(h) = &home {
            paths.push(h.join(".chromaterm.yml"));
            paths.push(h.join(".chromaterm.yaml"));
        }
    }
    paths.push(PathBuf::from("/etc/chromaterm/config.toml"));
    if cfg!(feature = "legacy-yaml") {
        paths.push(PathBuf::from("/etc/chromaterm/chromaterm.yml"));
    }
    paths
}

/// The default path used by `ct config init`.
pub fn default_init_path() -> Option<PathBuf> {
    search_paths().into_iter().next()
}

/// Inputs that determine which configuration is loaded.
#[derive(Debug, Clone, Default)]
pub struct Sources {
    /// Explicit file (`--config` or `$CHROMATERM_CONFIG`).
    pub file: Option<PathBuf>,
    /// Inline documents (`-i`), in order.
    pub inline: Vec<String>,
    /// Skip config files entirely.
    pub no_config: bool,
}

/// Parsed layers, lowest precedence first (built-in is always first).
#[derive(Debug, Clone)]
pub struct Layers(pub Vec<(Origin, ConfigFile)>);

impl Sources {
    /// Locate and parse every layer.
    pub fn load(&self) -> Result<Layers, ConfigErrors> {
        let mut layers = vec![(Origin::Builtin, builtin())];
        let mut errors = Vec::new();
        if !self.no_config {
            let path = match &self.file {
                Some(p) => Some(p.clone()),
                None => search_paths().into_iter().find(|p| p.is_file()),
            };
            if let Some(path) = path {
                match load_file(&path) {
                    Ok(cfg) => layers.push((Origin::File(path), cfg)),
                    Err(e) => errors.push(e),
                }
            }
        }
        for (i, text) in self.inline.iter().enumerate() {
            match parse_inline(text) {
                Ok(cfg) => layers.push((Origin::Inline(i), cfg)),
                Err(e) => errors.push(format!("{}: {e}", Origin::Inline(i))),
            }
        }
        if errors.is_empty() {
            Ok(Layers(layers))
        } else {
            Err(ConfigErrors(errors))
        }
    }
}

impl Layers {
    /// Combine the user's layers (file + inline, no built-ins) into one
    /// document that behaves the same when loaded on its own.
    pub fn user_config(&self) -> ConfigFile {
        let mut out = ConfigFile::default();
        let (mut inline_rules, mut file_rules) = (Vec::new(), Vec::new());
        for (origin, cfg) in &self.0 {
            match origin {
                Origin::Builtin => continue,
                Origin::File(_) => file_rules.extend(cfg.rules.iter().cloned()),
                Origin::Inline(_) => inline_rules.extend(cfg.rules.iter().cloned()),
            }
            out.version = cfg.version.or(out.version);
            out.defaults = cfg.defaults.or(out.defaults);
            out.theme = cfg.theme.clone().or(out.theme.take());
            out.settings.merge(&cfg.settings);
            out.palette
                .extend(cfg.palette.iter().map(|(k, v)| (k.clone(), v.clone())));
            out.patterns
                .extend(cfg.patterns.iter().map(|(k, v)| (k.clone(), v.clone())));
            for (name, overlay) in &cfg.themes {
                out.themes
                    .entry(name.clone())
                    .or_default()
                    .extend(overlay.iter().map(|(k, v)| (k.clone(), v.clone())));
            }
        }
        out.version.get_or_insert(SCHEMA_VERSION);
        inline_rules.extend(file_rules);
        out.rules = inline_rules;
        out
    }

    /// The config file layer in use, if any.
    pub fn file(&self) -> Option<&Path> {
        self.0.iter().find_map(|(o, _)| match o {
            Origin::File(p) => Some(p.as_path()),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_parses() {
        let b = builtin();
        assert!(!b.rules.is_empty());
        assert!(b.palette.contains_key("error"));
        assert!(b.patterns.contains_key("ipv4"));
        assert!(b.themes.contains_key("light"));
    }

    #[test]
    fn template_parses() {
        parse(TEMPLATE_TOML, Format::Toml).unwrap();
    }

    #[test]
    fn inline_json_and_toml() {
        let j = parse_inline(r#"{"rules":[{"regex":"\\d+","color":"f.number"}]}"#).unwrap();
        assert_eq!(j.rules[0].regex.as_deref(), Some(r"\d+"));
        let t = parse_inline(r#"rules = [{ regex = '\d+', color = { 0 = "f.number" } }]"#).unwrap();
        assert_eq!(t.rules[0].regex.as_deref(), Some(r"\d+"));
        assert!(matches!(t.rules[0].color, ColorDef::Groups(_)));
        let j = parse_inline(r#"{"rules":[{"regex":"x","color":{"1":"red"}}]}"#).unwrap();
        assert!(matches!(j.rules[0].color, ColorDef::Groups(_)));
    }

    #[test]
    fn rejects_unknown_fields_and_versions() {
        let e = parse_inline(r#"{"rulez":[]}"#).unwrap_err();
        assert!(e.contains("unknown field"), "{e}");
        let e = parse_inline(r#"rules = [{ regex = "x", colour = "red" }]"#).unwrap_err();
        assert!(e.contains("colour"), "{e}");
        let e = parse_inline("version = 99").unwrap_err();
        assert!(e.contains("unsupported config version 99"), "{e}");
        let e = parse_inline(r#"rules = [{ regex = "x", color = 5 }]"#).unwrap_err();
        assert!(e.contains("color spec"), "{e}");
    }

    #[test]
    fn format_from_extension() {
        assert_eq!(Format::from_path(Path::new("a.json")), Format::Json);
        assert_eq!(Format::from_path(Path::new("a.YML")), Format::LegacyYaml);
        assert_eq!(Format::from_path(Path::new("a.toml")), Format::Toml);
        assert_eq!(Format::from_path(Path::new("noext")), Format::Toml);
    }
}
