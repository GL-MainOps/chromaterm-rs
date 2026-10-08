//! Command-line interface.

use std::ffi::OsString;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};

use crate::color::{Color, ColorMode};
use crate::config::resolve::{ResolveOptions, effective_config, expand_patterns, merge};
use crate::config::{self, Layers, Origin, Resolved, Sources};
use crate::engine::{Engine, Highlighter};
use crate::stream::Stream;

const EXAMPLES: &str = "\
Examples:
  tail -f /var/log/syslog | ct          highlight a stream
  ct ssh router1                        run a program under a highlighting PTY
  ct --theme light kubectl get pods     pick a theme
  ct -i '{\"rules\":[{\"regex\":\"ERROR\",\"color\":\"f.white b.bg-red bold\"}]}' make
  ct -c ./team.toml journalctl -f       use a specific config file
  ct config init                        write a commented config to edit
  ct explain 'GET /api 500 10.0.0.1'    see which rule colors what

A PROGRAM whose name clashes with a subcommand can be run with `ct run -- PROGRAM`.";

/// Colorize terminal output with regex rules. A fast, static Rust rewrite of ChromaTerm.
#[derive(Debug, Parser)]
#[command(
    name = "ct",
    version,
    about,
    after_help = EXAMPLES,
    allow_external_subcommands = true,
    disable_version_flag = true,
    override_usage = "ct [OPTIONS] [PROGRAM [ARGS]...]\n       ct [OPTIONS] <COMMAND>",
    subcommand_value_name = "PROGRAM",
    subcommand_help_heading = "Commands (anything else is run as PROGRAM [ARGS]...)"
)]
pub struct Cli {
    /// Print version (also -V)
    #[arg(short = 'v', long, short_alias = 'V', action = clap::ArgAction::Version)]
    version: Option<bool>,
    #[command(flatten)]
    pub opts: GlobalOpts,
    #[command(subcommand)]
    pub command: Option<Cmd>,
}

#[derive(Debug, Args)]
pub struct GlobalOpts {
    /// Config file (TOML, JSON, or legacy ChromaTerm YAML)
    #[arg(
        short,
        long,
        global = true,
        env = "CHROMATERM_CONFIG",
        value_name = "PATH"
    )]
    pub config: Option<PathBuf>,

    /// Inline config as JSON ('{"rules":[…]}') or TOML; repeatable, layered in order
    #[arg(short = 'i', long = "inline", global = true, value_name = "CONFIG")]
    pub inline: Vec<String>,

    /// Ignore config files (built-in defaults + --inline only)
    #[arg(short = 'N', long, global = true)]
    pub no_config: bool,

    /// Theme to use (built-in: dark, light)
    #[arg(
        short,
        long,
        global = true,
        env = "CHROMATERM_THEME",
        value_name = "NAME"
    )]
    pub theme: Option<String>,

    /// Color output mode
    #[arg(long, global = true, value_enum, value_name = "MODE")]
    pub color_mode: Option<ColorModeArg>,

    /// Force truecolor output (same as --color-mode truecolor)
    #[arg(short = 'R', long, global = true, conflicts_with = "color_mode")]
    pub rgb: bool,

    /// Milliseconds to wait for the rest of a partial line before flushing it
    #[arg(long, global = true, value_name = "MS")]
    pub read_timeout: Option<u64>,

    /// Print per-rule match counts and timings to stderr on exit
    #[arg(short, long, global = true)]
    pub benchmark: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ColorModeArg {
    Auto,
    Truecolor,
    #[value(name = "256")]
    Ansi256,
}

