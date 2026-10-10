//! chromaterm-core: the ChromaTerm highlighting engine as a library.
//!
//! - [`config`]: load configs (TOML, JSON, legacy ChromaTerm YAML), layer them (built-in →
//!   file → inline) and resolve themes, palettes and rules.
//! - [`appearance`]: pick the `dark` or `light` theme from a known background color.
//! - [`engine`]: the [`engine::Highlighter`]. `highlight_line` rewrites a byte stream
//!   (what `ct` does); `spans` returns styled ranges of plain text for renderers that draw
//!   the text themselves (terminal emulators).
//!
//! No I/O beyond reading config files, no terminal handling: that lives in the `ct` binary.

pub mod ansi;
pub mod appearance;
pub mod color;
pub mod config;
pub mod engine;

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
