//! chromaterm-rs: regex-based terminal output highlighter (library facade).
//!
//! The binary (`ct`) lives in `main.rs`. This crate exposes the engine for
//! tests, benchmarks and embedding.
//!
//! Pipeline: raw bytes → [`stream::Stream`] (line framing) →
//! [`engine::Highlighter`] (strip escapes → match rules → render SGR diffs).

pub mod ansi;
pub mod cli;
pub mod color;
pub mod config;
pub mod engine;
pub mod instances;
pub mod io;
pub mod pty;
pub mod signals;
pub mod stream;

/// Build a highlighter from inline config documents on top of the built-in
/// layer (no config files). Convenient for tests and embedding.
pub fn highlighter_from_inline(
    inline: &[&str],
    mode: color::ColorMode,
) -> Result<engine::Highlighter, config::ConfigErrors> {
    let layers = config::Sources {
        file: None,
        inline: inline.iter().map(|s| s.to_string()).collect(),
        no_config: true,
    }
    .load()?;
    let opts = config::resolve::ResolveOptions {
        color_mode: Some(mode),
        ..Default::default()
    };
    Ok(config::resolve(&layers, &opts)?.into_highlighter())
}
