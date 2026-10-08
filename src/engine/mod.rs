//! The highlighting engine: compiled rules → spans → rendered bytes.

pub mod matcher;

use std::time::Instant;

use memchr::memchr;

use crate::ansi::{self, Attrs, ESC, Escape};
use crate::color::Style;
pub use matcher::{Engine, Matcher};

/// How a rule colors its matches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuleStyles {
    /// Color the entire match.
    Whole(Style),
    /// Color individual capture groups `(group index, style)`, ascending.
    Groups(Vec<(usize, Style)>),
}

/// A compiled highlighting rule.
#[derive(Debug)]
pub struct Rule {
    pub description: String,
    pub matcher: Matcher,
    pub styles: RuleStyles,
    pub exclusive: bool,
}

impl Rule {
    fn slots(&self) -> usize {
        match &self.styles {
            RuleStyles::Whole(_) => 1,
            RuleStyles::Groups(g) => g.last().map_or(1, |(idx, _)| idx + 1),
        }
    }
}

/// Per-rule counters collected when benchmarking is enabled.
#[derive(Clone, Copy, Debug, Default)]
pub struct RuleStats {
    pub matches: u64,
    pub nanos: u64,
    pub limit_hits: u64,
}

/// One highlighted region of the stripped text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub style: Style,
    pub rule: u32,
}

#[derive(Clone, Copy, Debug)]
struct EscRef {
    /// Position in the stripped text where the sequence sits.
    pos: usize,
    /// Byte range in the raw line.
    start: usize,
    end: usize,
    sgr: bool,
}

/// Applies rules to lines of terminal output.
///
/// The highlighter is stateful: it tracks the program's own SGR state across
/// lines. When a highlight ends, it restores exactly what the program had
/// set, never a blanket reset.
/// One linear pass over a line that tells which fast rules can match at all.
/// Rules that cannot match are skipped.
#[derive(Debug)]
struct Prefilter {
    set: regex::bytes::RegexSet,
    /// Set index for each rule (`None` = always run, e.g. fancy rules).
    index: Vec<Option<usize>>,
    hits: Vec<bool>,
}

impl Prefilter {
    fn build(rules: &[Rule]) -> Option<Self> {
        let mut patterns = Vec::new();
        let index = rules
            .iter()
            .map(|r| {
                r.matcher.set_source().map(|p| {
                    patterns.push(p);
                    patterns.len() - 1
                })
            })
            .collect();
        if patterns.len() < 2 {
            return None;
        }
        let set = regex::bytes::RegexSetBuilder::new(&patterns)
            .size_limit(64 << 20)
            .build()
            .ok()?;
        Some(Prefilter {
            hits: vec![false; patterns.len()],
            set,
            index,
        })
    }
}

#[derive(Debug)]
pub struct Highlighter {
    rules: Vec<Rule>,
    prefilter: Option<Prefilter>,
    state: Attrs,
    stats: Option<Vec<RuleStats>>,
    // Scratch buffers, reused across lines (steady state is allocation-free).
    text: Vec<u8>,
    escapes: Vec<EscRef>,
    found: Vec<(usize, usize)>,
    spans: Vec<Span>,
    claimed: Vec<(usize, usize)>,
    bounds: Vec<usize>,
    by_start: Vec<u32>,
    by_end: Vec<u32>,
    active: Vec<u32>,
}

impl Highlighter {
    pub fn new(rules: Vec<Rule>) -> Self {
        Highlighter {
            prefilter: Prefilter::build(&rules),
            rules,
            state: Attrs::default(),
            stats: None,
            text: Vec::new(),
            escapes: Vec::new(),
            found: Vec::new(),
            spans: Vec::new(),
            claimed: Vec::new(),
            bounds: Vec::new(),
            by_start: Vec::new(),
            by_end: Vec::new(),
            active: Vec::new(),
        }
    }

    /// Continue from `old` after a config reload: keep the program's tracked
    /// color state (and benchmarking, if it was on).
    pub fn inherit_from(&mut self, old: &Highlighter) {
        self.state = old.state;
        if old.stats.is_some() && self.stats.is_none() {
            self.enable_stats();
        }
    }

    /// Enable per-rule timing and match counting.
    pub fn enable_stats(&mut self) {
        self.stats = Some(vec![RuleStats::default(); self.rules.len()]);
    }

    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    pub fn stats(&self) -> Option<&[RuleStats]> {
        self.stats.as_deref()
    }

