//! Bounded regression gate with optional timing output. Work ceilings apply
//! unchanged in ordinary and release tests, including provisional fallback.
use super::*;
use crate::document::FormattedTextTree;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

const SLICE_FUEL: usize = 200_000;
const LOCAL_FUEL: usize = 2_000_000;
const COLD_FUEL: usize = 20_000_000;
const LOCAL_INPUT: usize = 128 * 1024;
const LOCAL_LINES: usize = 4096;
const RETAINED: usize = 64 * 1024 * 1024;

fn input(tree: FormattedTextTree, revision: u64) -> SyntaxInputSnapshot {
    SyntaxInputSnapshot::new(
        SyntaxInputIdentity {
            document: 992,
            revision,
            generation: 1,
        },
        tree,
    )
}

fn percentiles(mut values: Vec<f64>) -> Value {
    values.sort_by(f64::total_cmp);
    if values.is_empty() {
        return json!({"samples":0});
    }
    let at = |p: usize| values[(values.len() - 1) * p / 100];
    json!({"samples":values.len(),"p50_ms":at(50),"p95_ms":at(95),"p99_ms":at(99),"maximum_ms":values.last()})
}

fn measured(
    session: &mut VimSession,
    input: &SyntaxInputSnapshot,
    viewport: Range<usize>,
    budget: VimBudget,
    aggregate_fuel: usize,
    slices: &mut Vec<f64>,
) -> (VimResult, VimStats, bool) {
    let mut total = VimStats::default();
    // A finite call count also bounds a heavily interrupted machine that
    // consumes no matcher fuel before its cooperative wall deadline.
    for _ in 0..4096 {
        let began = Instant::now();
        let deadline = began + Duration::from_millis(2);
        let output = session.highlight_with_control(input, viewport.clone(), budget, &mut || {
            Instant::now() >= deadline
        });
        slices.push(began.elapsed().as_secs_f64() * 1000.0);
        assert!(output.stats.instructions <= budget.instructions);
        assert!(output.stats.retained_bytes <= RETAINED);
        total.instructions += output.stats.instructions;
        total.input_bytes += output.stats.input_bytes;
        total.evaluated_lines += output.stats.evaluated_lines;
        total.checkpoint_visits += output.stats.checkpoint_visits;
        total.retained_bytes = total.retained_bytes.max(output.stats.retained_bytes);
        if !output.stats.yielded {
            let complete = output.covered == viewport && output.diagnostics.is_empty();
            return (output, total, complete);
        }
        if total.instructions >= aggregate_fuel {
            return (output, total, false);
        }
    }
    panic!("finite Vim call-count budget exhausted");
}

fn colors(output: &VimResult, range: Range<usize>) -> Vec<String> {
    let mut colors = vec![String::new(); range.len()];
    for run in &output.runs {
        let clipped = run.range.start.max(range.start)..run.range.end.min(range.end);
        if clipped.start < clipped.end {
            colors[clipped.start - range.start..clipped.end - range.start].fill(run.name.to_string());
        }
    }
    colors
}

fn command(program: &str, args: &[&str]) -> String {
    std::process::Command::new(program)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unavailable".into())
}

fn hardware() -> Value {
    let output = command(
        "/usr/sbin/system_profiler",
        &["SPHardwareDataType", "-json"],
    );
    let parsed: Value = serde_json::from_str(&output).unwrap_or(Value::Null);
    let data = &parsed["SPHardwareDataType"][0];
    // Retain performance context only, excluding device serial/UUID fields.
    json!({"model":data["machine_model"],"cpu":data["chip_type"],"physical_memory":data["physical_memory"]})
}

