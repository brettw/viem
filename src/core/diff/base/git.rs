//! The git base source. It runs the installed `git` program with argument
//! lists, never a shell, from the repository root, and only read-only
//! commands. Options and environment keep git from taking locks, prompting,
//! paging or starting the programs a repository's configuration can name
//! (`core.fsmonitor`, external diff drivers and textconv). Base contents are
//! read as raw blobs and converted to working-tree line endings here.

use super::eol::{checkout_line_endings, EolAttributes};
use super::process::{run, CancellationToken, ProcessLimits, ProcessOutput};
use super::{BaseError, BaseKind, BaseText, ChangedFile, FileChange};
use std::ffi::{OsStr, OsString};
use std::path::{Component, Path, PathBuf};

const ENVIRONMENT: &[(&str, &str)] = &[
    ("GIT_OPTIONAL_LOCKS", "0"),
    ("GIT_TERMINAL_PROMPT", "0"),
    ("GIT_PAGER", "cat"),
    ("GIT_LITERAL_PATHSPECS", "1"),
    ("LC_ALL", "C"),
];

const SAFETY_OPTIONS: &[&str] = &[
    "--no-optional-locks",
    "-c", "core.fsmonitor=false",
    "-c", "core.pager=cat",
    "-c", "diff.external=",
];

/// One repository, found from a file inside it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitRepository {
    program: PathBuf,
    root: PathBuf,
    limits: ProcessLimits,
}