    /// Highlight one line (without its terminator) and append the result to `out`.
    pub fn highlight_line(&mut self, line: &[u8], out: &mut Vec<u8>) {
        if self.rules.is_empty() {
            out.extend_from_slice(line);
            self.track_state_only(line);
            return;
        }
        let mut text = std::mem::take(&mut self.text);
        let stripped = self.strip(line, &mut text);
        let hay: &[u8] = if stripped { &text } else { line };
        self.collect_spans(hay);
        if self.spans.is_empty() {
            out.extend_from_slice(line);
            self.apply_escapes_state(line);
        } else {
            self.render(line, hay, out);
        }
        self.text = text;
    }

    /// Return the spans the rules produce for `line` (for `ct explain`).
    pub fn explain(&mut self, line: &[u8]) -> (Vec<u8>, Vec<Span>) {
        let mut text = Vec::new();
        let hay = if self.strip(line, &mut text) {
            text
        } else {
            line.to_vec()
        };
        self.collect_spans(&hay);
        (hay, self.spans.clone())
    }

    /// Split `line` into stripped text and escape references. Return false
    /// (and leave `text` untouched) if the line contains no escape sequences.
    fn strip(&mut self, line: &[u8], text: &mut Vec<u8>) -> bool {
        self.escapes.clear();
        if memchr(ESC, line).is_none() {
            return false;
        }
        text.clear();
        let mut i = 0;
        while let Some(off) = memchr(ESC, &line[i..]) {
            let at = i + off;
            text.extend_from_slice(&line[i..at]);
            let (end, sgr) = match ansi::scan(line, at) {
                Escape::Complete { end, sgr } => (end, sgr),
                // Unterminated: treat the rest of the line as part of the sequence.
                Escape::Incomplete => (line.len(), false),
            };
            self.escapes.push(EscRef {
                pos: text.len(),
                start: at,
                end,
                sgr,
            });
            i = end;
        }
        text.extend_from_slice(&line[i..]);
        true
    }

    fn collect_spans(&mut self, hay: &[u8]) {
        self.spans.clear();
        self.claimed.clear();
        if let Some(pf) = self.prefilter.as_mut() {
            pf.hits.fill(false);
            pf.set.matches_read_at(&mut pf.hits, hay, 0);
        }
        for (ri, rule) in self.rules.iter_mut().enumerate() {
            if let Some(pf) = &self.prefilter {
                if pf.index[ri].is_some_and(|i| !pf.hits[i]) {
                    continue;
                }
            }
            let slots = rule.slots();
            self.found.clear();
            let started = self.stats.as_ref().map(|_| Instant::now());
            let limited = rule.matcher.find_all(hay, slots, &mut self.found).is_err();
            let mut kept = 0u64;
            for m in self.found.chunks_exact(slots) {
                let (s, e) = m[0];
                if !is_boundary(hay, s) || !is_boundary(hay, e) {
                    continue;
                }
                let idx = self.claimed.partition_point(|&(_, ce)| ce <= s);
                if idx < self.claimed.len() && self.claimed[idx].0 < e {
                    continue; // overlaps an exclusive match from an earlier rule
                }
                if rule.exclusive {
                    self.claimed.insert(idx, (s, e));
                }
                kept += 1;
                let rule_idx = ri as u32;
                match &rule.styles {
                    RuleStyles::Whole(style) => self.spans.push(Span {
                        start: s,
                        end: e,
                        style: *style,
                        rule: rule_idx,
                    }),
                    RuleStyles::Groups(groups) => {
                        for &(g, style) in groups {
                            let (gs, ge) = m[g];
                            if gs != usize::MAX
                                && gs < ge
                                && is_boundary(hay, gs)
                                && is_boundary(hay, ge)
                            {
                                self.spans.push(Span {
                                    start: gs,
                                    end: ge,
                                    style,
                                    rule: rule_idx,
                                });
                            }
                        }
                    }
                }
            }
            if let (Some(stats), Some(t0)) = (self.stats.as_mut(), started) {
                let s = &mut stats[ri];
                s.nanos += t0.elapsed().as_nanos() as u64;
                s.matches += kept;
                s.limit_hits += limited as u64;
            }
        }
    }

