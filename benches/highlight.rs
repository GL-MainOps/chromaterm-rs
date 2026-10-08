//! Throughput benchmarks: `cargo bench --bench highlight`.
//!
//! Set `CT_BENCH_CONFIG=/path/to/config` to also benchmark your own config.

use std::hint::black_box;

use chromaterm::color::ColorMode;
use chromaterm::config::resolve::ResolveOptions;
use chromaterm::config::{self, Sources};
use chromaterm::engine::Highlighter;
use chromaterm::stream::Stream;
use criterion::{Criterion, Throughput, criterion_group, criterion_main};

const LINES: &[&str] = &[
    "Oct  9 00:28:01 web-01 sshd[1234]: Failed password for root from 10.0.0.5 port 22 ssh2",
    "10.1.2.3 - - [01/May/2024:13:37:00 +0000] \"GET /api/v1/users?id=42 HTTP/1.1\" 200 512 \"-\" \"curl/8.5.0\"",
    "{\"level\":\"INFO\",\"ts\":\"2024-05-01T13:37:00.123Z\",\"msg\":\"request done\",\"duration\":\"15ms\",\"ok\":true}",
    "pod/api-7d9f8c6b5-x2kqz   1/1   Running   0   3d4h   10.244.1.17   node-2   <none>",
    "2024-05-01 13:37:00,123 WARNING [worker-3] retrying connection to db.internal:5432 (attempt 3/5)",
    "plain text line without anything special in it, just words and more words here",
    "commit 5b8cb16a2f0e9d1c3b4a5f6e7d8c9b0a1f2e3d4c",
    "/usr/local/bin/ct: 3.1M  -rwxr-xr-x  2024-05-01  sha256:9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08",
    "\x1b[32mok\x1b[0m: \x1b[1mtest_parse\x1b[0m ... passed in 0.25s (fe80::1%eth0, 2001:db8::/32)",
    "ERROR: connection refused (errno=111) at 0x7ffd5e8a1c40 — retry in 5s ✓ héllo wörld",
];

fn corpus(n: usize) -> Vec<u8> {
    let mut v = Vec::new();
    for i in 0..n {
        v.extend_from_slice(LINES[i % LINES.len()].as_bytes());
        v.push(b'\n');
    }
    v
}

fn highlighter(sources: Sources) -> Highlighter {
    let layers = sources.load().expect("config");
    let opts = ResolveOptions {
        color_mode: Some(ColorMode::TrueColor),
        ..Default::default()
    };
    config::resolve(&layers, &opts)
        .expect("config")
        .into_highlighter()
}

fn bench_stream(c: &mut Criterion, name: &str, sources: Sources) {
    let data = corpus(5_000);
    let mut group = c.benchmark_group(name);
    group.throughput(Throughput::Bytes(data.len() as u64));
    group.sample_size(20);
    let mut stream = Stream::new(highlighter(sources));
    group.bench_function("stream_5k_lines", |b| {
        b.iter(|| {
            for chunk in data.chunks(64 * 1024) {
                stream.feed(black_box(chunk));
                stream.clear_output();
            }
            stream.finish();
            stream.clear_output();
        })
    });
    group.finish();
}

fn benches(c: &mut Criterion) {
    bench_stream(
        c,
        "builtin",
        Sources {
            no_config: true,
            ..Default::default()
        },
    );
    bench_stream(
        c,
        "no_rules",
        Sources {
            inline: vec!["defaults = false".into()],
            no_config: true,
            ..Default::default()
        },
    );
    if let Some(path) = std::env::var_os("CT_BENCH_CONFIG") {
        bench_stream(
            c,
            "custom_config",
            Sources {
                file: Some(path.into()),
                ..Default::default()
            },
        );
    }

    c.bench_function("startup/compile_builtin", |b| {
        b.iter(|| {
            highlighter(Sources {
                no_config: true,
                ..Default::default()
            })
        })
    });
}

criterion_group!(highlight, benches);
criterion_main!(highlight);