#[derive(Debug, Subcommand)]
pub enum Cmd {
    /// Create, inspect, validate, and convert configuration
    #[command(subcommand)]
    Config(ConfigCmd),
    /// List named regex patterns (built-in and your own)
    Patterns {
        /// Show one pattern fully expanded
        name: Option<String>,
    },
    /// List named colors with live swatches
    Colors,
    /// Show which rules highlight which parts of TEXT (reads stdin if no TEXT)
    Explain {
        /// Text to analyze
        text: Vec<String>,
    },
    /// Print a shell completion script
    Completions {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
    /// Run PROGRAM under a highlighting PTY (for names that clash with subcommands)
    Run {
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        program: Vec<OsString>,
    },
    #[command(external_subcommand)]
    External(Vec<OsString>),
}

#[derive(Debug, Subcommand)]
pub enum ConfigCmd {
    /// Write a fully commented config file
    Init {
        /// Include a full, editable copy of every built-in rule, palette entry and pattern
        #[arg(long)]
        full: bool,
        /// Output path ('-' for stdout) [default: ~/.config/chromaterm/config.toml]
        #[arg(short, long, value_name = "PATH")]
        output: Option<PathBuf>,
        /// Overwrite an existing file
        #[arg(short, long)]
        force: bool,
    },
    /// Validate the effective configuration (or a specific file)
    Check {
        /// File to check instead of the discovered one
        path: Option<PathBuf>,
    },
    /// Print the effective, merged configuration
    Show {
        /// Print JSON instead of TOML
        #[arg(long)]
        json: bool,
    },
    /// Show where configuration is searched for and which file is active
    Path,
    /// Convert a Python ChromaTerm YAML config to TOML
    Import {
        /// Legacy YAML config file
        input: PathBuf,
        /// Output path (default: stdout)
        #[arg(short, long, value_name = "PATH")]
        output: Option<PathBuf>,
        /// Overwrite an existing output file
        #[arg(short, long)]
        force: bool,
    },
}

impl GlobalOpts {
    fn sources(&self) -> Sources {
        Sources {
            file: self.config.clone(),
            inline: self.inline.clone(),
            no_config: self.no_config,
        }
    }

    fn resolve_options(&self) -> ResolveOptions {
        let color_mode = if self.rgb {
            Some(ColorMode::TrueColor)
        } else {
            match self.color_mode {
                Some(ColorModeArg::Truecolor) => Some(ColorMode::TrueColor),
                Some(ColorModeArg::Ansi256) => Some(ColorMode::Ansi256),
                Some(ColorModeArg::Auto) => Some(ColorMode::detect()),
                None => None,
            }
        };
        ResolveOptions {
            theme: self.theme.clone(),
            color_mode,
            read_timeout_ms: self.read_timeout,
        }
    }

    fn load(&self) -> Result<(Layers, Resolved)> {
        let layers = self.sources().load()?;
        let resolved = config::resolve(&layers, &self.resolve_options())?;
        Ok((layers, resolved))
    }
}

/// Parse arguments and run. Returns the process exit code.
pub fn main() -> Result<i32> {
    let cli = Cli::parse();
    let opts = &cli.opts;
    match cli.command {
        None => {
            if std::io::stdin().is_terminal() {
                eprintln!(
                    "ct: nothing to highlight.\n\
                     Pipe output into it (`cmd | ct`) or run a program (`ct PROGRAM [ARGS]`).\n\
                     See `ct --help`."
                );
                return Ok(2);
            }
            let (_, resolved) = opts.load()?;
            let (mut stream, timeout) = build_stream(resolved, opts.benchmark);
            crate::io::run_stdin(&mut stream, timeout)?;
            report_benchmark(&stream, opts.benchmark);
            Ok(0)
        }
        Some(Cmd::External(program)) | Some(Cmd::Run { program }) => {
            let (_, resolved) = opts.load()?;
            let (mut stream, timeout) = build_stream(resolved, opts.benchmark);
            let code = match crate::pty::run(&program, &mut stream, timeout) {
                Ok(code) => code,
                Err(e) => {
                    eprintln!("ct: {e:#}");
                    let not_found = e
                        .downcast_ref::<std::io::Error>()
                        .or_else(|| e.root_cause().downcast_ref::<std::io::Error>())
                        .is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound);
                    return Ok(if not_found { 127 } else { 126 });
                }
            };
            report_benchmark(&stream, opts.benchmark);
            Ok(code)
        }
        Some(Cmd::Config(cmd)) => config_cmd(cmd, opts),
        Some(Cmd::Patterns { name }) => patterns_cmd(opts, name.as_deref()),
        Some(Cmd::Colors) => colors_cmd(opts),
        Some(Cmd::Explain { text }) => explain_cmd(opts, &text),
        Some(Cmd::Completions { shell }) => {
            clap_complete::generate(shell, &mut Cli::command(), "ct", &mut std::io::stdout());
            Ok(0)
        }
    }
}

