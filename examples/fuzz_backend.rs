//! Deterministic stateful/property fuzzing. See docs/fuzzing.md and --help.
#[path = "fuzz_backend/core.rs"]
mod core;
#[path = "fuzz_backend/model.rs"]
mod model;
#[path = "fuzz_backend/projection.rs"]
mod projection;
#[path = "fuzz_backend/support.rs"]
mod support;

use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Instant;
use support::{Recorder, RunStats};

#[global_allocator]
static ALLOCATOR: support::TrackingAllocator = support::TrackingAllocator;

struct Options {
    suite: String,
    seed: u64,
    steps: usize,
    sessions: usize,
    trace: PathBuf,
    replay: Option<PathBuf>,
}

fn options() -> Result<Option<Options>, String> {
    let mut o = Options {
        suite: "core".into(),
        seed: 1,
        steps: 1000,
        sessions: 1,
        trace: "fuzz-artifacts/trace.jsonl".into(),
        replay: None,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--help" || arg == "-h" {
            println!("fuzz_backend --suite core|model|projection --seed N --steps N --sessions N --trace PATH\n\
                      fuzz_backend --replay INPUT.jsonl --trace REPLAY.jsonl\n\
                      Use scripts/fuzz-backend.py for campaigns, RSS limits and timeouts.\n\
                      Every action is flushed before execution. Nonzero exit means a finding or invalid input.");
            return Ok(None);
        }
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {arg}"))?;
        match arg.as_str() {
            "--suite" => o.suite = value,
            "--seed" => o.seed = value.parse().map_err(|_| "invalid seed")?,
            "--steps" => o.steps = value.parse().map_err(|_| "invalid steps")?,
            "--sessions" => o.sessions = value.parse().map_err(|_| "invalid sessions")?,
            "--trace" => o.trace = value.into(),
            "--replay" => o.replay = Some(value.into()),
            _ => return Err(format!("unknown option {arg}")),
        }
    }
    if !["core", "model", "projection"].contains(&o.suite.as_str()) {
        return Err("unknown suite".into());
    }
    if o.sessions == 0 || o.steps == 0 {
        return Err("steps and sessions must be positive".into());
    }
    Ok(Some(o))
}

struct ReplaySession {
    seed: u64,
    actions: Vec<Value>,
}
fn read_replay(path: &std::path::Path) -> Result<(String, Vec<ReplaySession>), String> {
    use std::io::BufRead;
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut suite = None;
    let mut sessions: Vec<ReplaySession> = Vec::new();
    for (line_index, line) in std::io::BufReader::new(file).lines().enumerate() {
        let line = line.map_err(|e| e.to_string())?;
        let value: Value = serde_json::from_str(&line)
            .map_err(|e| format!("trace line {}: {e}", line_index + 1))?;
        match value["type"].as_str() {
            Some("header") => {
                if suite.is_some() || value["version"] != 1 {
                    return Err("unsupported/duplicate trace header".into());
                }
                suite = Some(value["suite"].as_str().ok_or("missing suite")?.to_string());
            }
            Some("session") => sessions.push(ReplaySession {
                seed: value["seed"].as_u64().ok_or("missing session seed")?,
                actions: Vec::new(),
            }),
            Some("action") => {
                let session = sessions.last_mut().ok_or("action before session")?;
                if value["index"].as_u64() != Some(session.actions.len() as u64) {
                    return Err("nonsequential action trace".into());
                }
                session
                    .actions
                    .push(value.get("action").ok_or("missing action")?.clone());
            }
            Some(
                "memory" | "session_end" | "result" | "failure" | "expected_rejection"
                | "failure_context",
            ) => {}
            _ => {
                return Err(format!(
                    "unrecognized trace record on line {}",
                    line_index + 1
                ))
            }
        }
    }
    Ok((suite.ok_or("missing trace header")?, sessions))
}

fn run_suite(
    suite: &str,
    seed: u64,
    steps: usize,
    replay: Option<&[Value]>,
    recorder: &mut Recorder,
) -> Result<RunStats, String> {
    match suite {
        "core" => core::run(seed, steps, replay, recorder),
        "model" => model::run(seed, steps, replay, recorder),
        "projection" => projection::run(seed, steps, replay, recorder),
        _ => Err(format!("unknown replay suite {suite}")),
    }
}

fn main() {
    let code = match execute() {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("FUZZ FAILURE: {error}");
            1
        }
    };
    std::process::exit(code);
}

fn execute() -> Result<(), String> {
    let Some(mut options) = options()? else {
        return Ok(());
    };
    let replay = if let Some(path) = &options.replay {
        if path == &options.trace
            || (path.canonicalize().ok().is_some()
                && path.canonicalize().ok() == options.trace.canonicalize().ok())
        {
            return Err("replay input and output trace must differ".into());
        }
        let (suite, sessions) = read_replay(path)?;
        options.suite = suite;
        options.sessions = sessions.len();
        Some(sessions)
    } else {
        None
    };
    let mut recorder = Recorder::new(&options.trace)?;
    recorder.event(
        json!({"type":"header","version":1,"suite":options.suite,"seed":options.seed,
        "steps":options.steps,"sessions":options.sessions,"generator":"splitmix64-v1",
        "package":env!("CARGO_PKG_VERSION"),"replay":options.replay,"memory":support::memory()}),
    )?;
    let began = Instant::now();
    let mut total = RunStats::default();
    for index in 0..options.sessions {
        let seed = replay
            .as_ref()
            .map_or(options.seed.wrapping_add(index as u64), |r| r[index].seed);
        recorder.session(index, seed)?;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_suite(
                &options.suite,
                seed,
                options.steps,
                replay.as_ref().map(|r| r[index].actions.as_slice()),
                &mut recorder,
            )
        }));
        let result = match result {
            Ok(result) => result,
            Err(payload) => Err(format!(
                "panic: {}",
                payload
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "non-string payload".into())
            )),
        };
        match result {
            Ok(stats) => {
                // Suite-local state has dropped before this live-heap sample.
                recorder.event(json!({"type":"session_end","index":index,"stats":stats,"memory":support::memory()}))?;
                total.actions += stats.actions;
                total.expected_rejections += stats.expected_rejections;
                total.max_source_bytes = total.max_source_bytes.max(stats.max_source_bytes);
            }
            Err(error) => {
                recorder.event(json!({"type":"failure","session":index,"error":error,"memory":support::memory()}))?;
                return Err(format!("suite={} seed={seed} session={index}: {error}\nReplay: cargo run --release --example fuzz_backend -- --replay '{}' --trace /tmp/evim-fuzz-replay.jsonl",options.suite,options.trace.display()));
            }
        }
    }
    let summary = json!({"type":"result","status":"ok","suite":options.suite,"seed":options.seed,
        "sessions":options.sessions,"stats":total,"elapsed_seconds":began.elapsed().as_secs_f64(),"memory":support::memory()});
    recorder.event(summary.clone())?;
    println!("{summary}");
    Ok(())
}
