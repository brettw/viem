//! Where a diff's base text comes from, independent of any version control
//! system. Every call here may block on I/O or a child process: hosts call
//! them from worker threads and install results only if the window, file and
//! base selection still match.

mod eol;
mod git;
mod process;

pub use eol::{checkout_line_endings, EolAttributes};
pub use git::GitRepository;
pub use process::{CancellationToken, ProcessLimits};

use std::path::{Path, PathBuf};

/// Largest base file read from disk.
pub const MAX_BASE_FILE_BYTES: u64 = 256 * 1024 * 1024;

/// The version a document is compared with.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum BaseKind {
    /// The source as last loaded or saved. The host supplies it from memory;
    /// Refresh reads the file on disk instead.
    LastSaved,
    /// The staged version, as `git diff` compares.
    Unstaged,
    /// The version in the current commit, as `git diff HEAD` compares.
    Head,
    /// The version at a branch tip, as `git diff <branch>` compares.
    Branch(String),
    /// An immutable full object ID resolved by the commit picker.
    Commit(String),
    /// An explicitly chosen file.
    File(PathBuf),
}

impl BaseKind {
    pub fn is_git(&self) -> bool {
        matches!(self, Self::Unstaged | Self::Head | Self::Branch(_) | Self::Commit(_))
    }
}

/// One loaded base: its bytes, or absence when the file doesn't exist in that
/// version (a newly added file compares with an empty base).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BaseText {
    pub bytes: Vec<u8>,
    pub exists: bool,
}

/// How a listed file differs from the base.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileChange {
    Modified,
    Added,
    Deleted,
    Renamed,
    /// Only unsaved edits in an open buffer differ.
    Unsaved,
}

/// One entry in the diff's file list. Paths are absolute. A rename's base
/// version lives at `base_path`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChangedFile {
    pub path: PathBuf,
    pub base_path: PathBuf,
    pub change: FileChange,
    pub added: u64,
    pub removed: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BaseError {
    /// The base can't be used for this file, for example outside a
    /// repository or without an installed `git`.
    Unavailable(String),
    Cancelled,
    TimedOut,
    TooLarge,
    Io(String),
    /// A git command failed.
    Command { status: Option<i32>, message: String },
}

impl std::fmt::Display for BaseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(reason) => write!(formatter, "base unavailable: {reason}"),
            Self::Cancelled => formatter.write_str("base load cancelled"),
            Self::TimedOut => formatter.write_str("base load timed out"),
            Self::TooLarge => formatter.write_str("base is too large"),
            Self::Io(message) => write!(formatter, "could not read base: {message}"),
            Self::Command { message, .. } => write!(formatter, "git failed: {message}"),
        }
    }
}

impl std::error::Error for BaseError {}

/// Read an explicitly chosen base file, or the document's file for a Last
/// saved refresh. A missing file is an absent base, not an error.
pub fn read_file_base(path: &Path) -> Result<BaseText, BaseError> {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => Err(BaseError::Unavailable(format!("{} is a directory", path.display()))),
        Ok(metadata) if metadata.len() > MAX_BASE_FILE_BYTES => Err(BaseError::TooLarge),
        Ok(_) => std::fs::read(path)
            .map(|bytes| BaseText { bytes, exists: true })
            .map_err(|error| BaseError::Io(error.to_string())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BaseText { bytes: Vec::new(), exists: false }),
        Err(error) => Err(BaseError::Io(error.to_string())),
    }
}

/// Add open buffers with unsaved changes to a file list. Files already
/// listed keep their entry; the others are listed as unsaved changes.
pub fn include_unsaved(files: &mut Vec<ChangedFile>, unsaved: impl IntoIterator<Item = PathBuf>) {
    for path in unsaved {
        if !files.iter().any(|file| file.path == path) {
            files.push(ChangedFile { base_path: path.clone(), path, change: FileChange::Unsaved, added: 0, removed: 0 });
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
}

#[cfg(test)]
mod tests;
