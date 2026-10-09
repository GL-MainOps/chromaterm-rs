mod common;

use common::{Ct, only_rule, strip_sgr};
use predicates::prelude::*;

#[test]
fn version_and_help() {
    let ct = Ct::new();
    for flag in ["-v", "-V"] {
        ct.cmd()
            .arg(flag)
            .assert()
            .success()
            .stdout(predicate::str::starts_with("ct "));
    }
    ct.cmd()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::starts_with("ct "));
    ct.cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("--inline").and(predicate::str::contains("config")));
}

#[test]
fn highlights_stdin_with_inline_json() {
    let ct = Ct::new();
    ct.cmd()
        .args(["--rgb", "-i", &only_rule(r"\d+", "#ff0000 bold")])
        .write_stdin("a 42 b\nno digits\n")
        .assert()
        .success()
        .stdout("a \x1b[1;38;2;255;0;0m42\x1b[22;39m b\nno digits\n");
}

#[test]
fn inline_toml_and_256_color_mode() {
    let ct = Ct::new();
    ct.cmd()
        .args([
            "--color-mode",
            "256",
            "-i",
            "defaults = false",
            "-i",
            r#"rules = [{ regex = '\d+', color = 'f#ff0000' }]"#,
        ])
        .write_stdin("x 7\n")
        .assert()
        .success()
        .stdout("x \x1b[38;5;196m7\x1b[39m\n");
}

#[test]
fn partial_last_line_is_flushed_at_eof() {
    let ct = Ct::new();
    ct.cmd()
        .args(["--rgb", "-i", &only_rule("x", "#010203")])
        .write_stdin("ax")
        .assert()
        .success()
        .stdout("a\x1b[38;2;1;2;3mx\x1b[39m");
}

#[test]
fn builtin_defaults_apply_without_config() {
    let ct = Ct::new();
    let out = ct
        .cmd()
        .arg("--rgb")
        .write_stdin("ERROR from 10.1.2.3 at 2024-05-01T10:00:00Z\n")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let s = String::from_utf8(out.clone()).unwrap();
    assert!(s.contains("\x1b["), "expected colors: {s:?}");
    assert_eq!(
        strip_sgr(&out),
        b"ERROR from 10.1.2.3 at 2024-05-01T10:00:00Z\n"
    );
}

#[test]
fn invalid_config_reports_all_problems() {
    let ct = Ct::new();
    ct.cmd()
        .args([
            "-i",
            r#"{"rules":[{"regex":"(","color":"red"},{"regex":"x","color":"nope"}]}"#,
        ])
        .write_stdin("x\n")
        .assert()
        .code(1)
        .stderr(
            predicate::str::contains("2 problems")
                .and(predicate::str::contains("invalid regex"))
                .and(predicate::str::contains("unknown color or style \"nope\"")),
        );
    ct.cmd()
        .args(["-i", "{not json"])
        .write_stdin("x\n")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("inline config #1"));
}

#[test]
fn config_file_via_flag_env_and_discovery() {
    let ct = Ct::new();
    let path = ct.home.path().join("my.toml");
    std::fs::write(
        &path,
        "defaults = false\n[[rules]]\nregex = 'hit'\ncolor = '#00ff00'\n",
    )
    .unwrap();
    let expect = "\x1b[38;2;0;255;0mhit\x1b[39m\n";
    ct.cmd()
        .args(["--rgb", "-c"])
        .arg(&path)
        .write_stdin("hit\n")
        .assert()
        .stdout(expect);
    ct.cmd()
        .arg("--rgb")
        .env("CHROMATERM_CONFIG", &path)
        .write_stdin("hit\n")
        .assert()
        .stdout(expect);
    // Discovered at the XDG location.
    let xdg = ct.home.path().join(".config/chromaterm");
    std::fs::create_dir_all(&xdg).unwrap();
    std::fs::copy(&path, xdg.join("config.toml")).unwrap();
    ct.cmd()
        .arg("--rgb")
        .write_stdin("hit\n")
        .assert()
        .stdout(expect);
    // --no-config ignores it (built-in rules don't color "hit").
    ct.cmd()
        .args(["--rgb", "-N"])
        .write_stdin("hit\n")
        .assert()
        .stdout("hit\n");
    ct.cmd()
        .args(["-c", "/nonexistent/ct.toml"])
        .write_stdin("x\n")
        .assert()
        .code(1);
}