fn build_stream(resolved: Resolved, benchmark: bool) -> (Stream, std::time::Duration) {
    let timeout = resolved.read_timeout;
    let max_line = resolved.max_line_bytes;
    let mut hl = resolved.into_highlighter();
    if benchmark {
        hl.enable_stats();
    }
    (Stream::with_max_pending(hl, max_line), timeout)
}

fn report_benchmark(stream: &Stream, enabled: bool) {
    if !enabled {
        return;
    }
    let hl = stream.highlighter();
    let Some(stats) = hl.stats() else { return };
    let total: u64 = stats.iter().map(|s| s.nanos).sum::<u64>().max(1);
    let mut rows: Vec<_> = hl.rules().iter().zip(stats).enumerate().collect();
    rows.sort_by_key(|(_, (_, s))| std::cmp::Reverse(s.nanos));
    let mut err = std::io::stderr().lock();
    let _ = writeln!(
        err,
        "\n{:>4}  {:>10}  {:>6}  {:>9}  {:<5}  description",
        "rule", "time (ms)", "share", "matches", "eng"
    );
    for (i, (rule, s)) in rows {
        let _ = writeln!(
            err,
            "{:>4}  {:>10.3}  {:>5.1}%  {:>9}  {:<5}  {}{}",
            i + 1,
            s.nanos as f64 / 1e6,
            s.nanos as f64 * 100.0 / total as f64,
            s.matches,
            match rule.matcher.engine() {
                Engine::Fast => "fast",
                Engine::Fancy => "fancy",
            },
            rule.description,
            if s.limit_hits > 0 {
                format!("  [backtrack limit hit {}×]", s.limit_hits)
            } else {
                String::new()
            }
        );
    }
    let _ = writeln!(err, "      {:>10.3}  total", total as f64 / 1e6);
}

