//! Run a child process with an argument list (never a shell), a time limit,
//! an output limit and cancellation from another thread.

use super::BaseError;
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessLimits {
    pub timeout: Duration,
    pub max_output_bytes: usize,
}

impl Default for ProcessLimits {
    fn default() -> Self {
        Self { timeout: Duration::from_secs(20), max_output_bytes: super::MAX_BASE_FILE_BYTES as usize }
    }
}

/// Cancels the process a load is running, from any thread. Cancelling
/// before the process starts makes the load fail immediately.
#[derive(Clone, Debug, Default)]
pub struct CancellationToken {
    state: Arc<Mutex<CancellationState>>,
}

#[derive(Debug, Default)]
struct CancellationState {
    cancelled: bool,
    running: Option<Arc<Mutex<Child>>>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        let running = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state.cancelled = true;
            state.running.take()
        };
        if let Some(child) = running {
            let _ = child.lock().unwrap_or_else(|error| error.into_inner()).kill();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.state.lock().unwrap_or_else(|error| error.into_inner()).cancelled
    }
}

/// Standard output and the exit status of a finished process.
pub(super) struct ProcessOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub status: Option<i32>,
    pub success: bool,
}

pub(super) fn run(
    program: &Path,
    arguments: &[&std::ffi::OsStr],
    directory: &Path,
    environment: &[(&str, &str)],
    limits: ProcessLimits,
    cancellation: &CancellationToken,
) -> Result<ProcessOutput, BaseError> {
    if cancellation.is_cancelled() {
        return Err(BaseError::Cancelled);
    }
    let mut command = Command::new(program);
    command.args(arguments).current_dir(directory).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    for (key, value) in environment {
        command.env(key, value);
    }
    let mut child = command.spawn().map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => BaseError::Unavailable(format!("{} is not installed", program.display())),
        _ => BaseError::Io(error.to_string()),
    })?;
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let child = Arc::new(Mutex::new(child));
    {
        let mut state = cancellation.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.cancelled {
            drop(state);
            let _ = child.lock().unwrap_or_else(|error| error.into_inner()).kill();
            return Err(BaseError::Cancelled);
        }
        state.running = Some(Arc::clone(&child));
    }
    let limit = limits.max_output_bytes;
    let (sender, receiver) = mpsc::channel();
    let read = move |mut pipe: Box<dyn Read + Send>, sender: mpsc::Sender<(bool, Result<Vec<u8>, ()>)>, is_stdout: bool| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let result = (&mut pipe).take(limit as u64 + 1).read_to_end(&mut bytes)
                .map_err(|_| ())
                .and_then(|_| if bytes.len() > limit { Err(()) } else { Ok(bytes) });
            let _ = sender.send((is_stdout, result));
        });
    };
    read(Box::new(stdout), sender.clone(), true);
    read(Box::new(stderr), sender, false);
    let deadline = std::time::Instant::now() + limits.timeout;
    let (mut out, mut err) = (None, None);
    let mut failure = None;
    while out.is_none() || err.is_none() {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        match receiver.recv_timeout(remaining) {
            Ok((true, result)) => out = Some(result),
            Ok((false, result)) => err = Some(result),
            Err(_) => {
                failure = Some(BaseError::TimedOut);
                break;
            }
        }
        if matches!(out, Some(Err(()))) {
            failure = Some(BaseError::TooLarge);
            break;
        }
    }
    let finished = {
        let mut state = cancellation.state.lock().unwrap_or_else(|error| error.into_inner());
        state.running = None;
        state.cancelled
    };
    let mut child = child.lock().unwrap_or_else(|error| error.into_inner());
    if failure.is_some() || finished {
        let _ = child.kill();
        let _ = child.wait();
        return Err(if finished { BaseError::Cancelled } else { failure.unwrap() });
    }
    let status = child.wait().map_err(|error| BaseError::Io(error.to_string()))?;
    Ok(ProcessOutput {
        stdout: out.and_then(Result::ok).unwrap_or_default(),
        stderr: err.and_then(Result::ok).unwrap_or_default(),
        status: status.code(),
        success: status.success(),
    })
}
