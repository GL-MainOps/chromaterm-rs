//! Regex matcher abstraction.
//!
//! Every pattern is first compiled with the linear-time `regex` crate (finite
//! automata, no catastrophic backtracking). Only patterns that need features it
//! deliberately lacks (look-around, back-references, atomic groups) fall back
//! to `fancy-regex`. That engine runs the backtracking VM with a hard step limit,
//! behind a regular "seek" pre-filter.

use regex::bytes::{CaptureLocations, Regex, RegexBuilder};

/// Placeholder slot for a capture group that did not participate in a match.
pub const NO_MATCH: (usize, usize) = (usize::MAX, usize::MAX);

/// Upper bound for a compiled automaton (guards against pathological patterns).
const SIZE_LIMIT: usize = 64 * (1 << 20);
/// Max backtracking steps per search for fancy patterns (ReDoS guard).
const BACKTRACK_LIMIT: usize = 1_000_000;

/// A backtracking search exceeded its step limit (the pattern is too
/// expensive for this input). Matches found before the limit are kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BacktrackLimit;

/// Which engine a pattern compiled to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    /// Linear-time `regex` crate.
    Fast,
    /// Backtracking `fancy-regex` (look-around / back-references).
    Fancy,
}

pub enum Matcher {
    Fast {
        re: Regex,
        locs: CaptureLocations,
        unicode: bool,
    },
    Fancy(fancy_regex::Regex),
}

impl std::fmt::Debug for Matcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Matcher::{:?}({:?})", self.engine(), self.as_str())
    }
}

impl Matcher {
    /// Compile `pattern` with Unicode-aware classes (`\w`, `\d`, `\b`, `.`).
    pub fn new(pattern: &str) -> Result<Self, String> {
        Self::with_unicode(pattern, true)
    }

    /// Compile `pattern`, preferring the linear-time engine.
    ///
    /// With `unicode == false`, `\w`, `\d`, `\s` and `\b` are ASCII-only.
    /// This keeps the fast DFA engine running on lines that contain
    /// non-ASCII text. Unicode word boundaries force it onto a slower engine.
    pub fn with_unicode(pattern: &str, unicode: bool) -> Result<Self, String> {
        match RegexBuilder::new(pattern)
            .unicode(unicode)
            .size_limit(SIZE_LIMIT)
            .build()
        {
            Ok(re) => {
                let locs = re.capture_locations();
                Ok(Matcher::Fast { re, locs, unicode })
            }
            Err(fast_err) => {
                if matches!(fast_err, regex::Error::CompiledTooBig(_)) {
                    return Err(fast_err.to_string());
                }
                fancy_regex::RegexBuilder::new(pattern)
                    .bytes_mode(if unicode {
                        fancy_regex::BytesMode::UnicodeBytes
                    } else {
                        fancy_regex::BytesMode::Ascii
                    })
                    .backtrack_limit(BACKTRACK_LIMIT)
                    .delegate_size_limit(SIZE_LIMIT)
                    .seek(true)
                    .build()
                    .map(Matcher::Fancy)
                    .map_err(|e| e.to_string())
            }
        }
    }

    /// Pattern text usable inside a combined `RegexSet` (fast engine only),
    /// with this matcher's Unicode mode pinned by an inline flag group.
    pub fn set_source(&self) -> Option<String> {
        match self {
            Matcher::Fast { re, unicode, .. } => Some(format!(
                "(?{}u:{})",
                if *unicode { "" } else { "-" },
                re.as_str()
            )),
            Matcher::Fancy(_) => None,
        }
    }

    pub fn engine(&self) -> Engine {
        match self {
            Matcher::Fast { .. } => Engine::Fast,
            Matcher::Fancy(_) => Engine::Fancy,
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Matcher::Fast { re, .. } => re.as_str(),
            Matcher::Fancy(re) => re.as_str(),
        }
    }

    /// Number of capture slots including the implicit group 0.
    pub fn group_count(&self) -> usize {
        match self {
            Matcher::Fast { re, .. } => re.captures_len(),
            Matcher::Fancy(re) => re.captures_len(),
        }
    }

    /// Index of a named capture group.
    pub fn group_index(&self, name: &str) -> Option<usize> {
        match self {
            Matcher::Fast { re, .. } => re.capture_names().position(|n| n == Some(name)),
            Matcher::Fancy(re) => re.capture_names().position(|n| n == Some(name)),
        }
    }

