//! Explicit release benchmark and structural regression gate. Run with:
//! cargo test --release --offline --lib pinned_provider_performance -- --ignored --nocapture
use super::*;
use crate::document::FormattedTextTree;
use serde_json::{json, Value};

const FIXTURE_REVISION: u64 = 1;
const LOCAL_READ_CEILING: usize = 128 * 1024;
const LOCAL_PROGRESS_CEILING: usize = 4096;
const QUERY_MATCH_CEILING: usize = 512;
const QUERY_COPY_CEILING: usize = 16 * 1024;

fn budget() -> TreeSitterBudget {
    TreeSitterBudget {
        max_native_bytes: 768 * 1024 * 1024,
        max_tree_nodes: 50_000_000,
        max_edit_tree_nodes: usize::MAX,
        max_edit_root_children: usize::MAX,
        ..TreeSitterBudget::default()
    }
}
fn snapshot(tree: FormattedTextTree, revision: u64) -> SyntaxInputSnapshot {
    SyntaxInputSnapshot::new(
        SyntaxInputIdentity {
            document: 991,
            revision,
            generation: 1,
        },
        tree,
    )
}
fn complete(
    session: &mut TreeSitterSession,
    input: &SyntaxInputSnapshot,
    edit: &[SyntaxInputEdit],
    slices: &mut Vec<f64>,
) -> (ParseSnapshot, TreeSitterWork) {
    let mut sum = TreeSitterWork::default();
    for _ in 0..16_384 {
        let began = Instant::now();
        let output = session
            .parse(input.clone(), edit, &[], &budget(), &AtomicBool::new(false))
            .unwrap();
        slices.push(began.elapsed().as_secs_f64() * 1000.0);
        let work = match &output {
            ParseOutcome::Complete { work, .. } | ParseOutcome::Yielded { work } => work,
        };
        sum.input_bytes_supplied += work.input_bytes_supplied;
        sum.input_callbacks += work.input_callbacks;
        sum.progress_callbacks += work.progress_callbacks;
        sum.maximum_input_chunk = sum.maximum_input_chunk.max(work.maximum_input_chunk);
        sum.native_peak_bytes = sum.native_peak_bytes.max(work.native_peak_bytes);
        if let ParseOutcome::Complete { snapshot, .. } = output {
            return (snapshot, sum);
        }
    }
    panic!("fixture exhausted finite parse budget");
}
fn percentiles(mut values: Vec<f64>) -> Value {
    values.sort_by(f64::total_cmp);
    let at = |percent: usize| values[(values.len().saturating_sub(1) * percent) / 100];
    json!({ "samples": values.len(), "p50_ms":at(50), "p95_ms":at(95), "p99_ms":at(99), "maximum_ms":values.last() })
}
fn command(program: &str, args: &[&str]) -> String {
    std::process::Command::new(program)
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_else(|_| "unavailable".into())
}

