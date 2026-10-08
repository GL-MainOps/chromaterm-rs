//! Compatibility with real Python ChromaTerm configs.
#![cfg(feature = "legacy-yaml")]

use std::path::Path;

use chromaterm::color::ColorMode;
use chromaterm::config::resolve::ResolveOptions;
use chromaterm::config::{self, Sources};

/// The legacy sample shipped next to the repo (not committed) must import,
/// round-trip through TOML, and compile in both Unicode modes.
#[test]
fn sample_legacy_config_compiles() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("sample-config-file-for-current-python-implementation.yaml");
    if !path.is_file() {
        eprintln!("skipping: {} not present", path.display());
        return;
    }
    let text = std::fs::read_to_string(&path).unwrap();
    let doc = config::legacy::import(&text).unwrap();
    assert!(doc.rules.len() > 100);
    let toml = doc.to_toml("sample");
    let reparsed = config::parse(&toml, config::Format::Toml).unwrap();
    assert_eq!(reparsed, doc.to_config());

    for unicode in ["false", "true"] {
        let layers = Sources {
            file: Some(path.clone()),
            inline: vec![format!("settings = {{ unicode = {unicode} }}")],
            no_config: false,
        }
        .load()
        .unwrap();
        let opts = ResolveOptions {
            color_mode: Some(ColorMode::TrueColor),
            ..Default::default()
        };
        let resolved = config::resolve(&layers, &opts).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(resolved.rules.len(), doc.rules.len());
    }
}