impl GitRepository {
    /// Find the repository containing `file`, or the folder `file` names.
    /// `program` is the git executable, normally `git` resolved through
    /// `PATH`.
    pub fn discover(
        program: &Path,
        file: &Path,
        limits: ProcessLimits,
        cancellation: &CancellationToken,
    ) -> Result<Self, BaseError> {
        let start = if file.is_dir() { file } else { file.parent().unwrap_or(file) };
        let directory = existing_ancestor(start)
            .ok_or_else(|| BaseError::Unavailable(format!("{} has no existing folder", file.display())))?;
        let output = git(program, &directory, &["rev-parse", "--show-toplevel"], limits, cancellation)?;
        if !output.success {
            return Err(BaseError::Unavailable("not in a git repository".into()));
        }
        let text = String::from_utf8(output.stdout)
            .map_err(|_| BaseError::Unavailable("the repository path is not UTF-8".into()))?;
        let root = PathBuf::from(text.trim_end_matches(['\n', '\r']));
        if root.as_os_str().is_empty() {
            return Err(BaseError::Unavailable("not in a working tree".into()));
        }
        Ok(Self { program: program.to_owned(), root, limits })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `file` relative to the repository root, with `/` separators.
    pub fn relative_path(&self, file: &Path) -> Result<String, BaseError> {
        let root = std::fs::canonicalize(&self.root).map_err(|error| BaseError::Io(error.to_string()))?;
        let file = canonical_through_ancestors(file).ok_or_else(|| BaseError::Unavailable("file path is unavailable".into()))?;
        let relative = file
            .strip_prefix(&root)
            .map_err(|_| BaseError::Unavailable(format!("{} is outside the repository", file.display())))?;
        let mut parts = Vec::new();
        for component in relative.components() {
            match component {
                Component::Normal(part) => parts.push(
                    part.to_str().ok_or_else(|| BaseError::Unavailable("file path is not UTF-8".into()))?.to_owned(),
                ),
                _ => return Err(BaseError::Unavailable("file path is not inside the working tree".into())),
            }
        }
        if parts.is_empty() {
            return Err(BaseError::Unavailable("the repository root is not a file".into()));
        }
        Ok(parts.join("/"))
    }

    /// A folder relative to the repository root, with `/` separators; `None`
    /// for the root itself.
    pub fn relative_folder(&self, folder: &Path) -> Result<Option<String>, BaseError> {
        let root = std::fs::canonicalize(&self.root).map_err(|error| BaseError::Io(error.to_string()))?;
        let folder = std::fs::canonicalize(folder).map_err(|error| BaseError::Io(error.to_string()))?;
        let relative = folder
            .strip_prefix(&root)
            .map_err(|_| BaseError::Unavailable(format!("{} is outside the repository", folder.display())))?;
        let mut parts = Vec::new();
        for component in relative.components() {
            match component {
                Component::Normal(part) => parts.push(
                    part.to_str().ok_or_else(|| BaseError::Unavailable("folder path is not UTF-8".into()))?.to_owned(),
                ),
                _ => return Err(BaseError::Unavailable("folder is not inside the working tree".into())),
            }
        }
        Ok((!parts.is_empty()).then(|| parts.join("/")))
    }

    /// Load the base version of `file` for a git base kind.
    pub fn load(&self, kind: &BaseKind, file: &Path, cancellation: &CancellationToken) -> Result<BaseText, BaseError> {
        Self::load_file(&self.program, kind, file, self.limits, cancellation)
    }

    /// Load the base version of `file` for a git base kind without finding
    /// the repository first. Every command runs from the file's nearest
    /// existing folder and names the file relative to it (`:./name`,
    /// `HEAD:./name`), and the object, its attributes and the line-ending
    /// configuration are read concurrently: one process round trip, not five
    /// in sequence. A file outside a repository is unavailable.
    pub fn load_file(
        program: &Path,
        kind: &BaseKind,
        file: &Path,
        limits: ProcessLimits,
        cancellation: &CancellationToken,
    ) -> Result<BaseText, BaseError> {
        let revision = match kind {
            BaseKind::Unstaged => "",
            BaseKind::Head => "HEAD",
            BaseKind::Branch(branch) => validated_revision(branch)?,
            _ => return Err(BaseError::Unavailable("not a git base".into())),
        };
        let directory = existing_ancestor(file.parent().unwrap_or(file))
            .ok_or_else(|| BaseError::Unavailable(format!("{} has no existing folder", file.display())))?;
        let mut parts = Vec::new();
        for component in file.strip_prefix(&directory).map_err(|_| BaseError::Unavailable("file path is unavailable".into()))?.components() {
            match component {
                Component::Normal(part) => parts.push(
                    part.to_str().ok_or_else(|| BaseError::Unavailable("file path is not UTF-8".into()))?,
                ),
                _ => return Err(BaseError::Unavailable("file path is not inside a folder".into())),
            }
        }
        let relative = format!("./{}", parts.join("/"));
        // `cat-file --batch` reads one object name per line.
        if relative.contains(['\n', '\r']) {
            let repository = Self::discover(program, file, limits, cancellation)?;
            return repository.load_by_discovery(kind, file, cancellation);
        }
        let request = format!("{revision}:{relative}\n");
        let (object, attributes, config) = std::thread::scope(|scope| {
            let attributes = scope.spawn(|| {
                git(program, &directory, &["check-attr", "-z", "text", "eol", "crlf", "--", &relative], limits, cancellation)
            });
            let config = scope.spawn(|| {
                git(program, &directory, &["config", "-z", "--get-regexp", r"^core\.(autocrlf|eol)$"], limits, cancellation)
            });
            let object = git_with_input(program, &directory, &["cat-file", "--batch"], request.as_bytes(), limits, cancellation);
            (object, attributes.join(), config.join())
        });
        let object = object?;
        if !object.success {
            if String::from_utf8_lossy(&object.stderr).contains("not a git repository") {
                return Err(BaseError::Unavailable("not in a git repository".into()));
            }
            return Err(command_error(object));
        }
        let Some(blob) = batch_blob(&object.stdout)? else {
            return Ok(BaseText { bytes: Vec::new(), exists: false });
        };
        let mut eol = EolAttributes::default();
        if let Ok(Ok(output)) = attributes {
            if output.success {
                parse_attributes(&output.stdout, &mut eol);
            }
        }
        if let Ok(Ok(output)) = config {
            // Exit status 1 means neither setting is present.
            parse_core_config(&output.stdout, &mut eol);
        }
        let bytes = match checkout_line_endings(blob, &eol) {
            std::borrow::Cow::Borrowed(bytes) => bytes.to_vec(),
            std::borrow::Cow::Owned(converted) => converted,
        };
        Ok(BaseText { bytes, exists: true })
    }

    /// The original object lookup through the discovered repository, for
    /// paths `cat-file --batch` cannot name.
    fn load_by_discovery(&self, kind: &BaseKind, file: &Path, cancellation: &CancellationToken) -> Result<BaseText, BaseError> {
        let relative = self.relative_path(file)?;
        let object = match kind {
            BaseKind::Unstaged => self.index_object(&relative, cancellation)?,
            BaseKind::Head => self.tree_object("HEAD", &relative, cancellation)?,
            BaseKind::Branch(branch) => self.tree_object(validated_revision(branch)?, &relative, cancellation)?,
            _ => return Err(BaseError::Unavailable("not a git base".into())),
        };
        let Some(object) = object else {
            return Ok(BaseText { bytes: Vec::new(), exists: false });
        };
        let output = self.git(&["cat-file", "blob", &object], cancellation)?;
        if !output.success {
            return Err(command_error(output));
        }
        let attributes = self.eol_attributes(&relative, cancellation)?;
        let bytes = match checkout_line_endings(&output.stdout, &attributes) {
            std::borrow::Cow::Borrowed(_) => output.stdout,
            std::borrow::Cow::Owned(converted) => converted,
        };
        Ok(BaseText { bytes, exists: true })
    }

    /// Tracked files that differ from a git base, as `git diff` lists them,
    /// including its rename detection. Untracked files are never listed.
    /// `scope` limits the list to one folder, as `git diff -- <folder>` does.
    pub fn changed_files(
        &self,
        kind: &BaseKind,
        scope: Option<&Path>,
        cancellation: &CancellationToken,
    ) -> Result<Vec<ChangedFile>, BaseError> {
        let folder = scope.map(|folder| self.relative_folder(folder)).transpose()?.flatten();
        let mut arguments = vec!["diff", "--name-status", "-z", "--no-ext-diff", "--no-textconv"];
        match kind {
            BaseKind::Unstaged => {}
            BaseKind::Head => arguments.push("HEAD"),
            BaseKind::Branch(branch) => arguments.push(validated_revision(branch)?),
            _ => return Err(BaseError::Unavailable("not a git base".into())),
        }
        arguments.push("--");
        // Pathspecs are literal: the environment sets GIT_LITERAL_PATHSPECS.
        if let Some(folder) = &folder {
            arguments.push(folder);
        }
        let output = self.git(&arguments, cancellation)?;
        if !output.success {
            return Err(command_error(output));
        }
        Ok(parse_name_status(&output.stdout, &self.root))
    }

    /// Local and remote branch names for the Branch base.
    pub fn branches(&self, cancellation: &CancellationToken) -> Result<Vec<String>, BaseError> {
        let output = self.git(
            &["for-each-ref", "--format=%(refname:short)", "refs/heads", "refs/remotes"],
            cancellation,
        )?;
        if !output.success {
            return Err(command_error(output));
        }
        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|name| !name.is_empty() && !name.ends_with("/HEAD"))
            .map(str::to_owned)
            .collect())
    }

    fn index_object(&self, relative: &str, cancellation: &CancellationToken) -> Result<Option<String>, BaseError> {
        let output = self.git(&["ls-files", "-s", "-z", "--", relative], cancellation)?;
        if !output.success {
            return Err(command_error(output));
        }
        // `<mode> <object> <stage>\t<path>`; take stage 0 or, while a merge
        // is unresolved, the first listed stage.
        Ok(output
            .stdout
            .split(|&byte| byte == 0)
            .filter_map(|entry| std::str::from_utf8(entry).ok())
            .filter_map(|entry| entry.split_once('\t'))
            .filter(|(_, path)| *path == relative)
            .filter_map(|(fields, _)| fields.split(' ').nth(1).map(str::to_owned))
            .next())
    }

    fn tree_object(&self, revision: &str, relative: &str, cancellation: &CancellationToken) -> Result<Option<String>, BaseError> {
        let output = self.git(&["ls-tree", "-z", revision, "--", relative], cancellation)?;
        if !output.success {
            return Err(command_error(output));
        }
        // `<mode> <type> <object>\t<path>`
        Ok(output
            .stdout
            .split(|&byte| byte == 0)
            .filter_map(|entry| std::str::from_utf8(entry).ok())
            .filter_map(|entry| entry.split_once('\t'))
            .filter(|(fields, path)| *path == relative && fields.split(' ').nth(1) == Some("blob"))
            .filter_map(|(fields, _)| fields.split(' ').nth(2).map(str::to_owned))
            .next())
    }

    fn eol_attributes(&self, relative: &str, cancellation: &CancellationToken) -> Result<EolAttributes, BaseError> {
        let mut attributes = EolAttributes::default();
        let output = self.git(&["check-attr", "-z", "text", "eol", "crlf", "--", relative], cancellation)?;
        if output.success {
            parse_attributes(&output.stdout, &mut attributes);
        }
        let output = self.git(&["config", "-z", "--get-regexp", r"^core\.(autocrlf|eol)$"], cancellation)?;
        // Exit status 1 means neither setting is present.
        parse_core_config(&output.stdout, &mut attributes);
        Ok(attributes)
    }

    fn git(&self, arguments: &[&str], cancellation: &CancellationToken) -> Result<ProcessOutput, BaseError> {
        git(&self.program, &self.root, arguments, self.limits, cancellation)
    }
}