#[test]
fn json_config_file() {
    let ct = Ct::new();
    let path = ct.home.path().join("c.json");
    std::fs::write(&path, only_rule("j", "#0000ff")).unwrap();
    ct.cmd()
        .args(["--rgb", "-c"])
        .arg(&path)
        .write_stdin("j\n")
        .assert()
        .stdout("\x1b[38;2;0;0;255mj\x1b[39m\n");
}

#[test]
fn themes_switch_palette() {
    let ct = Ct::new();
    let rule = only_rule("E", "f.red");
    let dark = ct
        .cmd()
        .args(["--rgb", "-i", &rule])
        .write_stdin("E\n")
        .output()
        .unwrap()
        .stdout;
    let light = ct
        .cmd()
        .args(["--rgb", "-i", &rule])
        .env("CHROMATERM_THEME", "light")
        .write_stdin("E\n")
        .output()
        .unwrap()
        .stdout;
    assert_ne!(dark, light);
    ct.cmd()
        .args(["--theme", "nope", "config", "check"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("unknown theme"));
}

#[test]
fn config_init_check_and_force() {
    let ct = Ct::new();
    let path = ct.home.path().join("out/config.toml");
    ct.cmd()
        .args(["config", "init", "-o"])
        .arg(&path)
        .assert()
        .success();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("# PALETTE") && text.contains("[[rules]]"));
    ct.cmd()
        .args(["config", "check"])
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("built-in rules: included"));
    ct.cmd()
        .args(["config", "init", "-o"])
        .arg(&path)
        .assert()
        .code(1)
        .stderr(predicate::str::contains("--force"));
    ct.cmd()
        .args(["config", "init", "--full", "--force", "-o"])
        .arg(&path)
        .assert()
        .success();
    ct.cmd()
        .args(["config", "check"])
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("built-in rules: off"));
    // Default location is the XDG path.
    ct.cmd().args(["config", "init"]).assert().success();
    assert!(
        ct.home
            .path()
            .join(".config/chromaterm/config.toml")
            .is_file()
    );
}

#[test]
fn full_init_behaves_like_builtin_defaults() {
    let ct = Ct::new();
    let path = ct.home.path().join("full.toml");
    ct.cmd()
        .args(["config", "init", "--full", "-o"])
        .arg(&path)
        .assert()
        .success();
    let input = "GET http://x.io/a 200 OK 10.0.0.1 sshd[22] 5ms ERROR true\n";
    let builtin = ct
        .cmd()
        .args(["--rgb", "-N"])
        .write_stdin(input)
        .output()
        .unwrap()
        .stdout;
    let full = ct
        .cmd()
        .args(["--rgb", "-c"])
        .arg(&path)
        .write_stdin(input)
        .output()
        .unwrap()
        .stdout;
    assert_eq!(builtin, full);
}

#[test]
fn config_show_round_trips() {
    let ct = Ct::new();
    for json in [false, true] {
        let mut cmd = ct.cmd();
        cmd.args(["-N", "config", "show"]);
        if json {
            cmd.arg("--json");
        }
        let shown = cmd.output().unwrap().stdout;
        let path = ct
            .home
            .path()
            .join(if json { "shown.json" } else { "shown.toml" });
        std::fs::write(&path, &shown).unwrap();
        let input = "ERROR 10.0.0.1 \"str\" 42 /usr/bin\n";
        let a = ct
            .cmd()
            .args(["--rgb", "-N"])
            .write_stdin(input)
            .output()
            .unwrap()
            .stdout;
        let b = ct
            .cmd()
            .args(["--rgb", "-c"])
            .arg(&path)
            .write_stdin(input)
            .output()
            .unwrap()
            .stdout;
        assert_eq!(a, b, "json={json}");
    }
}