    /// Render `hay` (the stripped text) with spans, re-inserting the original
    /// escape sequences from `line`.
    fn render(&mut self, line: &[u8], hay: &[u8], out: &mut Vec<u8>) {
        let n = self.spans.len();
        self.bounds.clear();
        self.bounds.push(0);
        self.bounds.push(hay.len());
        for s in &self.spans {
            self.bounds.push(s.start);
            self.bounds.push(s.end);
        }
        self.bounds.extend(self.escapes.iter().map(|e| e.pos));
        self.bounds.sort_unstable();
        self.bounds.dedup();

        self.by_start.clear();
        self.by_start.extend(0..n as u32);
        let spans = &self.spans;
        self.by_start.sort_by_key(|&i| (spans[i as usize].start, i));
        self.by_end.clear();
        self.by_end.extend(0..n as u32);
        self.by_end.sort_by_key(|&i| (spans[i as usize].end, i));
        self.active.clear();

        let mut term = self.state;
        let (mut si, mut ei, mut xi) = (0, 0, 0);
        for k in 0..self.bounds.len() {
            let p = self.bounds[k];
            // 1. Spans ending here stop applying.
            while xi < n && self.spans[self.by_end[xi] as usize].end <= p {
                let id = self.by_end[xi];
                if let Ok(at) = self.active.binary_search(&id) {
                    self.active.remove(at);
                }
                xi += 1;
            }
            // 2. Original escapes at this position, emitted under the styling of
            //    spans that continue across it (so e.g. `ESC[K` doesn't paint
            //    with a highlight that already ended).
            if ei < self.escapes.len() && self.escapes[ei].pos == p {
                let desired = self.desired();
                term.write_transition(&desired, out);
                term = desired;
                while ei < self.escapes.len() && self.escapes[ei].pos == p {
                    let esc = self.escapes[ei];
                    out.extend_from_slice(&line[esc.start..esc.end]);
                    if esc.sgr {
                        let params = &line[esc.start + 2..esc.end - 1];
                        self.state.apply_sgr(params);
                        term.apply_sgr(params);
                    }
                    ei += 1;
                }
            }
            // 3. Spans starting here begin applying.
            while si < n && self.spans[self.by_start[si] as usize].start <= p {
                let id = self.by_start[si];
                let at = self.active.binary_search(&id).unwrap_or_else(|e| e);
                self.active.insert(at, id);
                si += 1;
            }
            let Some(&q) = self.bounds.get(k + 1) else {
                break;
            };
            let desired = self.desired();
            term.write_transition(&desired, out);
            term = desired;
            out.extend_from_slice(&hay[p..q]);
        }
        term.write_transition(&self.state, out);
    }

    /// Program state overlaid with active spans in rule order (later wins).
    fn desired(&self) -> Attrs {
        let mut d = self.state;
        for &id in &self.active {
            let st = self.spans[id as usize].style;
            if let Some(fg) = st.fg {
                d.fg = fg;
            }
            if let Some(bg) = st.bg {
                d.bg = bg;
            }
            d.flags |= st.flags;
        }
        d
    }

    /// Account for an escape sequence that was emitted without highlighting
    /// (e.g. one split across a forced flush).
    pub fn observe_escape(&mut self, seq: &[u8]) {
        if let Escape::Complete { end, sgr: true } = ansi::scan(seq, 0) {
            self.state.apply_sgr(&seq[2..end - 1]);
        }
    }

    fn apply_escapes_state(&mut self, line: &[u8]) {
        for esc in &self.escapes {
            if esc.sgr {
                self.state.apply_sgr(&line[esc.start + 2..esc.end - 1]);
            }
        }
    }

    fn track_state_only(&mut self, line: &[u8]) {
        let mut scratch = std::mem::take(&mut self.text);
        if self.strip(line, &mut scratch) {
            self.apply_escapes_state(line);
        }
        self.text = scratch;
    }
}

