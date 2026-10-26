//! Runs the same moderation samples through every candidate engine.
//!
//! `cargo run --release` checks each sample script against the sample posts
//! and times it; `cargo run --release -- limits` runs the hostile scripts in
//! `scripts/<engine>/limits/`, which every engine should stop. Either takes
//! an engine's folder name (`rhai`, `luau`, `star`, `js`, `toml`) to run
//! only that one.

mod context;
mod engines;
mod text;

use std::path::Path;
use std::time::{Duration, Instant};

use context::{Fixture, SAMPLES, fixtures};
use engines::{Engine, start_clock};

/// The wall-clock cap for one script run, as the server would set it.
const TIME_CAP: Duration = Duration::from_millis(100);
const TIMING_RUNS: u32 = 200;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let limits = args.iter().any(|a| a == "limits");
    let mistakes = args.iter().any(|a| a == "mistakes");
    let only = args.iter().find(|a| *a != "limits" && *a != "mistakes");
    let scripts = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts");

    let mut failures = 0;
    for engine in engines::all() {
        if only.is_some_and(|o| o != engine.extension()) {
            continue;
        }
        let dir = scripts.join(engine.extension());
        failures += if limits {
            run_limits(engine.as_ref(), &dir.join("limits"))
        } else if mistakes {
            show_mistakes(engine.as_ref(), &dir.join("mistakes"));
            0
        } else {
            run_samples(engine.as_ref(), &dir)
        };
    }
    if failures > 0 {
        eprintln!("\n{failures} unexpected result(s)");
        std::process::exit(1);
    }
}

fn run_samples(engine: &dyn Engine, dir: &Path) -> usize {
    println!("\n== {} ==", engine.name());
    let fixtures = fixtures();
    let mut failures = 0;
    for sample in SAMPLES {
        let path = dir.join(format!("{sample}.{}", engine.extension()));
        let Ok(source) = std::fs::read_to_string(&path) else {
            println!("{sample:<11} (no script)");
            continue;
        };
        let started = Instant::now();
        let script = match engine.prepare(&source) {
            Ok(script) => script,
            Err(e) => {
                println!("{sample:<11} COMPILE ERROR {e}");
                failures += 1;
                continue;
            }
        };
        let compile = started.elapsed();

        let mut wrong = Vec::new();
        for fixture in &fixtures {
            start_clock(TIME_CAP);
            let expected = expected(fixture, sample);
            match script.run(&fixture.post) {
                Ok(action) if action.kind() == expected => {}
                Ok(action) => wrong.push(format!("{}: expected {expected}, got {action:?}", fixture.name)),
                Err(e) => wrong.push(format!("{}: error {e}", fixture.name)),
            }
        }

        let started = Instant::now();
        for _ in 0..TIMING_RUNS {
            for fixture in &fixtures {
                start_clock(TIME_CAP);
                let _ = script.run(&fixture.post);
            }
        }
        let per_run = started.elapsed() / (TIMING_RUNS * fixtures.len() as u32);

        let verdict = if wrong.is_empty() { "ok" } else { "WRONG" };
        println!(
            "{sample:<11} {verdict:<5}  compile {:>7.1} µs  run {:>6.1} µs",
            micros(compile),
            micros(per_run)
        );
        for w in &wrong {
            println!("            - {w}");
        }
        failures += wrong.len();
    }
    failures
}

fn expected(fixture: &Fixture, sample: &str) -> &'static str {
    fixture.expect.iter().find(|(s, _)| *s == sample).map_or("pass", |(_, kind)| kind)
}

/// Every hostile script must be refused or stopped, and quickly.
fn run_limits(engine: &dyn Engine, dir: &Path) -> usize {
    println!("\n== {} limits ==", engine.name());
    let Ok(entries) = std::fs::read_dir(dir) else {
        println!("(no limit scripts)");
        return 0;
    };
    let mut paths: Vec<_> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    paths.sort();
    let post = &fixtures()[0].post;
    let mut failures = 0;
    for path in paths {
        let name = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
        let source = std::fs::read_to_string(&path).unwrap_or_default();
        let started = Instant::now();
        start_clock(TIME_CAP);
        let outcome = engine.prepare(&source).and_then(|script| script.run(post));
        let took = started.elapsed();
        // Scripts named `ok-*` are hostile to other engines but should
        // simply finish in time here.
        let should_finish = name.starts_with("ok-");
        match outcome {
            Ok(_) if should_finish && took < TIME_CAP => println!("{name:<14} finished in {:>6.1} ms", millis(took)),
            Err(e) if !should_finish => {
                println!("{name:<14} stopped after {:>6.1} ms: {}", millis(took), first_line(&e))
            }
            Err(e) => {
                println!("{name:<14} FAILED after {:.1} ms: {}", millis(took), first_line(&e));
                failures += 1;
            }
            Ok(action) => {
                println!("{name:<14} NOT STOPPED ({:.1} ms), returned {}", millis(took), action.kind());
                failures += 1;
            }
        }
    }
    failures
}

/// Prints what an admin would see for each script in `mistakes/`: refused
/// at upload, an error when it runs, or nothing at all.
fn show_mistakes(engine: &dyn Engine, dir: &Path) {
    println!("\n== {} mistakes ==", engine.name());
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<_> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    paths.sort();
    let post = &fixtures()[1].post;
    for path in paths {
        let name = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
        let source = std::fs::read_to_string(&path).unwrap_or_default();
        start_clock(TIME_CAP);
        let (when, detail) = match engine.prepare(&source) {
            Err(e) => ("refused at upload", e),
            Ok(script) => match script.run(post) {
                Err(e) => ("error when run", e),
                Ok(action) => ("NO ERROR", format!("ran and returned {}", action.kind())),
            },
        };
        println!("{name} ({when}):");
        for line in detail.lines().filter(|l| !l.trim().is_empty()).take(8) {
            println!("    {line}");
        }
    }
}

/// The line of an error that says what happened (the last one, for a
/// Starlark traceback).
fn first_line(s: &str) -> &str {
    let mut lines = s.lines().filter(|l| !l.trim().is_empty());
    let first = lines.next().unwrap_or(s);
    let line = if first.starts_with("Traceback") { s.lines().find(|l| l.starts_with("error:")).unwrap_or(first) } else { first };
    let line = line.trim();
    &line[..line.len().min(110)]
}

fn micros(d: Duration) -> f64 {
    d.as_secs_f64() * 1e6
}

fn millis(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}
