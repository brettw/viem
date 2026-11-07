use super::git::parse_name_status_for_tests;
use super::*;
use std::process::Command;

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new(name: &str) -> Self {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("viem-diff-{name}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn git_available() -> bool {
    Command::new("git").arg("--version").output().is_ok_and(|output| output.status.success())
}

fn git(directory: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .args(["-c", "user.name=Viem", "-c", "user.email=viem@example.invalid", "-c", "commit.gpgsign=false", "-c", "core.autocrlf=false"])
        .args(arguments)
        .current_dir(directory)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {arguments:?}: {}", String::from_utf8_lossy(&output.stderr));
}

fn repository(name: &str) -> Option<(TempDirectory, GitRepository)> {
    if !git_available() {
        eprintln!("skipping: git is not installed");
        return None;
    }
    let directory = TempDirectory::new(name);
    git(&directory.0, &["init", "-q", "-b", "main"]);
    git(&directory.0, &["config", "core.autocrlf", "false"]);
    std::fs::write(directory.0.join("a.txt"), "one\ntwo\n").unwrap();
    std::fs::write(directory.0.join("gone.txt"), "bye\n").unwrap();
    git(&directory.0, &["add", "."]);
    git(&directory.0, &["commit", "-q", "-m", "initial"]);
    let repository = GitRepository::discover(
        Path::new("git"),
        &directory.0.join("a.txt"),
        ProcessLimits::default(),
        &CancellationToken::new(),
    )
    .unwrap();
    Some((directory, repository))
}

#[test]
fn name_status_parsing_handles_renames_and_paths_with_spaces() {
    let root = Path::new("/repo");
    let output = b"M\0dir/a b.txt\0R087\0old.txt\0new.txt\0D\0gone.txt\0A\0added.txt\0";
    let files = parse_name_status_for_tests(output, root);
    assert_eq!(
        files,
        vec![
            ChangedFile { path: root.join("added.txt"), base_path: root.join("added.txt"), change: FileChange::Added },
            ChangedFile { path: root.join("dir/a b.txt"), base_path: root.join("dir/a b.txt"), change: FileChange::Modified },
            ChangedFile { path: root.join("gone.txt"), base_path: root.join("gone.txt"), change: FileChange::Deleted },
            ChangedFile { path: root.join("new.txt"), base_path: root.join("old.txt"), change: FileChange::Renamed },
        ]
    );
}

#[test]
fn file_bases_read_bytes_and_report_missing_files_as_absent() {
    let directory = TempDirectory::new("file-base");
    let path = directory.0.join(".hidden");
    std::fs::write(&path, b"\xff\xfebytes").unwrap();
    assert_eq!(read_file_base(&path).unwrap(), BaseText { bytes: b"\xff\xfebytes".to_vec(), exists: true });
    assert_eq!(read_file_base(&directory.0.join("missing")).unwrap(), BaseText { bytes: Vec::new(), exists: false });
    assert!(matches!(read_file_base(&directory.0), Err(BaseError::Unavailable(_))));
}

#[test]
fn unsaved_buffers_join_the_list_once() {
    let mut files = vec![ChangedFile { path: "/r/b".into(), base_path: "/r/b".into(), change: FileChange::Modified }];
    include_unsaved(&mut files, [PathBuf::from("/r/b"), PathBuf::from("/r/a")]);
    assert_eq!(files.iter().map(|file| (file.path.clone(), file.change)).collect::<Vec<_>>(), vec![
        (PathBuf::from("/r/a"), FileChange::Unsaved),
        (PathBuf::from("/r/b"), FileChange::Modified),
    ]);
}

#[test]
fn a_cancelled_token_stops_a_load_before_it_starts() {
    let token = CancellationToken::new();
    token.cancel();
    let result = GitRepository::discover(Path::new("git"), Path::new("/tmp/x"), ProcessLimits::default(), &token);
    assert_eq!(result, Err(BaseError::Cancelled));
}

#[test]
fn a_missing_git_program_makes_git_bases_unavailable() {
    let result = GitRepository::discover(
        Path::new("viem-no-such-git-program"),
        &std::env::temp_dir().join("x"),
        ProcessLimits::default(),
        &CancellationToken::new(),
    );
    assert!(matches!(result, Err(BaseError::Unavailable(_))), "{result:?}");
}

#[test]
fn a_folder_outside_any_repository_is_unavailable() {
    if !git_available() {
        return;
    }
    let directory = TempDirectory::new("no-repo");
    let result = GitRepository::discover(
        Path::new("git"),
        &directory.0.join("file.txt"),
        ProcessLimits::default(),
        &CancellationToken::new(),
    );
    // A temporary folder could sit inside some repository; only check that
    // discovery never fails with anything but unavailability.
    if let Err(error) = result {
        assert!(matches!(error, BaseError::Unavailable(_)), "{error:?}");
    }
}

#[test]
fn git_bases_load_index_head_and_branch_versions() {
    let Some((directory, repository)) = repository("versions") else { return };
    let root = &directory.0;
    git(root, &["branch", "old"]);
    std::fs::write(root.join("a.txt"), "one\ntwo\nthree\n").unwrap();
    git(root, &["commit", "-q", "-am", "three"]);
    std::fs::write(root.join("a.txt"), "staged\n").unwrap();
    git(root, &["add", "a.txt"]);
    std::fs::write(root.join("a.txt"), "worktree\n").unwrap();
    let token = CancellationToken::new();
    let file = root.join("a.txt");
    assert_eq!(repository.load(&BaseKind::Unstaged, &file, &token).unwrap().bytes, b"staged\n");
    assert_eq!(repository.load(&BaseKind::Head, &file, &token).unwrap().bytes, b"one\ntwo\nthree\n");
    assert_eq!(repository.load(&BaseKind::Branch("old".into()), &file, &token).unwrap().bytes, b"one\ntwo\n");
    let added = root.join("new.txt");
    std::fs::write(&added, "new\n").unwrap();
    assert_eq!(repository.load(&BaseKind::Head, &added, &token).unwrap(), BaseText { bytes: Vec::new(), exists: false });
    assert!(repository.load(&BaseKind::Branch("--output=x".into()), &file, &token).is_err());
    assert!(repository.branches(&token).unwrap().contains(&"old".to_string()));
}

#[test]
fn git_file_lists_follow_git_diff_and_ignore_untracked_files() {
    let Some((directory, repository)) = repository("lists") else { return };
    let root = std::fs::canonicalize(&directory.0).unwrap();
    std::fs::write(root.join("a.txt"), "one\nchanged\n").unwrap();
    std::fs::remove_file(root.join("gone.txt")).unwrap();
    std::fs::write(root.join("untracked.txt"), "x\n").unwrap();
    let token = CancellationToken::new();
    let files = repository.changed_files(&BaseKind::Unstaged, None, &token).unwrap();
    let canonical = |path: &Path| canonical_parent(path);
    assert_eq!(
        files.iter().map(|file| (canonical(&file.path), file.change)).collect::<Vec<_>>(),
        vec![(root.join("a.txt"), FileChange::Modified), (root.join("gone.txt"), FileChange::Deleted)]
    );
    // A staged rename shows against HEAD as one renamed entry.
    std::fs::write(root.join("a.txt"), "one\ntwo\n").unwrap();
    git(&root, &["mv", "a.txt", "renamed.txt"]);
    let files = repository.changed_files(&BaseKind::Head, None, &token).unwrap();
    let renamed = files.iter().find(|file| file.change == FileChange::Renamed).expect("rename detected");
    assert_eq!(canonical(&renamed.path), root.join("renamed.txt"));
    assert_eq!(canonical(&renamed.base_path), root.join("a.txt"));
}

#[test]
fn folder_scopes_list_only_the_files_under_the_folder() {
    let Some((directory, repository)) = repository("scoped") else { return };
    let root = std::fs::canonicalize(&directory.0).unwrap();
    std::fs::create_dir(root.join("sub dir")).unwrap();
    std::fs::write(root.join("sub dir/inner.txt"), "inner\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-q", "-m", "inner"]);
    std::fs::write(root.join("a.txt"), "outside\n").unwrap();
    std::fs::write(root.join("sub dir/inner.txt"), "changed\n").unwrap();
    let token = CancellationToken::new();
    let scoped = repository.changed_files(&BaseKind::Unstaged, Some(&root.join("sub dir")), &token).unwrap();
    assert_eq!(scoped.iter().map(|file| canonical_parent(&file.path)).collect::<Vec<_>>(), vec![root.join("sub dir/inner.txt")]);
    // The repository root scopes nothing out.
    assert_eq!(repository.changed_files(&BaseKind::Unstaged, Some(&root), &token).unwrap().len(), 2);
    // A folder is discovered directly, not through its parent.
    let from_folder = GitRepository::discover(Path::new("git"), &root.join("sub dir"), ProcessLimits::default(), &token).unwrap();
    assert_eq!(std::fs::canonicalize(from_folder.root()).unwrap(), root);
    let outside = std::env::temp_dir();
    assert!(repository.changed_files(&BaseKind::Unstaged, Some(&outside), &token).is_err());
}

#[test]
fn git_bases_apply_checkout_line_endings() {
    let Some((directory, repository)) = repository("eol") else { return };
    let root = &directory.0;
    std::fs::write(root.join(".gitattributes"), "*.txt eol=crlf\n").unwrap();
    let token = CancellationToken::new();
    let base = repository.load(&BaseKind::Head, &root.join("a.txt"), &token).unwrap();
    assert_eq!(base.bytes, b"one\r\ntwo\r\n");
}

fn canonical_parent(path: &Path) -> PathBuf {
    let parent = std::fs::canonicalize(path.parent().unwrap()).unwrap();
    parent.join(path.file_name().unwrap())
}
