//! Compile an installed syntax directory with Viem's native loader, without Vim.
//! Usage: audit_vim_syntax ROOT [FILTER] [--input FILE] [--filename NAME] [--highlight]
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};
use viem_core::document::{
    syntax::{
        vim::{
            VimBudget, VimLoadLimits, VimProgram, VimSession, VimSetupContext,
            NATIVE_PROFILE_VERSION,
        },
        SyntaxInputIdentity, SyntaxInputSnapshot,
    },
    FormattedTextTree,
};

fn highlight(program: Arc<VimProgram>, input: &SyntaxInputSnapshot) -> Value {
    let mut session = VimSession::new(program);
    let started = Instant::now();
    let deadline = started + Duration::from_secs(10);
    let mut instructions = 0;
    // Match the provider's aggregate instruction allowance, with an additional
    // audit deadline. Exact completion is reported separately from compilation.
    for slice in 1..=320 {
        let result = session.highlight_with_control(
            input,
            0..input.byte_len(),
            VimBudget {
                allow_provisional: false,
                ..Default::default()
            },
            &mut || Instant::now() >= deadline,
        );
        instructions += result.stats.instructions;
        if !result.stats.yielded || slice == 320 || Instant::now() >= deadline {
            return json!({
                "coverage": format!("{:?}", result.coverage),
                "covered": [result.covered.start, result.covered.end],
                "yielded": result.stats.yielded,
                "instructions": instructions,
                "slices": slice,
                "elapsed_ms": started.elapsed().as_millis(),
                "diagnostics": result.diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>(),
                "runs": result.runs.iter().map(|r| json!({
                    "start": r.range.start, "end": r.range.end, "group": r.name.as_str(),
                })).collect::<Vec<_>>(),
            });
        }
    }
    unreachable!()
}

fn files(root: &Path, result: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        // Do not recurse through symlinked directory trees outside this inventory.
        if entry.file_type()?.is_dir() {
            files(&path, result)?;
        } else if path.extension().is_some_and(|ext| ext == "vim") {
            result.push(path);
        }
    }
    Ok(())
}

