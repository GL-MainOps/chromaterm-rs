//! Import of Python ChromaTerm YAML configs (`palette:` + `rules:`).
//!
//! Uses yaml-rust2's event API with a small tree builder. Anchors/aliases are
//! supported for **scalars only** (as in typical palette files). Aliases to
//! sequences or mappings are rejected, which rules out exponential
//! "billion laughs" expansion. The number of nodes is also capped.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;

use yaml_rust2::parser::{Event, MarkedEventReceiver, Parser};
use yaml_rust2::scanner::Marker;

use super::{ColorDef, ConfigFile, RuleDef, SCHEMA_VERSION};

const MAX_NODES: usize = 500_000;

#[derive(Debug, Clone)]
enum Node {
    Scalar(String),
    Seq(Vec<Node>),
    Map(Vec<(Node, Node)>),
}

enum Frame {
    Seq(Vec<Node>),
    Map(Vec<(Node, Node)>, Option<Node>),
}

#[derive(Default)]
struct Builder {
    stack: Vec<Frame>,
    scalar_anchors: HashMap<usize, String>,
    collection_anchors: HashSet<usize>,
    root: Option<Node>,
    nodes: usize,
    error: Option<String>,
}

impl Builder {
    fn fail(&mut self, msg: String) {
        self.error.get_or_insert(msg);
    }

    fn push(&mut self, node: Node) {
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            self.fail(format!("document has more than {MAX_NODES} nodes"));
            return;
        }
        match self.stack.last_mut() {
            None => self.root = Some(node),
            Some(Frame::Seq(items)) => items.push(node),
            Some(Frame::Map(pairs, key)) => match key.take() {
                None => *key = Some(node),
                Some(k) => pairs.push((k, node)),
            },
        }
    }
}

impl MarkedEventReceiver for Builder {
    fn on_event(&mut self, ev: Event, mark: Marker) {
        if self.error.is_some() {
            return;
        }
        match ev {
            Event::Scalar(value, _style, anchor, _tag) => {
                if anchor > 0 {
                    self.scalar_anchors.insert(anchor, value.clone());
                }
                self.push(Node::Scalar(value));
            }
            Event::Alias(id) => match self.scalar_anchors.get(&id).cloned() {
                Some(value) => self.push(Node::Scalar(value)),
                None if self.collection_anchors.contains(&id) => self.fail(format!(
                    "line {}: aliases to sequences/mappings are not supported",
                    mark.line()
                )),
                None => self.fail(format!("line {}: unknown alias", mark.line())),
            },
            Event::SequenceStart(anchor, _) => {
                if anchor > 0 {
                    self.collection_anchors.insert(anchor);
                }
                self.stack.push(Frame::Seq(Vec::new()));
            }
            Event::MappingStart(anchor, _) => {
                if anchor > 0 {
                    self.collection_anchors.insert(anchor);
                }
                self.stack.push(Frame::Map(Vec::new(), None));
            }
            Event::SequenceEnd => {
                if let Some(Frame::Seq(items)) = self.stack.pop() {
                    self.push(Node::Seq(items));
                }
            }
            Event::MappingEnd => {
                if let Some(Frame::Map(pairs, _)) = self.stack.pop() {
                    self.push(Node::Map(pairs));
                }
            }
            _ => {}
        }
    }
}

/// A converted legacy document, keeping the original palette order.
#[derive(Debug, Clone, Default)]
pub struct LegacyDoc {
    pub palette: Vec<(String, String)>,
    pub rules: Vec<RuleDef>,
    pub warnings: Vec<String>,
}

fn scalar(node: &Node, what: &str) -> Result<String, String> {
    match node {
        Node::Scalar(value) => Ok(value.clone()),
        _ => Err(format!("{what}: expected a string")),
    }
}