fn config_cmd(cmd: ConfigCmd, opts: &GlobalOpts) -> Result<i32> {
    match cmd {
        ConfigCmd::Init {
            full,
            output,
            force,
        } => {
            let text = if full {
                format!(
                    "# Generated by `ct config init --full`: a complete, editable copy of the\n\
                     # built-in defaults. `defaults = false` below keeps the built-in rules\n\
                     # from being appended a second time.\n\n{}",
                    config::BUILTIN_TOML
                )
            } else {
                config::TEMPLATE_TOML.to_owned()
            };
            let path = match output {
                Some(p) => p,
                None => config::default_init_path().context("cannot determine $HOME")?,
            };
            write_output(&path, &text, force)?;
            if path != Path::new("-") {
                eprintln!("ct: wrote {}", path.display());
            }
            Ok(0)
        }
        ConfigCmd::Check { path } => {
            let mut sources = opts.sources();
            if let Some(p) = path {
                sources.file = Some(p);
                sources.no_config = false;
            }
            let layers = sources.load()?;
            let r = config::resolve(&layers, &opts.resolve_options())?;
            let fancy = r
                .rules
                .iter()
                .filter(|x| x.matcher.engine() == Engine::Fancy)
                .count();
            println!(
                "OK: {} rules ({} fast, {} backtracking), theme \"{}\", {} palette colors, {} patterns",
                r.rules.len(),
                r.rules.len() - fancy,
                fancy,
                r.merged.theme,
                r.palette.len(),
                r.merged.patterns.len()
            );
            println!(
                "    config file: {}",
                layers
                    .file()
                    .map_or("(none — built-in defaults)".into(), |p| p
                        .display()
                        .to_string())
            );
            if !opts.inline.is_empty() {
                println!("    inline layers: {}", opts.inline.len());
            }
            println!(
                "    built-in rules: {}",
                if r.merged.defaults { "included" } else { "off" }
            );
            Ok(0)
        }
        ConfigCmd::Show { json } => {
            let layers = opts.sources().load()?;
            let merged = merge(&layers, &opts.resolve_options())?;
            let cfg = effective_config(&merged);
            let text = if json {
                serde_json::to_string_pretty(&cfg)? + "\n"
            } else {
                format!(
                    "# Effective configuration (theme \"{}\" applied, {} rules).\n{}",
                    merged.theme,
                    merged.rules.len(),
                    toml::to_string_pretty(&cfg)?
                )
            };
            print!("{text}");
            Ok(0)
        }
        ConfigCmd::Path => {
            let explicit = opts.config.as_ref();
            println!("Search order (first existing file wins):");
            if let Some(p) = explicit {
                println!(
                    "  * {}  [--config / $CHROMATERM_CONFIG]{}",
                    p.display(),
                    if p.is_file() { "" } else { "  (MISSING)" }
                );
            }
            let mut active = explicit.filter(|p| p.is_file()).cloned();
            for p in config::search_paths() {
                let exists = p.is_file();
                let mark = if exists && active.is_none() {
                    active = Some(p.clone());
                    "*"
                } else if exists {
                    "+"
                } else {
                    " "
                };
                let legacy = matches!(config::Format::from_path(&p), config::Format::LegacyYaml);
                println!(
                    "  {mark} {}{}",
                    p.display(),
                    if legacy { "  (legacy YAML)" } else { "" }
                );
            }
            match &active {
                Some(p) => println!("Active: {}", p.display()),
                None => {
                    println!("Active: none (built-in defaults). Create one with `ct config init`.")
                }
            }
            if opts.no_config {
                println!("Note: --no-config is set; files are ignored.");
            }
            Ok(0)
        }
        ConfigCmd::Import {
            input,
            output,
            force,
        } => import_cmd(&input, output.as_deref(), force),
    }
}

#[cfg(feature = "legacy-yaml")]
fn import_cmd(input: &Path, output: Option<&Path>, force: bool) -> Result<i32> {
    let text = std::fs::read_to_string(input)
        .with_context(|| format!("cannot read {}", input.display()))?;
    let doc =
        config::legacy::import(&text).map_err(|e| anyhow::anyhow!("{}: {e}", input.display()))?;
    let toml = doc.to_toml(&input.display().to_string());
    // Validate the result before handing it over.
    let cfg = config::parse(&toml, config::Format::Toml).map_err(anyhow::Error::msg)?;
    let layers = Layers(vec![
        (Origin::Builtin, config::builtin()),
        (Origin::File(input.into()), cfg),
    ]);
    if let Err(e) = config::resolve(&layers, &ResolveOptions::default()) {
        eprintln!("ct: warning: the converted config has problems to fix by hand:\n{e}");
    }
    for w in &doc.warnings {
        eprintln!("ct: note: {w}");
    }
    write_output(output.unwrap_or(Path::new("-")), &toml, force)?;
    if let Some(p) = output.filter(|p| *p != Path::new("-")) {
        eprintln!(
            "ct: wrote {} ({} palette colors, {} rules)",
            p.display(),
            doc.palette.len(),
            doc.rules.len()
        );
    }
    Ok(0)
}

#[cfg(not(feature = "legacy-yaml"))]
fn import_cmd(_: &Path, _: Option<&Path>, _: bool) -> Result<i32> {
    bail!("this build of ct was compiled without the `legacy-yaml` feature")
}

fn write_output(path: &Path, text: &str, force: bool) -> Result<()> {
    if path == Path::new("-") {
        std::io::stdout().write_all(text.as_bytes())?;
        return Ok(());
    }
    if path.exists() && !force {
        bail!(
            "{} already exists (use --force to overwrite)",
            path.display()
        );
    }
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    }
    std::fs::write(path, text).with_context(|| format!("cannot write {}", path.display()))
}