#[test]
fn config_path_lists_search_order() {
    let ct = Ct::new();
    ct.cmd().args(["config", "path"]).assert().success().stdout(
        predicate::str::contains("config.toml").and(predicate::str::contains("Active: none")),
    );
}

#[test]
fn introspection_commands() {
    let ct = Ct::new();
    ct.cmd()
        .arg("patterns")
        .assert()
        .success()
        .stdout(predicate::str::contains("ipv4").and(predicate::str::contains("uuid")));
    ct.cmd()
        .args(["patterns", "ipv4"])
        .assert()
        .success()
        .stdout(predicate::str::contains("# expanded"));
    ct.cmd().args(["patterns", "nope"]).assert().code(1);
    ct.cmd()
        .arg("colors")
        .assert()
        .success()
        .stdout(predicate::str::contains("error").and(predicate::str::contains("← red")));
    ct.cmd()
        .args(["explain", "ERROR", "at", "10.0.0.1"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("IPv4 address")
                .and(predicate::str::contains("\"10.0.0.1\""))
                .and(predicate::str::contains("[exclusive]")),
        );
    ct.cmd()
        .args(["completions", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("_ct"));
}

#[test]
fn runs_programs_under_a_pty() {
    let ct = Ct::new();
    let out = ct
        .cmd()
        .args(["--rgb", "-i", &only_rule(r"\d+", "#ff0000"), "echo", "n=42"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let s = String::from_utf8_lossy(&out);
    assert!(s.contains("\x1b[38;2;255;0;0m42\x1b[39m"), "{s:?}");
    ct.cmd().args(["sh", "-c", "exit 7"]).assert().code(7);
    ct.cmd()
        .args(["run", "--", "sh", "-c", "exit 3"])
        .assert()
        .code(3);
    ct.cmd()
        .arg("definitely-not-a-real-program-xyz")
        .assert()
        .code(127)
        .stderr(predicate::str::contains("failed to run"));
}

#[test]
fn pty_child_receives_stdin_and_eof() {
    let ct = Ct::new();
    let out = ct
        .cmd()
        .args(["-N", "-i", "defaults = false", "cat"])
        .write_stdin("hello\n")
        .timeout(std::time::Duration::from_secs(10))
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert!(String::from_utf8_lossy(&out).contains("hello"));
}

#[test]
fn benchmark_report() {
    let ct = Ct::new();
    ct.cmd()
        .args(["-b", "-N"])
        .write_stdin("ERROR 1 2 3\n")
        .assert()
        .success()
        .stderr(predicate::str::contains("time (ms)").and(predicate::str::contains("Number")));
}

#[cfg(feature = "legacy-yaml")]
#[test]
fn legacy_yaml_import_and_direct_load() {
    let ct = Ct::new();
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy.yml");
    let out = ct.home.path().join("converted.toml");
    ct.cmd()
        .args(["config", "import"])
        .arg(&fixture)
        .arg("-o")
        .arg(&out)
        .assert()
        .success();
    let toml = std::fs::read_to_string(&out).unwrap();
    assert!(
        toml.contains("defaults = false") && toml.contains(r"regex = '\bERROR\b'"),
        "{toml}"
    );
    let input = "ERROR a=1\n";
    let from_toml = ct
        .cmd()
        .args(["--rgb", "-c"])
        .arg(&out)
        .write_stdin(input)
        .output()
        .unwrap();
    let from_yaml = ct
        .cmd()
        .args(["--rgb", "-c"])
        .arg(&fixture)
        .write_stdin(input)
        .output()
        .unwrap();
    assert!(from_toml.status.success());
    assert_eq!(from_toml.stdout, from_yaml.stdout);
    assert_eq!(
        String::from_utf8_lossy(&from_toml.stdout),
        "\x1b[1;38;2;255;0;0mERROR\x1b[22;39m \x1b[38;2;0;136;255ma\x1b[39m=\x1b[48;2;32;32;32m1\x1b[49m\n"
    );
}

#[test]
fn export_formats_round_trip() {
    let ct = Ct::new();
    let src = ct.home.path().join("src.toml");
    std::fs::write(
        &src,
        "defaults = false\n[palette]\nbrand = '#ff6600'\n[[rules]]\ndescription = \"it's\"\nregex = '\\bTODO\\b'\ncolor = 'brand bold'\n",
    )
    .unwrap();
    let input = "TODO x\n";
    let reference = ct
        .cmd()
        .args(["-R", "-c"])
        .arg(&src)
        .write_stdin(input)
        .output()
        .unwrap()
        .stdout;
    assert!(String::from_utf8_lossy(&reference).contains("\x1b[1;38;2;255;102;0mTODO"));

    // toml / json / yaml files (format inferred from the extension).
    // YAML can always be exported, but loading it needs the `legacy-yaml` feature.
    let exts: &[&str] = if cfg!(feature = "legacy-yaml") {
        &["toml", "json", "yml"]
    } else {
        &["toml", "json"]
    };
    for ext in exts {
        let out = ct.home.path().join(format!("out.{ext}"));
        ct.cmd()
            .args(["config", "convert"])
            .arg(&src)
            .arg("-o")
            .arg(&out)
            .assert()
            .success();
        let got = ct
            .cmd()
            .args(["-R", "-c"])
            .arg(&out)
            .write_stdin(input)
            .output()
            .unwrap()
            .stdout;
        assert_eq!(got, reference, "{ext}");
    }

    // --oneline is a single line of JSON usable with -i.
    let line = ct
        .cmd()
        .args(["config", "export", "--oneline"])
        .arg(&src)
        .output()
        .unwrap()
        .stdout;
    let line = String::from_utf8(line).unwrap();
    assert_eq!(line.trim_end().lines().count(), 1);
    let got = ct
        .cmd()
        .args(["-R", "-N", "-i", line.trim_end()])
        .write_stdin(input)
        .output()
        .unwrap()
        .stdout;
    assert_eq!(got, reference);

    // --shell is a quoted --inline argument.
    ct.cmd()
        .args(["config", "export", "--shell"])
        .arg(&src)
        .assert()
        .success()
        .stdout(predicate::str::starts_with("--inline '{").and(predicate::str::contains(r"'\''")));

    // Multi-line formats refuse --oneline.
    ct.cmd()
        .args(["config", "export", "-F", "toml", "--oneline"])
        .assert()
        .code(1);
}

#[test]
fn export_user_layers_and_effective() {
    let ct = Ct::new();
    let out = ct
        .cmd()
        .args([
            "-N",
            "-i",
            r##"{"palette":{"x":"#010203"}}"##,
            "config",
            "export",
            "-F",
            "json",
        ])
        .output()
        .unwrap()
        .stdout;
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["palette"]["x"], "#010203");
    assert!(
        v.get("rules").is_none(),
        "built-ins must not leak into a user export"
    );

    let out = ct
        .cmd()
        .args(["-N", "config", "export", "--effective", "-F", "json"])
        .output()
        .unwrap()
        .stdout;
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["defaults"], false);
    assert!(v["rules"].as_array().unwrap().len() > 20);
}

#[test]
fn reload_running_instances() {
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command, Stdio};

    let ct = Ct::new();
    let run = ct.runtime_dir();
    let cfg = ct.home.path().join("c.toml");
    let write_cfg = |color: &str| {
        std::fs::write(
            &cfg,
            format!("defaults = false\n[[rules]]\nregex = 'hit'\ncolor = '{color}'\n"),
        )
        .unwrap()
    };
    write_cfg("#ff0000");

    let mut child = Command::new(assert_cmd::cargo::cargo_bin("ct"))
        .args(["-R", "-c"])
        .arg(&cfg)
        .env("HOME", ct.home.path())
        .env("XDG_RUNTIME_DIR", &run)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();

    writeln!(stdin, "hit").unwrap();
    assert_eq!(
        lines.next().unwrap().unwrap(),
        "\x1b[38;2;255;0;0mhit\x1b[39m"
    );
    assert!(
        run.join("chromaterm")
            .join(child.id().to_string())
            .is_file()
    );

    write_cfg("#00ff00");
    ct.cmd()
        .env("XDG_RUNTIME_DIR", &run)
        .arg("--reload")
        .assert()
        .success()
        .stdout(predicate::str::contains("Reloaded 1"));
    // The signal is handled asynchronously; poll until the new color shows up.
    let mut reloaded = false;
    for _ in 0..50 {
        writeln!(stdin, "hit").unwrap();
        if lines.next().unwrap().unwrap() == "\x1b[38;2;0;255;0mhit\x1b[39m" {
            reloaded = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(reloaded, "instance did not pick up the new config");

    drop(stdin);
    assert!(child.wait().unwrap().success());
    assert!(!run.join("chromaterm").join(child.id().to_string()).exists());
    ct.cmd()
        .env("XDG_RUNTIME_DIR", &run)
        .args(["config", "reload"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No running ct instances"));
}

#[test]
fn yaml_export_has_no_builtin_extensions() {
    let ct = Ct::new();
    ct.cmd()
        .args(["-N", "config", "export", "-F", "yaml"])
        .assert()
        .success()
        .stdout(
            predicate::str::starts_with("# Exported")
                .and(predicate::str::contains("palette:\n"))
                .and(predicate::str::contains("${").not())
                .and(predicate::str::contains("pattern:").not()),
        );
}

// ---- automatic theme detection --------------------------------------------

/// `ERROR` is rendered bold in the theme's `error` color.
const DARK_ERROR: &str = "\x1b[1;38;2;255;89;77mERROR";
const LIGHT_ERROR: &str = "\x1b[1;38;2;216;14;0mERROR";

const WHITE_TERMINAL: common::Terminal = common::Terminal {
    background: Some("rgb:ffff/ffff/ffff"),
    foreground: Some("rgb:0000/0000/0000"),
    da1: true,
    typed: b"",
};
const DARK_TERMINAL: common::Terminal = common::Terminal {
    background: Some("rgb:0e0e/1313/1717"),
    foreground: Some("rgb:ffff/ffff/ffff"),
    da1: true,
    typed: b"",
};

#[test]
fn auto_theme_filter_mode_follows_the_terminal() {
    let ct = Ct::new();
    let light =
        common::run_in_terminal(&ct, &["-N", "-R"], &[], Some(b"ERROR 42\n"), WHITE_TERMINAL);
    assert!(light.status.success());
    assert!(light.queried());
    assert!(light.screen.contains(LIGHT_ERROR), "{:?}", light.screen);

    let dark = common::run_in_terminal(&ct, &["-N", "-R"], &[], Some(b"ERROR 42\n"), DARK_TERMINAL);
    assert!(dark.screen.contains(DARK_ERROR), "{:?}", dark.screen);
}

#[test]
fn auto_theme_program_mode_forwards_typeahead() {
    let ct = Ct::new();
    let term = common::Terminal {
        typed: b"hello\r",
        ..WHITE_TERMINAL
    };
    let run = common::run_in_terminal(
        &ct,
        &["-N", "-R", "sh", "-c", "read l; echo \"got $l ERROR\""],
        &[],
        None,
        term,
    );
    assert!(run.status.success(), "{:?}", run.screen);
    assert!(
        run.screen.contains("got hello"),
        "typeahead lost: {:?}",
        run.screen
    );
    assert!(run.screen.contains(LIGHT_ERROR), "{:?}", run.screen);
}

#[test]
fn auto_theme_falls_back_without_terminal_support() {
    let ct = Ct::new();
    // Answers DA1 but not OSC 10/11: no waiting for the timeout.
    let basic = common::Terminal {
        da1: true,
        ..Default::default()
    };
    let run = common::run_in_terminal(&ct, &["-N", "-R"], &[], Some(b"ERROR\n"), basic);
    assert!(run.screen.contains(DARK_ERROR), "{:?}", run.screen);
    assert!(
        run.elapsed < std::time::Duration::from_millis(900),
        "{:?}",
        run.elapsed
    );

    // Then $COLORFGBG decides.
    let run = common::run_in_terminal(
        &ct,
        &["-N", "-R"],
        &[("COLORFGBG", "0;15")],
        Some(b"ERROR\n"),
        basic,
    );
    assert!(run.screen.contains(LIGHT_ERROR), "{:?}", run.screen);

    // A terminal that never answers costs the timeout, then dark.
    let silent = common::run_in_terminal(
        &ct,
        &["-N", "-R"],
        &[],
        Some(b"ERROR\n"),
        common::Terminal::default(),
    );
    assert!(silent.status.success());
    assert!(silent.screen.contains(DARK_ERROR), "{:?}", silent.screen);
}

#[test]
fn explicit_theme_skips_detection() {
    let ct = Ct::new();
    for (args, envs) in [
        (vec!["-N", "-R", "--theme", "dark"], vec![]),
        (vec!["-N", "-R"], vec![("CHROMATERM_THEME", "dark")]),
        (vec!["-N", "-R", "-i", "theme = \"dark\""], vec![]),
    ] {
        let run = common::run_in_terminal(&ct, &args, &envs, Some(b"ERROR\n"), WHITE_TERMINAL);
        assert!(
            !run.queried(),
            "{args:?} {envs:?} must not query the terminal"
        );
        assert!(run.screen.contains(DARK_ERROR), "{:?}", run.screen);
    }
    // `auto` re-enables it, even over a config theme.
    let run = common::run_in_terminal(
        &ct,
        &["-N", "-R", "--theme", "auto", "-i", "theme = \"dark\""],
        &[],
        Some(b"ERROR\n"),
        WHITE_TERMINAL,
    );
    assert!(
        run.queried() && run.screen.contains(LIGHT_ERROR),
        "{:?}",
        run.screen
    );
}

#[test]
fn auto_theme_is_reported() {
    let ct = Ct::new();
    let run = common::run_in_terminal(&ct, &["-N", "config", "check"], &[], None, WHITE_TERMINAL);
    assert!(
        run.screen
            .contains("theme: light (auto: terminal background #ffffff / foreground #000000)"),
        "{:?}",
        run.screen
    );
    // Without a terminal: $COLORFGBG, else the default, and why.
    ct.cmd()
        .args(["-N", "config", "check"])
        .env("COLORFGBG", "0;15")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "theme: light (auto: $COLORFGBG=0;15)",
        ));
    ct.cmd()
        .args(["-N", "config", "check"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "theme: dark (auto: default (stdout is not a terminal))",
        ));
    ct.cmd()
        .args(["-N", "--theme", "light", "config", "check"])
        .assert()
        .success()
        .stdout(predicate::str::contains("theme: light (set by --theme"));
}

#[test]
fn colorfgbg_applies_without_a_terminal() {
    let ct = Ct::new();
    ct.cmd()
        .args(["-N", "-R"])
        .env("COLORFGBG", "0;15")
        .write_stdin("ERROR\n")
        .assert()
        .stdout(predicate::str::contains(LIGHT_ERROR));
    ct.cmd()
        .args(["-N", "-R", "--theme", "dark"])
        .env("COLORFGBG", "0;15")
        .write_stdin("ERROR\n")
        .assert()
        .stdout(predicate::str::contains(DARK_ERROR));
}

#[test]
fn background_jobs_never_touch_the_terminal() {
    // An inner ct started as a background job (job control on) is not in the
    // terminal's foreground process group. Querying would get it stopped by
    // SIGTTOU, so it must fall back silently instead. `timeout` turns a hang
    // into a failure.
    let ct = Ct::new();
    let inner = assert_cmd::cargo::cargo_bin("ct");
    let script = format!(
        "set -m; printf 'ERROR\\n' | {} -N -R & wait",
        inner.display()
    );
    let run = common::run_in_terminal(
        &ct,
        &["-N", "timeout", "--foreground", "10", "sh", "-c", &script],
        &[],
        None,
        WHITE_TERMINAL,
    );
    assert!(run.status.success(), "{:?} {:?}", run.status, run.screen);
    // Only the outer (foreground) ct queried.
    assert_eq!(
        run.screen.matches("\x1b]11;?").count(),
        1,
        "{:?}",
        run.screen
    );
    // The inner ct fell back to dark (its error color; the outer ct then
    // re-highlights the word in its own light theme).
    assert!(
        run.screen.contains("\x1b[1;38;2;255;89;77m"),
        "{:?}",
        run.screen
    );
}