/// Parse a legacy YAML document.
pub fn import(text: &str) -> Result<LegacyDoc, String> {
    let mut b = Builder::default();
    Parser::new_from_str(text)
        .load(&mut b, false)
        .map_err(|e| format!("YAML: {e}"))?;
    if let Some(e) = b.error {
        return Err(format!("YAML: {e}"));
    }
    let mut doc = LegacyDoc::default();
    let pairs = match b.root {
        None => return Ok(doc),
        Some(Node::Map(p)) => p,
        Some(_) => return Err("legacy config: top level must be a mapping".into()),
    };
    for (k, v) in pairs {
        match scalar(&k, "top-level key")?.as_str() {
            "palette" => {
                let Node::Map(entries) = v else {
                    return Err("palette: expected a mapping".into());
                };
                for (name, color) in entries {
                    let name = scalar(&name, "palette key")?;
                    let color = scalar(&color, &format!("palette.{name}"))?;
                    if let Some(slot) = doc.palette.iter_mut().find(|(n, _)| *n == name) {
                        doc.warnings
                            .push(format!("palette: duplicate key \"{name}\" (last one wins)"));
                        slot.1 = color;
                    } else {
                        doc.palette.push((name, color));
                    }
                }
            }
            "rules" => {
                let Node::Seq(items) = v else {
                    return Err("rules: expected a list".into());
                };
                for (i, item) in items.into_iter().enumerate() {
                    doc.rules
                        .push(convert_rule(item).map_err(|e| format!("rules[{i}]: {e}"))?);
                }
            }
            other => doc
                .warnings
                .push(format!("ignoring unknown top-level key \"{other}\"")),
        }
    }
    Ok(doc)
}

fn convert_rule(node: Node) -> Result<RuleDef, String> {
    let Node::Map(pairs) = node else {
        return Err("expected a mapping".into());
    };
    let mut rule = RuleDef {
        description: None,
        regex: None,
        pattern: None,
        color: ColorDef::Spec(String::new()),
        exclusive: false,
        ignore_case: false,
        unicode: None,
        enabled: true,
    };
    let mut has_color = false;
    for (k, v) in pairs {
        match scalar(&k, "rule key")?.as_str() {
            "description" => rule.description = Some(scalar(&v, "description")?),
            "regex" => rule.regex = Some(python_regex_compat(&scalar(&v, "regex")?)),
            "exclusive" => {
                rule.exclusive = match scalar(&v, "exclusive")?.to_ascii_lowercase().as_str() {
                    "true" | "yes" | "on" => true,
                    "false" | "no" | "off" | "" => false,
                    other => {
                        return Err(format!("exclusive: expected true/false, got \"{other}\""));
                    }
                }
            }
            "color" => {
                has_color = true;
                rule.color = match v {
                    Node::Scalar(value) => ColorDef::Spec(value),
                    Node::Map(groups) => {
                        let mut map = BTreeMap::new();
                        for (g, spec) in groups {
                            map.insert(scalar(&g, "color group")?, scalar(&spec, "color")?);
                        }
                        ColorDef::Groups(map)
                    }
                    Node::Seq(_) => return Err("color: expected a string or mapping".into()),
                };
            }
            other => return Err(format!("unknown key \"{other}\"")),
        }
    }
    if rule.regex.is_none() {
        return Err("missing `regex`".into());
    }
    if !has_color {
        return Err("missing `color`".into());
    }
    Ok(rule)
}