fn git(
    program: &Path,
    directory: &Path,
    arguments: &[&str],
    limits: ProcessLimits,
    cancellation: &CancellationToken,
) -> Result<ProcessOutput, BaseError> {
    run_git(program, directory, arguments, None, limits, cancellation)
}

fn git_with_input(
    program: &Path,
    directory: &Path,
    arguments: &[&str],
    input: &[u8],
    limits: ProcessLimits,
    cancellation: &CancellationToken,
) -> Result<ProcessOutput, BaseError> {
    run_git(program, directory, arguments, Some(input), limits, cancellation)
}

fn run_git(
    program: &Path,
    directory: &Path,
    arguments: &[&str],
    input: Option<&[u8]>,
    limits: ProcessLimits,
    cancellation: &CancellationToken,
) -> Result<ProcessOutput, BaseError> {
    let all: Vec<OsString> = SAFETY_OPTIONS.iter().chain(arguments).map(OsString::from).collect();
    let all: Vec<&OsStr> = all.iter().map(OsString::as_os_str).collect();
    run(program, &all, directory, ENVIRONMENT, input, limits, cancellation)
}

/// `check-attr -z` output: `<path> NUL <attribute> NUL <value> NUL` triples.
fn parse_attributes(stdout: &[u8], attributes: &mut EolAttributes) {
    let fields: Vec<&[u8]> = stdout.split(|&byte| byte == 0).collect();
    for entry in fields.chunks(3).filter(|entry| entry.len() == 3) {
        let value = Some(String::from_utf8_lossy(entry[2]).into_owned());
        match entry[1] {
            b"text" => attributes.text = value,
            b"eol" => attributes.eol = value,
            b"crlf" => attributes.crlf = value,
            _ => {}
        }
    }
}