    /// Find all non-empty, non-overlapping matches in `hay`. Push `slots`
    /// entries per match to `out`: slot `i` is the span of group `i`, or
    /// [`NO_MATCH`]. With `slots == 1` only whole-match spans are produced
    /// (the cheapest path).
    ///
    /// Returns `Err` if a backtracking search hit its step limit. Matches found
    /// up to that point remain in `out`.
    pub fn find_all(
        &mut self,
        hay: &[u8],
        slots: usize,
        out: &mut Vec<(usize, usize)>,
    ) -> Result<(), BacktrackLimit> {
        match self {
            Matcher::Fast { re, .. } if slots <= 1 => {
                out.extend(
                    re.find_iter(hay)
                        .filter(|m| !m.is_empty())
                        .map(|m| (m.start(), m.end())),
                );
                Ok(())
            }
            Matcher::Fast { re, locs, .. } => {
                let mut pos = 0;
                while pos <= hay.len() {
                    let Some(m) = re.captures_read_at(locs, hay, pos) else {
                        break;
                    };
                    if m.is_empty() {
                        pos = m.end() + 1;
                        continue;
                    }
                    out.extend((0..slots).map(|g| locs.get(g).unwrap_or(NO_MATCH)));
                    pos = m.end();
                }
                Ok(())
            }
            Matcher::Fancy(re) if slots <= 1 => {
                for m in re.find_iter(hay) {
                    let m = m.map_err(|_| BacktrackLimit)?;
                    if m.start() < m.end() {
                        out.push((m.start(), m.end()));
                    }
                }
                Ok(())
            }
            Matcher::Fancy(re) => {
                for caps in re.captures_iter(hay) {
                    let caps = caps.map_err(|_| BacktrackLimit)?;
                    let whole = caps.get(0).map(|m| (m.start(), m.end()));
                    if whole.is_none_or(|(s, e)| s >= e) {
                        continue;
                    }
                    out.extend(
                        (0..slots).map(|g| caps.get(g).map_or(NO_MATCH, |m| (m.start(), m.end()))),
                    );
                }
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(m: &mut Matcher, hay: &str, slots: usize) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        m.find_all(hay.as_bytes(), slots, &mut out).unwrap();
        out
    }

    #[test]
    fn prefers_fast_engine() {
        assert_eq!(Matcher::new(r"\d+").unwrap().engine(), Engine::Fast);
        assert_eq!(
            Matcher::new(r"(?<![\w-])ro(?![\w-])").unwrap().engine(),
            Engine::Fancy
        );
        assert_eq!(Matcher::new(r"(a)\1").unwrap().engine(), Engine::Fancy);
        assert!(Matcher::new(r"(unclosed").is_err());
    }

    #[test]
    fn whole_and_group_matches() {
        let mut m = Matcher::new(r"(\w+)=(\d+)?").unwrap();
        assert_eq!(all(&mut m, "a=1 b=", 1), vec![(0, 3), (4, 6)]);
        assert_eq!(
            all(&mut m, "a=1 b=", 3),
            vec![(0, 3), (0, 1), (2, 3), (4, 6), (4, 5), NO_MATCH]
        );
    }

    #[test]
    fn skips_empty_matches() {
        let mut m = Matcher::new(r"\d*").unwrap();
        assert_eq!(all(&mut m, "a12b3", 1), vec![(1, 3), (4, 5)]);
        assert_eq!(
            all(&mut m, "a12b3", 1),
            all(&mut m, "a12b3", 2)
                .into_iter()
                .step_by(2)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn fancy_matches_bytes_and_groups() {
        let mut m = Matcher::new(r"(?<![\w-])(ro|rw)(?![\w-])").unwrap();
        assert_eq!(all(&mut m, "x-ro ro,rw rwx", 1), vec![(5, 7), (8, 10)]);
        assert_eq!(all(&mut m, "ro", 2), vec![(0, 2), (0, 2)]);
        // Invalid UTF-8 must not break matching.
        let mut out = Vec::new();
        m.find_all(b"\xff ro", 1, &mut out).unwrap();
        assert_eq!(out, vec![(2, 4)]);
    }

    #[test]
    fn ascii_mode() {
        let mut m = Matcher::with_unicode(r"\b\w+\b", false).unwrap();
        assert_eq!(all(&mut m, "héllo ok", 1), vec![(0, 1), (3, 6), (7, 9)]);
        let mut m = Matcher::with_unicode(r"\b\w+\b", true).unwrap();
        assert_eq!(all(&mut m, "héllo ok", 1), vec![(0, 6), (7, 9)]);
        let mut m = Matcher::with_unicode(r"(?<!x)\d+", false).unwrap();
        assert_eq!(m.engine(), Engine::Fancy);
        assert_eq!(all(&mut m, "x1 é2", 1), vec![(5, 6)]);
    }

    #[test]
    fn named_groups() {
        let m = Matcher::new(r"(?P<key>\w+)=(?P<val>\w+)").unwrap();
        assert_eq!(m.group_index("key"), Some(1));
        assert_eq!(m.group_index("val"), Some(2));
        assert_eq!(m.group_index("nope"), None);
    }
}