fn patterns_cmd(opts: &GlobalOpts, name: Option<&str>) -> Result<i32> {
    let layers = opts.sources().load()?;
    let merged = merge(&layers, &opts.resolve_options())?;
    if let Some(name) = name {
        let Some((src, origin)) = merged.patterns.get(name) else {
            bail!("unknown pattern \"{name}\"");
        };
        println!("# {name} ({origin})\n{src}");
        let expanded = expand_patterns(src, &merged.patterns).map_err(anyhow::Error::msg)?;
        if expanded != *src {
            println!("\n# expanded\n{expanded}");
        }
        return Ok(0);
    }
    let width = merged.patterns.keys().map(String::len).max().unwrap_or(0);
    for (name, (src, origin)) in &merged.patterns {
        let one_line = src.split_whitespace().collect::<Vec<_>>().join(" ");
        let shown: String = if one_line.chars().count() > 90 {
            one_line.chars().take(89).chain(['…']).collect()
        } else {
            one_line
        };
        let tag = match origin {
            Origin::Builtin => "",
            _ => " *",
        };
        println!("{name:width$}{tag:2}  {shown}");
    }
    println!("\n(* = defined in your config.) Use `ct patterns NAME` for the full regex.");
    Ok(0)
}

fn colors_cmd(opts: &GlobalOpts) -> Result<i32> {
    let (_, resolved) = opts.load()?;
    let mode = resolved.color_mode;
    let width = resolved.palette.keys().map(String::len).max().unwrap_or(0);
    let mut out = std::io::stdout().lock();
    writeln!(
        out,
        "Theme: {} (themes: {})",
        resolved.merged.theme,
        resolved.merged.themes.join(", ")
    )?;
    for (name, color) in &resolved.palette {
        let raw = &resolved.merged.palette[name];
        let (swatch, sample) = match color.to_rgb() {
            Some(_) => {
                let mut bg = Vec::new();
                color.for_mode(mode).write_params(true, &mut bg);
                let mut fg = Vec::new();
                color.for_mode(mode).write_params(false, &mut fg);
                (
                    format!("\x1b[{}m      \x1b[0m", String::from_utf8_lossy(&bg)),
                    format!("\x1b[{}m{name}\x1b[0m", String::from_utf8_lossy(&fg)),
                )
            }
            None => ("      ".into(), name.clone()),
        };
        let pad = " ".repeat(width.saturating_sub(name.len()));
        let value = match color {
            Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
            Color::Ansi(n) | Color::Indexed(n) => format!("ansi:{n}"),
            Color::Default => "default".into(),
        };
        let alias = if raw.trim() != value && !raw.starts_with('#') {
            format!("  ← {raw}")
        } else {
            String::new()
        };
        writeln!(out, "{swatch} {sample}{pad}  {value}{alias}")?;
    }
    writeln!(
        out,
        "\nUse as `f.NAME` (foreground) / `b.NAME` (background) in a rule's color."
    )?;
    Ok(0)
}

fn explain_cmd(opts: &GlobalOpts, text: &[String]) -> Result<i32> {
    let (_, resolved) = opts.load()?;
    let lines: Vec<String> = if text.is_empty() {
        std::io::stdin().lines().collect::<std::io::Result<_>>()?
    } else {
        vec![text.join(" ")]
    };
    let mut hl: Highlighter = resolved.into_highlighter();
    let mut out = std::io::stdout().lock();
    for line in lines {
        let mut rendered = Vec::new();
        hl.highlight_line(line.as_bytes(), &mut rendered);
        out.write_all(&rendered)?;
        out.write_all(b"\x1b[0m\n")?;
        let (hay, spans) = hl.explain(line.as_bytes());
        if spans.is_empty() {
            writeln!(out, "  (no matches)")?;
        }
        for s in spans {
            let rule = &hl.rules()[s.rule as usize];
            writeln!(
                out,
                "  {:>3}..{:<3} {:<24} rule #{} {}{}",
                s.start,
                s.end,
                format!("{:?}", String::from_utf8_lossy(&hay[s.start..s.end])),
                s.rule + 1,
                rule.description,
                if rule.exclusive { " [exclusive]" } else { "" }
            )?;
        }
    }
    Ok(0)
}