fn category(message: &str) -> &'static str {
    if message.contains("budget") || message.contains("limit") {
        "resource limit"
    } else if message.contains("cancel") {
        "compilation timeout"
    } else if message.contains("setup")
        || message.contains("condition")
        || message.contains("variable")
        || message.contains("function")
        || message.contains("expression")
    {
        "setup language"
    } else if message.contains("offset") {
        "pattern offsets"
    } else if message.contains("sync") {
        "synchronization"
    } else if message.contains("pattern")
        || message.contains("Vim escape")
        || message.contains("Vim class")
        || message.contains("atom")
        || message.contains("regular")
        || message.contains("magic")
        || message.contains("assertion")
    {
        "pattern language"
    } else if message.contains("include")
        || message.contains("directory")
        || message.contains("No such")
    {
        "runtime dependency"
    } else if message.contains("link") || message.contains("group") || message.contains("cluster") {
        "groups and links"
    } else {
        "syntax declaration"
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let root = PathBuf::from(args.next().ok_or("usage: audit_vim_syntax ROOT [FILTER]")?)
        .canonicalize()?;
    let mut filter = String::new();
    let mut input_path = None;
    let mut filename = None;
    let mut do_highlight = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--input" => {
                input_path = Some(PathBuf::from(args.next().ok_or("--input needs a file")?))
            }
            "--filename" => filename = Some(args.next().ok_or("--filename needs a name")?),
            "--highlight" => do_highlight = true,
            _ if !arg.starts_with('-') && filter.is_empty() => filter = arg,
            _ => return Err(format!("unknown or repeated argument: {arg}").into()),
        }
    }
    if (do_highlight || filename.is_some()) && input_path.is_none() {
        return Err("--highlight and --filename require --input".into());
    }
    let context = input_path
        .as_ref()
        .map(|path| -> Result<_, Box<dyn std::error::Error>> {
            if fs::metadata(path)?.len() > 1024 * 1024 {
                return Err("audit input exceeds 1 MiB".into());
            }
            let input = SyntaxInputSnapshot::new(
                SyntaxInputIdentity {
                    document: 1,
                    revision: 1,
                    generation: 1,
                },
                FormattedTextTree::try_from_text(fs::read_to_string(path)?)?,
            );
            let mut context = VimSetupContext::from_input(&input);
            context.filename = filename.clone();
            Ok(context)
        })
        .transpose()?;
    let mut paths = Vec::new();
    files(&root, &mut paths)?;
    paths.sort();
    let mut entries = Vec::new();
    let mut categories = BTreeMap::<String, usize>::new();
    let started = Instant::now();
    for path in paths {
        let relative = path.strip_prefix(&root)?.to_string_lossy().into_owned();
        if !relative.contains(&filter) {
            continue;
        }
        let language = path.file_stem().unwrap().to_string_lossy();
        let package = path.parent() == Some(root.as_path())
            && language
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        // A watchdog supplies cancellation to the same bounded production loader.
        let cancelled = Arc::new(AtomicBool::new(false));
        let signal = cancelled.clone();
        let (done_tx, done_rx) = mpsc::channel();
        let watchdog = std::thread::spawn(move || {
            if done_rx.recv_timeout(Duration::from_secs(10)).is_err() {
                signal.store(true, Ordering::Relaxed);
            }
        });
        let begin = Instant::now();
        let result = if package {
            VimProgram::load_directory_with_context(
                &root,
                &language,
                VimLoadLimits::default(),
                context.as_ref().unwrap_or(&VimSetupContext::default()),
                &cancelled,
            )
        } else {
            // Nested files are support fragments, not selectable language packages.
            VimProgram::compile_cancellable(
                &path.display().to_string(),
                &fs::read_to_string(&path)?,
                VimLoadLimits::default(),
                cancelled,
            )
        };
        let _ = done_tx.send(());
        watchdog.join().unwrap();
        let elapsed = begin.elapsed().as_millis();
        let kind = if !package {
            "fragment"
        } else if matches!(
            language.as_ref(),
            "2html" | "manual" | "nosyntax" | "syncolor" | "synload" | "syntax"
        ) {
            "utility"
        } else {
            "package"
        };
        let entry = match result {
            Ok(program) => {
                let mut entry = json!({"file": relative, "kind": kind, "status": "compiled", "rules": program.rule_count(), "source_files": program.source_files, "elapsed_ms": elapsed});
                if do_highlight && kind == "package" {
                    entry["highlight"] =
                        highlight(program, context.as_ref().unwrap().input.as_ref().unwrap());
                }
                entry
            }
            Err(diagnostics) => {
                let diagnostics: Vec<Value> = diagnostics.iter().map(|d| {
                    let kind = category(&d.message);
                    *categories.entry(kind.into()).or_default() += 1;
                    json!({"file": d.file, "line": d.line, "category": kind, "message": d.message})
                }).collect();
                json!({"file": relative, "kind": kind, "status": "rejected", "diagnostics": diagnostics, "elapsed_ms": elapsed})
            }
        };
        eprintln!(
            "{}: {} ({} ms)",
            entry["file"].as_str().unwrap(),
            entry["status"].as_str().unwrap(),
            elapsed
        );
        entries.push(entry);
    }
    let compiled = entries.iter().filter(|e| e["status"] == "compiled").count();
    let report = json!({"root": root, "profile_version": NATIVE_PROFILE_VERSION, "filter": filter,
        "input": input_path, "filename": filename, "highlight": do_highlight,
        "files": entries.len(), "compiled": compiled, "rejected": entries.len() - compiled,
        "elapsed_ms": started.elapsed().as_millis(), "diagnostic_categories": categories, "entries": entries});
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
