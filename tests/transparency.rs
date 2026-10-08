//! Property-style tests: highlighting must never alter the text itself.
//!
//! For any input (random bytes, escape sequences, UTF-8, invalid UTF-8, any
//! chunking), removing all SGR sequences from the output must give the input
//! with its own SGR sequences removed.

mod common;

use chromaterm::color::ColorMode;
use chromaterm::stream::Stream;
use common::strip_sgr;

/// Small deterministic PRNG (xorshift) so the test needs no extra deps.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const PIECES: &[&[u8]] = &[
    b"ERROR ",
    b"10.0.0.1",
    b" 42 ",
    b"2024-05-01T10:00:00Z",
    b"https://example.com/x?y=1 ",
    b"\"quoted value\" ",
    b"key=value ",
    b"\x1b[1;31m",
    b"\x1b[0m",
    b"\x1b[38;2;1;2;3m",
    b"\x1b[K",
    b"\x1b]0;title\x07",
    b"\x1b]8;;http://x\x1b\\link\x1b]8;;\x1b\\",
    "héllo wörld ✓ ".as_bytes(),
    b"\xff\xfe",
    b"\n",
    b"\r\n",
    b"\r",
    b"\t",
    b"/usr/local/bin ",
    b"sshd[123]: ",
    b"fe80::1 ",
    b"true null ",
];

fn random_input(rng: &mut Rng) -> Vec<u8> {
    let mut v = Vec::new();
    for _ in 0..rng.below(40) + 1 {
        if rng.below(10) == 0 {
            v.push(rng.next() as u8); // raw random byte
        } else {
            v.extend_from_slice(PIECES[rng.below(PIECES.len())]);
        }
    }
    v
}

fn run_chunked(stream: &mut Stream, input: &[u8], rng: &mut Rng) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < input.len() {
        let n = (rng.below(16) + 1).min(input.len() - i);
        stream.feed(&input[i..i + n]);
        if rng.below(4) == 0 {
            stream.flush_partial(rng.below(2) == 0);
        }
        out.extend_from_slice(stream.output());
        stream.clear_output();
        i += n;
    }
    stream.finish();
    out.extend_from_slice(stream.output());
    out
}

#[test]
fn output_text_equals_input_text() {
    let mut rng = Rng(0x9e3779b97f4a7c15);
    for mode in [ColorMode::TrueColor, ColorMode::Ansi256] {
        let mut hl = chromaterm::highlighter_from_inline(&[], mode).unwrap();
        for case in 0..400 {
            let mut stream = Stream::with_max_pending(hl, 64 + rng.below(512));
            let input = random_input(&mut rng);
            let out = run_chunked(&mut stream, &input, &mut rng);
            hl = stream.into_highlighter();
            assert_eq!(
                strip_sgr(&out),
                strip_sgr(&input),
                "case {case}: input {:?}",
                String::from_utf8_lossy(&input)
            );
        }
    }
}

#[test]
fn never_inserts_inside_escape_sequences() {
    // Every ESC in the output must start a sequence that also appears in the
    // input, or be one of our SGR sequences: check by tokenizing the output.
    let mut rng = Rng(42);
    let mut hl = chromaterm::highlighter_from_inline(&[], ColorMode::TrueColor).unwrap();
    for _ in 0..300 {
        let mut stream = Stream::new(hl);
        let input = random_input(&mut rng);
        let out = run_chunked(&mut stream, &input, &mut rng);
        hl = stream.into_highlighter();
        let non_sgr = |b: &[u8]| -> Vec<Vec<u8>> {
            let mut seqs = Vec::new();
            let mut i = 0;
            while let Some(off) = memchr::memchr(0x1b, &b[i..]) {
                let at = i + off;
                match chromaterm::ansi::scan(b, at) {
                    chromaterm::ansi::Escape::Complete { end, sgr } => {
                        if !sgr {
                            seqs.push(b[at..end].to_vec());
                        }
                        i = end;
                    }
                    chromaterm::ansi::Escape::Incomplete => break,
                }
            }
            seqs
        };
        assert_eq!(
            non_sgr(&out),
            non_sgr(&input),
            "{:?}",
            String::from_utf8_lossy(&input)
        );
    }
}
