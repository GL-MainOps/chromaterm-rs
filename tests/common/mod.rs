#![allow(dead_code)]

use assert_cmd::Command;
use tempfile::TempDir;

/// A `ct` command isolated from the user's real configuration.
pub struct Ct {
    pub home: TempDir,
}

impl Ct {
    pub fn new() -> Self {
        Ct {
            home: tempfile::tempdir().unwrap(),
        }
    }

    pub fn cmd(&self) -> Command {
        let mut c = Command::cargo_bin("ct").unwrap();
        c.env("HOME", self.home.path())
            .env("XDG_CONFIG_HOME", self.home.path().join(".config"))
            .env_remove("CHROMATERM_CONFIG")
            .env_remove("CHROMATERM_THEME")
            .env_remove("COLORTERM")
            .env("TERM", "xterm-256color");
        c
    }
}

/// Inline config with a single rule and no built-in rules.
pub fn only_rule(regex: &str, color: &str) -> String {
    serde_json::json!({ "defaults": false, "rules": [{ "regex": regex, "color": color }] })
        .to_string()
}

/// Remove every SGR (`ESC [ … m`) sequence.
pub fn strip_sgr(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b && bytes.get(i + 1) == Some(&b'[') {
            let mut j = i + 2;
            while j < bytes.len()
                && (bytes[j].is_ascii_digit() || bytes[j] == b';' || bytes[j] == b':')
            {
                j += 1;
            }
            if bytes.get(j) == Some(&b'm') {
                i = j + 1;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}