#[inline]
fn is_boundary(text: &[u8], i: usize) -> bool {
    i >= text.len() || (text[i] as i8) >= -0x40 // not a UTF-8 continuation byte
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::{Color, flags};

    fn rule(re: &str, style: Style, exclusive: bool) -> Rule {
        Rule {
            description: re.into(),
            matcher: Matcher::new(re).unwrap(),
            styles: RuleStyles::Whole(style),
            exclusive,
        }
    }
    fn fg(n: u8) -> Style {
        Style {
            fg: Some(Color::Ansi(n)),
            ..Style::default()
        }
    }
    fn run(h: &mut Highlighter, line: &str) -> String {
        let mut out = Vec::new();
        h.highlight_line(line.as_bytes(), &mut out);
        String::from_utf8(out).unwrap().replace('\x1b', "E")
    }

    #[test]
    fn simple_highlight_and_restore() {
        let mut h = Highlighter::new(vec![rule(r"\d+", fg(1), false)]);
        assert_eq!(run(&mut h, "a 12 b"), "a E[31m12E[39m b");
        assert_eq!(run(&mut h, "none"), "none");
    }

    #[test]
    fn exclusive_blocks_later_overlaps() {
        let mut h = Highlighter::new(vec![
            rule(r"\d+\.\d+\.\d+\.\d+", fg(2), true),
            rule(r"\d+", fg(1), false),
        ]);
        assert_eq!(run(&mut h, "10.0.0.1 7"), "E[32m10.0.0.1E[39m E[31m7E[39m");
    }

    #[test]
    fn later_non_exclusive_overrides_per_attribute() {
        let bold = Style {
            flags: flags::BOLD,
            ..Style::default()
        };
        let mut h = Highlighter::new(vec![
            rule(r"x\d+x", fg(1), false),
            rule(r"\d+", bold, false),
        ]);
        // Outer keeps fg red; inner adds bold and then only bold is removed.
        assert_eq!(run(&mut h, "x12x"), "E[31mxE[1m12E[22mxE[39m");
        let mut h = Highlighter::new(vec![
            rule(r"x\d+x", fg(1), false),
            rule(r"\d+", fg(2), false),
        ]);
        assert_eq!(run(&mut h, "x12x"), "E[31mxE[32m12E[31mxE[39m");
    }

    #[test]
    fn group_styles() {
        let mut h = Highlighter::new(vec![Rule {
            description: "kv".into(),
            matcher: Matcher::new(r"(\w+)=(\w+)").unwrap(),
            styles: RuleStyles::Groups(vec![(1, fg(3)), (2, fg(4))]),
            exclusive: false,
        }]);
        assert_eq!(run(&mut h, "k=v"), "E[33mkE[39m=E[34mvE[39m");
    }

    #[test]
    fn preserves_and_restores_program_colors() {
        let mut h = Highlighter::new(vec![rule("ERR", fg(1), false)]);
        // Program sets green; the highlight inside restores green, not default.
        assert_eq!(
            run(&mut h, "\x1b[32mok ERR ok\x1b[0m"),
            "E[32mok E[31mERRE[32m ok\x1b[0m".replace('\x1b', "E")
        );
        // Escapes inside a match are kept and the match still applies.
        assert_eq!(run(&mut h, "E\x1b[1mRR"), "E[31mEE[1mRRE[39m");
    }

    #[test]
    fn program_state_carries_across_lines() {
        let mut h = Highlighter::new(vec![rule("x", fg(1), false)]);
        assert_eq!(run(&mut h, "\x1b[34ma"), "E[34ma");
        assert_eq!(run(&mut h, "x"), "E[31mxE[34m");
        assert_eq!(run(&mut h, "\x1b[0mx"), "E[0mE[31mxE[39m");
    }

    #[test]
    fn non_sgr_escape_after_span_is_emitted_unstyled() {
        let mut h = Highlighter::new(vec![rule("ab", fg(1), false)]);
        assert_eq!(run(&mut h, "ab\x1b[K"), "E[31mabE[39mE[K");
    }

    #[test]
    fn never_splits_utf8() {
        let mut h = Highlighter::new(vec![rule(r"(?-u:\xA9)", fg(1), false)]);
        assert_eq!(run(&mut h, "é©"), "é©");
    }

    #[test]
    fn prefilter_never_changes_output() {
        let lines = [
            "Oct  9 00:28:01 web sshd[12]: Failed password from 10.0.0.5 port 22",
            "{\"level\": \"INFO\", \"ts\": \"2024-05-01T13:37:00Z\", \"ok\": true}",
            "GET https://x.io/a?b=1 HTTP/1.1\" 404 512 0.25s ✓ héllo",
            "fe80::1%eth0 2001:db8::/32 aa:bb:cc:dd:ee:ff v1.2.3 4.0K /usr/bin",
            "\x1b[32mok\x1b[0m nothing \x1b[1mERROR\x1b[0m pod/web-1",
        ];
        let mode = crate::color::ColorMode::TrueColor;
        let mut with = crate::highlighter_from_inline(&[], mode).unwrap();
        let mut without = crate::highlighter_from_inline(&[], mode).unwrap();
        assert!(with.prefilter.is_some());
        without.prefilter = None;
        for line in lines {
            let (mut a, mut b) = (Vec::new(), Vec::new());
            with.highlight_line(line.as_bytes(), &mut a);
            without.highlight_line(line.as_bytes(), &mut b);
            assert_eq!(a, b, "{line}");
        }
    }

    #[test]
    fn stats_are_counted() {
        let mut h = Highlighter::new(vec![rule(r"\d", fg(1), false)]);
        h.enable_stats();
        run(&mut h, "1 2 3");
        assert_eq!(h.stats().unwrap()[0].matches, 3);
    }
}
