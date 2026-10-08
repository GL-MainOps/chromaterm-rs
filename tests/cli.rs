mod common;

use common::{Ct, only_rule, strip_sgr};
use predicates::prelude::*;

#[test]
fn version_and_help() {
    let ct = Ct::new();
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
