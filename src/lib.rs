//! chromaterm-rs: regex-based terminal output highlighter (library facade).
//!
//! The binary (`ct`) lives in `main.rs`. The engine, config and color code live in the
//! `chromaterm-core` crate and are re-exported here under their old paths, so tests,
//! benchmarks and embedders keep working.
//!
//! Pipeline: raw bytes → [`stream::Stream`] (line framing) →
//! [`engine::Highlighter`] (strip escapes → match rules → render SGR diffs).

pub use chromaterm_core::{ansi, color, config, engine, highlighter_from_inline};

pub mod appearance;
pub mod cli;
pub mod instances;
pub mod io;
pub mod pty;
pub mod signals;
pub mod stream;