/// `config -z --get-regexp` output: `<key> LF <value> NUL` entries.
fn parse_core_config(stdout: &[u8], attributes: &mut EolAttributes) {
    for entry in stdout.split(|&byte| byte == 0) {
        let entry = String::from_utf8_lossy(entry);
        if let Some((key, value)) = entry.split_once('\n') {
            match key {
                "core.autocrlf" => attributes.core_autocrlf = Some(value.to_owned()),
                "core.eol" => attributes.core_eol = Some(value.to_owned()),
                _ => {}
            }
        }
    }
}

/// The blob in `cat-file --batch` output for one request: `None` when the
/// name is missing or not a blob (a directory or a submodule).
fn batch_blob(stdout: &[u8]) -> Result<Option<&[u8]>, BaseError> {
    let malformed = || BaseError::Command { status: None, message: "unexpected git cat-file output".into() };
    let header_end = stdout.iter().position(|&byte| byte == b'\n').ok_or_else(malformed)?;
    let header = std::str::from_utf8(&stdout[..header_end]).map_err(|_| malformed())?;
    if header.ends_with(" missing") {
        return Ok(None);
    }
    if header.ends_with(" ambiguous") {
        return Err(BaseError::Command { status: None, message: format!("git object name is ambiguous: {header}") });
    }
    let mut fields = header.rsplitn(3, ' ');
    let (size, kind) = (fields.next(), fields.next());
    let size: usize = size.and_then(|size| size.parse().ok()).ok_or_else(malformed)?;
    let body = stdout.get(header_end + 1..header_end + 1 + size).ok_or_else(malformed)?;
    Ok((kind == Some("blob")).then_some(body))
}

