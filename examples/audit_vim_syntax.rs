//! Compile an installed syntax directory with Viem's native loader, without Vim.
//! Usage: cargo run --release --offline --example audit_vim_syntax -- ROOT [FILTER]
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
use viem_core::document::syntax::vim::{VimLoadLimits, VimProgram, NATIVE_PROFILE_VERSION};

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
    let filter = args.next().unwrap_or_default();
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
            VimProgram::load_directory_cancellable(
                &root,
                &language,
                VimLoadLimits::default(),
                cancelled,
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
        let entry = match result {
            Ok(program) => {
                json!({"file": relative, "kind": if package { "package" } else { "fragment" }, "status": "compiled", "rules": program.rule_count(), "source_files": program.source_files, "elapsed_ms": elapsed})
            }
            Err(diagnostics) => {
                let diagnostics: Vec<Value> = diagnostics.iter().map(|d| {
                    let kind = category(&d.message);
                    *categories.entry(kind.into()).or_default() += 1;
                    json!({"file": d.file, "line": d.line, "category": kind, "message": d.message})
                }).collect();
                json!({"file": relative, "kind": if package { "package" } else { "fragment" }, "status": "rejected", "diagnostics": diagnostics, "elapsed_ms": elapsed})
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
    let report = json!({"root": root, "profile_version": NATIVE_PROFILE_VERSION, "filter": filter, "files": entries.len(), "compiled": compiled, "rejected": entries.len() - compiled, "elapsed_ms": started.elapsed().as_millis(), "diagnostic_categories": categories, "entries": entries});
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