#[test]
#[ignore = "Pinned 10k/100k/1m nine-language release performance gate; emits target/treesitter-benchmark.json"]
fn pinned_provider_performance() {
    let fixtures = [
        ("c", "functions", "int f(void) {\nreturn 1;\n}\n"),
        ("cpp", "functions", "int f() {\nreturn 1;\n}\n"),
        ("rust", "functions", "fn f()->u8 {\n1\n}\n"),
        ("swift", "functions", "func f()->Int {\nreturn 1\n}\n"),
        ("objc", "functions", "int f(void) {\nreturn 1;\n}\n"),
        (
            "c_sharp",
            "functions",
            "class C {\nint F() {\nreturn 1;\n}\n}\n",
        ),
        ("javascript", "functions", "function f() {\nreturn 1;\n}\n"),
        (
            "typescript",
            "functions",
            "function f(): number {\nreturn 1;\n}\n",
        ),
        ("python", "functions", "def f():\n    return 1\n\n"),
        ("c", "wide-statements", "int x = 1;\n"),
    ];
    let mut rows = Vec::new();
    let mut failures = 0;
    for (language, fixture, line) in fixtures {
        let package = TreeSitterPackage::bundled(language).unwrap();
        for count in [10_000, 100_000, 1_000_000] {
            let attempt = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let lines_per_unit = line.bytes().filter(|b| *b == b'\n').count();
                let units = count / lines_per_unit;
                let text = line.repeat(units) + &"\n".repeat(count % lines_per_unit);
                let mut input = snapshot(FormattedTextTree::try_from_text(text).unwrap(), 1);
                let mut session = TreeSitterSession::new(package.clone()).unwrap();
                let mut cold_slices = Vec::new();
                let began = Instant::now();
                let (cold, cold_work) = complete(&mut session, &input, &[], &mut cold_slices);
                let cold_ms = began.elapsed().as_secs_f64() * 1000.0;
                assert!(!cold.has_errors(), "{language}");
                assert!(cold_work.maximum_input_chunk <= 4096);
                drop(cold);
                let mut edit_slices = Vec::new();
                let mut edit_times = Vec::new();
                let mut query_times = Vec::new();
                let mut maximum_read = 0;
                let mut maximum_progress = 0;
                let mut maximum_matches = 0;
                let mut maximum_copies = 0;
                // All three positions exercise character insertion/deletion and
                // hard-line insertion/deletion, restoring each fixture afterward.
                for row in [0, units / 2, units - 1] {
                    let at = row * line.len() + line.find('1').unwrap();
                    let boundary = row * line.len();
                    for (range, replacement) in [
                        (at..at, "0"),
                        (at..at + 1, ""),
                        (boundary..boundary, "\n"),
                        (boundary..boundary + 1, ""),
                    ] {
                        let tree = input
                            .text_tree()
                            .splice(range.clone(), replacement)
                            .unwrap();
                        let next = snapshot(tree, input.identity().revision + 1);
                        let edit = SyntaxInputEdit::new(
                            &input,
                            &next,
                            range.clone(),
                            range.start..range.start + replacement.len(),
                        )
                        .unwrap();
                        let began = Instant::now();
                        let (parsed, work) =
                            complete(&mut session, &next, &[edit], &mut edit_slices);
                        edit_times.push(began.elapsed().as_secs_f64() * 1000.0);
                        maximum_read = maximum_read.max(work.input_bytes_supplied);
                        maximum_progress = maximum_progress.max(work.progress_callbacks);
                        let viewport = row * line.len()
                            ..(row * line.len() + line.len() + 1).min(next.byte_len());
                        let began = Instant::now();
                        let output = highlight(
                            &parsed,
                            viewport.clone(),
                            &TreeSitterBudget {
                                slice_duration: Duration::from_secs(1),
                                ..budget()
                            },
                            &AtomicBool::new(false),
                        );
                        query_times.push(began.elapsed().as_secs_f64() * 1000.0);
                        assert_eq!(
                            output.coverage,
                            Coverage::Exact,
                            "{language}/{count}: {:?}",
                            output.diagnostic
                        );
                        assert!(output.work.query_matches <= QUERY_MATCH_CEILING);
                        assert!(output.work.predicate_bytes_copied <= QUERY_COPY_CEILING);
                        assert_eq!(output.work.input_callbacks, 0);
                        maximum_matches = maximum_matches.max(output.work.query_matches);
                        maximum_copies = maximum_copies.max(output.work.predicate_bytes_copied);
                        // Re-requesting the same parse for another viewport is free.
                        let ParseOutcome::Complete { work, .. } = session
                            .parse(next.clone(), &[], &[], &budget(), &AtomicBool::new(false))
                            .unwrap()
                        else {
                            panic!()
                        };
                        assert_eq!(work.input_callbacks, 0);
                        assert_eq!(work.progress_callbacks, 0);
                        input = next;
                    }
                }
                let passed = maximum_read <= LOCAL_READ_CEILING
                    && maximum_progress <= LOCAL_PROGRESS_CEILING;
                let row = json!({
                    "language":language, "fixture":fixture, "lines":count, "source_bytes":input.byte_len(),
                    "status":if passed { "exact" } else { "native-gate-failure" },
                    "cold_total_ms":cold_ms, "cold_slices":percentiles(cold_slices),
                    "edit_repair_slices":percentiles(edit_slices), "regional_queries":percentiles(query_times),
                    "edit_repair_total":percentiles(edit_times),
                    "maximum_edit_bytes_supplied":maximum_read, "maximum_edit_progress_callbacks":maximum_progress,
                    "maximum_query_matches":maximum_matches, "maximum_query_bytes_copied":maximum_copies,
                    "native_peak_bytes":cold_work.native_peak_bytes,
                });
                row
            }));
            let row = match attempt {
                Ok(row) => {
                    if row["status"] == "native-gate-failure" {
                        failures += 1;
                    }
                    row
                }
                Err(error) => {
                    failures += 1;
                    let reason = error
                        .downcast_ref::<String>()
                        .cloned()
                        .or_else(|| error.downcast_ref::<&str>().map(|s| (*s).into()))
                        .unwrap_or_else(|| "unknown panic".into());
                    json!({"language":language,"fixture":fixture,"lines":count,"status":"native-gate-failure","reason":reason})
                }
            };
            eprintln!("{row}");
            rows.push(row);
        }
    }
    // Cold input itself is one immutable tree. Only borrowed 4 KiB chunks may
    // reach a provider, and display need not await this deliberately suspended parse.
    let mut source = String::with_capacity(100 * 1024 * 1024);
    while source.len() < 100 * 1024 * 1024 {
        source.push_str("// café: a mixed-length UTF-8 line\nint x = 1;\n");
        source.push_str(&" ".repeat(source.len() % 127));
    }
    let input = snapshot(FormattedTextTree::try_from_text(source).unwrap(), 1);
    let mut session = TreeSitterSession::new(TreeSitterPackage::bundled("c").unwrap()).unwrap();
    let began = Instant::now();
    let ParseOutcome::Yielded { work } = session
        .parse(input.clone(), &[], &[], &budget(), &AtomicBool::new(false))
        .unwrap()
    else {
        panic!("100 MiB parsed without yielding")
    };
    let cold_100mib = json!({ "bytes":input.byte_len(), "slice_ms":began.elapsed().as_secs_f64()*1000.0,
        "bytes_supplied":work.input_bytes_supplied, "maximum_chunk":work.maximum_input_chunk,
        "native_peak_bytes":work.native_peak_bytes });
    assert!(work.maximum_input_chunk <= 4096);
    assert!(work.input_bytes_supplied <= budget().max_input_bytes_supplied);
    assert!(session
        .parse(input, &[], &[], &budget(), &AtomicBool::new(true))
        .is_err());
    session.discard();
    let report = json!({
        "fixture_revision":FIXTURE_REVISION,
        "compiler":command("rustc", &["--version"]), "architecture":std::env::consts::ARCH,
        "os":command("sw_vers", &["-productVersion"]), "cpu":command("sysctl", &["-n","machdep.cpu.brand_string"]),
        "engine":"tree-sitter 0.25.10", "grammar_and_query_revisions":"Cargo.lock and treesitter/PROFILE.md",
        "build":"release", "provider_only":true,
        "exact_provider_gate_failures":failures,
        "reference_hardware":{"model":"Mac13,2","cpu":"Apple M1 Ultra","memory_bytes":137438953472u64},
        "ceilings":{"local_input_bytes":LOCAL_READ_CEILING,"local_progress":LOCAL_PROGRESS_CEILING,
            "query_matches":QUERY_MATCH_CEILING,"query_copied_bytes":QUERY_COPY_CEILING},
        "rows":rows, "cold_100mib":cold_100mib,
        "native_scanner_note":"Callbacks are cooperative; actual maximum slices above are recorded without hard-preemption claims.",
    });
    std::fs::create_dir_all("target").unwrap();
    std::fs::write(
        "target/treesitter-benchmark.json",
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    assert_eq!(failures, 0, "Exact native provider gates failed; details retained in target/treesitter-benchmark.json. Production uses bounded Vim/default fallback.");
}