#[test]
fn pinned_vim_provider_performance() {
    let fixtures = [
        (
            "conf",
            include_str!("fixtures/conf.vim"),
            "# TODO item\nkey = \"hello\"\npath = '/tmp/file'\nplain = yes\n",
            "hello",
        ),
        (
            "dosini",
            include_str!("fixtures/dosini.vim"),
            "[section]\ncount=42\nlabel=value\n; comment\n",
            "42",
        ),
    ];
    let budget = VimBudget {
        instructions: SLICE_FUEL,
        retained_bytes: RETAINED,
        ..Default::default()
    };
    let mut rows = Vec::new();
    for (name, rules, unit, marker) in fixtures {
        let compile_start = Instant::now();
        let program =
            VimProgram::compile(&format!("{name}.vim"), rules, VimLoadLimits::default()).unwrap();
        let compile_ms = compile_start.elapsed().as_secs_f64() * 1000.0;
        for lines in [10_000, 100_000, 1_000_000] {
            let units = lines / 4;
            let mut current = input(
                FormattedTextTree::try_from_text(unit.repeat(units)).unwrap(),
                1,
            );
            let mut session = VimSession::new(program.clone());
            let mut cold_slices = Vec::new();
            let began = Instant::now();
            let (_, cold_work, cold_complete) = measured(
                &mut session,
                &current,
                current.byte_len() - unit.len()..current.byte_len(),
                VimBudget {
                    allow_provisional: false,
                    ..budget
                },
                COLD_FUEL,
                &mut cold_slices,
            );
            let cold_ms = began.elapsed().as_secs_f64() * 1000.0;
            let mut edit_times = Vec::new();
            let mut edit_slices = Vec::new();
            let mut viewport_times = Vec::new();
            let mut viewport_prime_times = Vec::new();
            let mut exact_warm_viewports = 0;
            let mut cache_times = Vec::new();
            let mut maximum_fuel = 0;
            let mut maximum_input = 0;
            let mut maximum_lines = 0;
            let mut maximum_checkpoints = 0;
            let mut maximum_retained = cold_work.retained_bytes;
            let mut exact_repairs = 0;
            let mut provisional_repairs = 0;
            for unit_index in [0, units / 2, units - 2] {
                let boundary = unit_index * unit.len();
                let at = boundary + unit.find(marker).unwrap();
                let viewport = boundary..boundary + unit.len();
                // Exact checkpoint priming is separately budgeted background
                // work, excluded from interactive repair/fallback timings.
                let began = Instant::now();
                let (_, _, primed) = measured(
                    &mut session,
                    &current,
                    viewport.clone(),
                    VimBudget {
                        allow_provisional: false,
                        ..budget
                    },
                    COLD_FUEL,
                    &mut Vec::new(),
                );
                viewport_prime_times.push(began.elapsed().as_secs_f64() * 1000.0);
                exact_warm_viewports += usize::from(primed);
                if !primed {
                    session.discard_pending();
                }
                let began = Instant::now();
                let (initial, initial_work, warm) = measured(
                    &mut session,
                    &current,
                    viewport,
                    budget,
                    LOCAL_FUEL,
                    &mut Vec::new(),
                );
                viewport_times.push(began.elapsed().as_secs_f64() * 1000.0);
                assert!(
                    warm,
                    "{name}/{lines}/{boundary}: initial viewport exhausted finite work: {:?}; {:?}; {:?}",
                    initial.diagnostics, initial.covered, initial_work
                );
                for (range, replacement) in [
                    (at..at, "X"),
                    (at..at + 1, ""),
                    (boundary..boundary, "\n"),
                    (boundary..boundary + 1, ""),
                ] {
                    let next = input(
                        current
                            .text_tree()
                            .splice(range.clone(), replacement)
                            .unwrap(),
                        current.identity().revision + 1,
                    );
                    let viewport =
                        boundary..boundary + unit.len() + replacement.len() - range.len();
                    let began = Instant::now();
                    session
                        .apply_edit(
                            current.identity(),
                            next.identity(),
                            range.clone(),
                            range.start + replacement.len(),
                        )
                        .unwrap();
                    let (output, work, complete) = measured(
                        &mut session,
                        &next,
                        viewport.clone(),
                        budget,
                        LOCAL_FUEL,
                        &mut edit_slices,
                    );
                    edit_times.push(began.elapsed().as_secs_f64() * 1000.0);
                    assert!(
                        complete,
                        "{name}/{lines}: finite repair exhausted: {:?}",
                        output.diagnostics
                    );
                    match output.coverage {
                        Coverage::Exact => exact_repairs += 1,
                        Coverage::Provisional => provisional_repairs += 1,
                        Coverage::Missing => panic!("missing completed repair"),
                    }
                    assert!(work.instructions <= LOCAL_FUEL + SLICE_FUEL);
                    assert!(work.input_bytes <= LOCAL_INPUT, "{name}/{lines}: {work:?}");
                    assert!(work.evaluated_lines <= LOCAL_LINES);
                    assert!(work.checkpoint_visits <= 4096);
                    maximum_fuel = maximum_fuel.max(work.instructions);
                    maximum_input = maximum_input.max(work.input_bytes);
                    maximum_lines = maximum_lines.max(work.evaluated_lines);
                    maximum_checkpoints = maximum_checkpoints.max(work.checkpoint_visits);
                    maximum_retained = maximum_retained.max(work.retained_bytes);
                    // Each fixture section begins with a self-contained line or
                    // section delimiter. Include its predecessor (a newly
                    // inserted newline belongs to that section) and successor
                    // so end lookahead does not see an artificial EOF.
                    let oracle_start = boundary.saturating_sub(unit.len());
                    let oracle_end = (viewport.end + unit.len()).min(next.byte_len());
                    let oracle_range = boundary - oracle_start..viewport.end - oracle_start;
                    let oracle_input = input(
                        FormattedTextTree::try_from_text(
                            next.slice(oracle_start..oracle_end).unwrap(),
                        )
                        .unwrap(),
                        1,
                    );
                    let (oracle, _, complete) = measured(
                        &mut VimSession::new(program.clone()),
                        &oracle_input,
                        oracle_range.clone(),
                        VimBudget {
                            allow_provisional: false,
                            ..budget
                        },
                        LOCAL_FUEL,
                        &mut Vec::new(),
                    );
                    assert!(complete);
                    assert_eq!(
                        colors(&output, viewport.clone()),
                        colors(&oracle, oracle_range),
                        "{name}/{lines}/{range:?}"
                    );
                    let began = Instant::now();
                    let cached = session.highlight(&next, viewport, budget);
                    cache_times.push(began.elapsed().as_secs_f64() * 1000.0);
                    assert_eq!(cached.stats.instructions, 0);
                    assert_eq!(cached.stats.input_bytes, 0);
                    assert_eq!(cached.stats.cache_hits, 1);
                    current = next;
                }
            }
            let row = json!({"fixture":name,"lines":lines,"source_bytes":current.byte_len(),"program_generation":program.generation,
                "compile_ms":compile_ms,"exact_cold_prime_complete":cold_complete,"cold_prime_ms":cold_ms,
                "cold_prime_instructions":cold_work.instructions,"cold_prime_slices":percentiles(cold_slices),
                "viewport_exact_prime":percentiles(viewport_prime_times),"exact_warm_viewports":exact_warm_viewports,
                "viewport_initial_or_fallback":percentiles(viewport_times),"edit_repair_total":percentiles(edit_times),
                "edit_repair_slices":percentiles(edit_slices),"cached_repaint":percentiles(cache_times),
                "exact_repairs":exact_repairs,"provisional_repairs":provisional_repairs,
                "maximum_edit_instructions":maximum_fuel,"maximum_edit_input_bytes":maximum_input,
                "maximum_edit_line_entries":maximum_lines,"maximum_checkpoint_visits":maximum_checkpoints,
                "maximum_accounted_retained_bytes":maximum_retained});
            eprintln!("{row}");
            rows.push(row);
        }
    }
    // A hostile failed pattern demonstrates measured finite fallback and its
    // memoized repaint. The same budget applies at both large input sizes.
    let hostile = VimProgram::compile(
        "hostile.vim",
        "syn match Expensive /a\\+$/",
        VimLoadLimits::default(),
    )
    .unwrap();
    let mut fallback_times = Vec::new();
    let mut fallback_slices = Vec::new();
    for bytes in [4 * 1024 * 1024, 100 * 1024 * 1024] {
        let current = input(
            FormattedTextTree::try_from_text("a".repeat(bytes) + "!").unwrap(),
            1,
        );
        for _ in 0..6 {
            let mut session = VimSession::new(hostile.clone());
            let budget = VimBudget {
                instructions: 2000,
                match_instructions: 20_000,
                ..budget
            };
            let began = Instant::now();
            let (output, work, complete) = measured(
                &mut session,
                &current,
                0..80,
                budget,
                40_000,
                &mut fallback_slices,
            );
            fallback_times.push(began.elapsed().as_secs_f64() * 1000.0);
            assert!(!complete && !output.diagnostics.is_empty());
            assert_eq!(output.coverage, Coverage::Missing);
            assert!(work.instructions <= 24_000);
            let cached = session.highlight(&current, 0..80, budget);
            assert_eq!(cached.stats.instructions, 0);
            assert_eq!(cached.stats.cache_hits, 1);
        }
    }
    let Ok(path) = std::env::var("VIEM_VIM_BENCHMARK_REPORT") else {
        return;
    };
    let report = json!({"fixture_revision":1,"runtime_fixture_version":"MacVim 9.1.1887; unmodified conf.vim and dosini.vim",
        "engine":format!("native Vim profile {}", super::NATIVE_PROFILE_VERSION),
        "build":if cfg!(debug_assertions) { "test" } else { "release" },"provider_only":true,
        "compiler":command("rustc", &["--version"]),"architecture":std::env::consts::ARCH,
        "os":command("sw_vers", &["-productVersion"]),"hardware":hardware(),
        "ceilings":{"slice_instructions":SLICE_FUEL,"cold_prime_instructions":COLD_FUEL + SLICE_FUEL,
            "local_instructions":LOCAL_FUEL + SLICE_FUEL,"local_input_bytes":LOCAL_INPUT,"local_line_entries":LOCAL_LINES,
            "local_checkpoint_visits":4096,"accounted_retained_bytes":RETAINED,"continuation_bytes":budget.continuation_bytes,
            "compiled_program_bytes":VimLoadLimits::default().program_bytes},
        "rows":rows,"hostile_fallback_total":percentiles(fallback_times),"hostile_fallback_slices":percentiles(fallback_slices),
        "notes":"All work loops are finite. A 2 ms cooperative deadline is a target, not a hard maximum; actual maxima are recorded. Incomplete exact cold priming is followed by the real provisional viewport policy. Accounted syntax bytes exclude shared input storage and allocator/RSS overhead. Timings do not establish whole-editor latency."});
    std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
}