/// Minimal Python `re` → Rust syntax adjustments.
fn python_regex_compat(re: &str) -> String {
    // Python's `\Z` (absolute end) is `\z` in Rust.
    let mut out = String::with_capacity(re.len());
    let mut chars = re.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('Z') => out.push_str("\\z"),
                Some(n) => {
                    out.push('\\');
                    out.push(n);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Parse legacy YAML directly into a [`ConfigFile`] (for loading `.yml` configs).
pub fn parse(text: &str) -> Result<ConfigFile, String> {
    Ok(import(text)?.to_config())
}

impl LegacyDoc {
    pub fn to_config(&self) -> ConfigFile {
        ConfigFile {
            version: Some(SCHEMA_VERSION),
            // Python ChromaTerm configs fully define their rule set.
            defaults: Some(false),
            settings: super::Settings {
                unicode: Some(false),
                ..Default::default()
            },
            palette: self.palette.iter().cloned().collect(),
            rules: self.rules.clone(),
            ..ConfigFile::default()
        }
    }

    /// Render as a readable, commented TOML document.
    pub fn to_toml(&self, source: &str) -> String {
        let mut s = String::new();
        let _ = writeln!(
            s,
            "# Converted from Python ChromaTerm config: {source}\n\
             # by `ct config import` (chromaterm-rs {}).\n\
             #\n\
             # Legacy configs define the complete rule set, so built-in default\n\
             # rules are disabled. Set `defaults = true` to append them.\n\
             #\n\
             # `unicode = false` makes \\w \\d \\s \\b ASCII-only: much faster startup\n\
             # and matching. Python's `re` is Unicode-aware. Set `unicode = true`\n\
             # for identical semantics (slower, uses more memory for patterns like\n\
             # `\\w{{1,63}}`).\n",
            env!("CARGO_PKG_VERSION")
        );
        for w in &self.warnings {
            let _ = writeln!(s, "# NOTE: {w}");
        }
        let _ = writeln!(
            s,
            "version = {SCHEMA_VERSION}\ndefaults = false\n\n[settings]\nunicode = false\n"
        );
        if !self.palette.is_empty() {
            s.push_str("[palette]\n");
            let width = self
                .palette
                .iter()
                .map(|(k, _)| key(k).len())
                .max()
                .unwrap_or(0);
            for (k, v) in &self.palette {
                let _ = writeln!(s, "{:width$} = {}", key(k), basic(v));
            }
            s.push('\n');
        }
        for r in &self.rules {
            s.push_str("[[rules]]\n");
            if let Some(d) = &r.description {
                let _ = writeln!(s, "description = {}", basic(d));
            }
            if let Some(re) = &r.regex {
                let _ = writeln!(s, "regex = {}", literal(re));
            }
            match &r.color {
                ColorDef::Spec(c) => {
                    let _ = writeln!(s, "color = {}", basic(c));
                }
                ColorDef::Groups(g) => {
                    let mut items: Vec<_> = g.iter().collect();
                    items.sort_by_key(|(k, _)| (k.parse::<usize>().unwrap_or(usize::MAX), *k));
                    let body = items
                        .iter()
                        .map(|(k, v)| format!("{} = {}", key(k), basic(v)))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let _ = writeln!(s, "color = {{ {body} }}");
                }
            }
            if r.exclusive {
                s.push_str("exclusive = true\n");
            }
            s.push('\n');
        }
        s
    }
}

fn key(k: &str) -> String {
    if !k.is_empty()
        && k.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        k.to_owned()
    } else {
        basic(k)
    }
}

/// A TOML basic (double-quoted, escaped) string.
fn basic(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04X}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Prefer TOML literal strings for regexes (no backslash escaping).
fn literal(s: &str) -> String {
    let bad_control = |c: char| c.is_control() && c != '\t' && c != '\n';
    if s.chars().any(bad_control) {
        return basic(s);
    }
    if !s.contains('\'') && !s.contains('\n') {
        return format!("'{s}'");
    }
    if !s.contains("'''") && !s.ends_with("''") {
        // A newline right after the opening delimiter is trimmed by TOML.
        return format!("'''\n{s}'''");
    }
    basic(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Format, parse as parse_doc};

    const SAMPLE: &str = r#"
palette:
  base: &base '#112233'
  alias: *base
  dup: '#000000'
  dup: '#ffffff'
rules:
  - description: Numbers
    regex: \b\d+\Z
    color: f.base bold
    exclusive: true
  - description: Groups
    regex: |
      (?x) (\w+)
      =(\d+) 'q'
    color:
      1: f.alias
      2: b#abcdef
"#;

    #[test]
    fn imports_and_round_trips() {
        let doc = import(SAMPLE).unwrap();
        assert_eq!(doc.palette.len(), 3);
        assert_eq!(doc.palette[1], ("alias".into(), "#112233".into()));
        assert_eq!(doc.palette[2].1, "#ffffff");
        assert!(doc.warnings[0].contains("duplicate"));
        assert_eq!(doc.rules[0].regex.as_deref(), Some(r"\b\d+\z"));
        assert!(doc.rules[0].exclusive);
        let toml = doc.to_toml("sample.yml");
        let reparsed = parse_doc(&toml, Format::Toml).unwrap();
        assert_eq!(reparsed, doc.to_config(), "{toml}");
    }

    #[test]
    fn rejects_collection_aliases() {
        let bomb = "a: &a [1, 2]\nb: [*a, *a]\n";
        assert!(import(bomb).unwrap_err().contains("not supported"));
    }

    #[test]
    fn rejects_bad_rules() {
        assert!(
            import("rules:\n  - color: red\n")
                .unwrap_err()
                .contains("missing `regex`")
        );
        assert!(
            import("rules:\n  - regex: x\n    colour: red\n")
                .unwrap_err()
                .contains("unknown key")
        );
    }

    #[test]
    fn literal_strings() {
        assert_eq!(literal(r"\d+"), r"'\d+'");
        assert_eq!(literal("it's"), "'''\nit's'''");
        assert_eq!(literal("a'''b"), r#""a'''b""#);
    }
}