fn command_error(output: ProcessOutput) -> BaseError {
    BaseError::Command {
        status: output.status,
        message: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
    }
}

/// A branch or revision name that can't be mistaken for an option or a
/// `revision:path` expression.
fn validated_revision(name: &str) -> Result<&str, BaseError> {
    if name.is_empty() || name.starts_with('-') || name.contains([':', '\0', '\n']) || name.contains("..") {
        return Err(BaseError::Unavailable(format!("{name:?} is not a branch name")));
    }
    Ok(name)
}

fn existing_ancestor(path: &Path) -> Option<PathBuf> {
    path.ancestors().find(|ancestor| ancestor.is_dir()).map(Path::to_owned)
}

/// Canonicalize a path whose final components may no longer exist (a
/// deleted file or folder), by canonicalizing its nearest existing ancestor.
fn canonical_through_ancestors(path: &Path) -> Option<PathBuf> {
    let mut missing = Vec::new();
    let mut current = path;
    loop {
        if let Ok(canonical) = std::fs::canonicalize(current) {
            let mut result = canonical;
            for part in missing.iter().rev() {
                result.push(part);
            }
            return Some(result);
        }
        missing.push(current.file_name()?.to_owned());
        current = current.parent()?;
    }
}

/// Parse `git diff --name-status -z`: a status field, then one path, or two
/// for renames and copies.
fn parse_name_status(output: &[u8], root: &Path) -> Vec<ChangedFile> {
    let mut fields = output.split(|&byte| byte == 0).filter(|field| !field.is_empty());
    let mut files = Vec::new();
    let path = |field: &[u8]| root.join(String::from_utf8_lossy(field).as_ref());
    while let Some(status) = fields.next() {
        let Some(&letter) = status.first() else { continue };
        match letter {
            b'R' | b'C' => {
                let (Some(old), Some(new)) = (fields.next(), fields.next()) else { break };
                let change = if letter == b'R' { FileChange::Renamed } else { FileChange::Added };
                files.push(ChangedFile { path: path(new), base_path: path(old), change });
            }
            _ => {
                let Some(field) = fields.next() else { break };
                let change = match letter {
                    b'A' => FileChange::Added,
                    b'D' => FileChange::Deleted,
                    _ => FileChange::Modified,
                };
                files.push(ChangedFile { path: path(field), base_path: path(field), change });
            }
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files
}

#[cfg(test)]
pub(super) fn parse_name_status_for_tests(output: &[u8], root: &Path) -> Vec<ChangedFile> {
    parse_name_status(output, root)
}

#[cfg(test)]
mod batch_tests {
    use super::batch_blob;

    #[test]
    fn batch_output_names_blobs_and_absent_objects() {
        assert_eq!(batch_blob(b"0123 blob 3\nabc\n").unwrap(), Some(&b"abc"[..]));
        assert_eq!(batch_blob(b"0123 blob 0\n\n").unwrap(), Some(&b""[..]));
        assert_eq!(batch_blob(b":./x missing\n").unwrap(), None);
        assert_eq!(batch_blob(b"0123 tree 30\n").unwrap_or(None), None);
        assert!(batch_blob(b"HEAD:./x ambiguous\n").is_err());
        assert!(batch_blob(b"0123 blob 10\nshort\n").is_err(), "a truncated body is malformed");
        assert!(batch_blob(b"").is_err());
    }
}
