use super::git::parse_file_stats;
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
fn git_statistics_preserve_rename_paths_binary_files_and_line_counts() {
    let root = Path::new("/repo");
    let output = b":a M\0dir/a b.txt\0:a R087\0old.txt\0new.txt\0:a D\0gone.txt\0:a A\0added.txt\0-\t-\tadded.txt\02\t3\tdir/a b.txt\00\t4\tgone.txt\01\t2\t\0old.txt\0new.txt\0";
    let files = parse_file_stats(output, root).unwrap();
    assert_eq!(
        files,
        vec![
            ChangedFile { path: root.join("added.txt"), base_path: root.join("added.txt"), change: FileChange::Added, added: 0, removed: 0 },
            ChangedFile { path: root.join("dir/a b.txt"), base_path: root.join("dir/a b.txt"), change: FileChange::Modified, added: 2, removed: 3 },
            ChangedFile { path: root.join("gone.txt"), base_path: root.join("gone.txt"), change: FileChange::Deleted, added: 0, removed: 4 },
            ChangedFile { path: root.join("new.txt"), base_path: root.join("old.txt"), change: FileChange::Renamed, added: 1, removed: 2 },
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
    let mut files = vec![ChangedFile { path: "/r/b".into(), base_path: "/r/b".into(), change: FileChange::Modified, added: 2, removed: 3 }];
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
fn commit_picker_lists_fifty_from_head_and_resolves_an_older_hash() {
    use std::io::Write;
    use std::process::Stdio;
    let Some((directory, repo)) = repository("commits") else { return };
    let root = &directory.0;
    let token = CancellationToken::new();
    let original = repo.commits(None, &token).unwrap().remove(0);
    let mut history = String::new();
    for i in 1..=52 {
        let title = format!("revision {i}");
        history.push_str(&format!("commit refs/heads/history\ncommitter Test <t@example.invalid> {i} +0000\ndata {}\n{title}\n", title.len()));
        if i == 1 { history.push_str(&format!("from {}\n", original.hash)); }
        history.push('\n');
    }
    let mut import = Command::new("git").args(["fast-import", "--quiet"]).current_dir(root).stdin(Stdio::piped()).spawn().unwrap();
    import.stdin.take().unwrap().write_all(history.as_bytes()).unwrap();
    assert!(import.wait().unwrap().success());
    assert_eq!(repo.commits(None, &token).unwrap().len(), 1, "unmerged branches are excluded");
    git(root, &["checkout", "-q", "history"]);
    let recent = repo.commits(None, &token).unwrap();
    assert_eq!(recent.len(), 50);
    assert_eq!(recent[0].title, "revision 52");
    assert_eq!(recent[49].title, "revision 3");
    for hash in [&original.hash, &original.short] {
        let resolved = repo.commits(Some(hash), &token).unwrap();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].hash, original.hash);
    }
    let base = BaseKind::Commit(original.hash);
    std::fs::write(root.join("a.txt"), "changed\n").unwrap();
    assert_eq!(repo.changed_files(&base, None, &token).unwrap().len(), 1);
    assert_eq!(repo.load(&base, &root.join("a.txt"), &token).unwrap().bytes, b"one\ntwo\n");
    for invalid in ["HEAD", "--all", "123", "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"] {
        assert!(repo.commits(Some(invalid), &token).is_err());
    }
}

#[test]
fn git_bases_load_index_head_and_branch_versions() {
    let Some((directory, repository)) = repository("versions") else { return };
    let root = &directory.0;
    git(root, &["branch", "old"]);
    std::fs::write(root.join("a.txt"), "one\ntwo\nthree\n").unwrap();
    git(root, &["commit", "-q", "-am", "three"]);
    git(root, &["tag", "old"]);
    git(root, &["branch", "topic/HEAD"]);
    git(root, &["update-ref", "refs/remotes/origin/remote-only", "HEAD"]);
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
    git(root, &["config", "branch.sort", "-refname"]);
    git(root, &["checkout", "-q", "--detach"]);
    assert_eq!(repository.branches(&token).unwrap(), ["topic/HEAD", "old", "main"]);
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
fn listed_files_keep_the_current_buffer_name_through_a_directory_alias() {
    let Some((directory, repository)) = repository("open-alias") else { return };
    let root = &directory.0;
    let link = root.join("alias");
    #[cfg(unix)]
    std::os::unix::fs::symlink(root, &link).unwrap();
    #[cfg(windows)]
    let link = if std::os::windows::fs::symlink_dir(root, &link).is_ok() { link } else {
        // A parent traversal also exercises path normalization without symlink privileges.
        std::fs::create_dir(&link).unwrap();
        link.join("..")
    };
    std::fs::write(root.join("a.txt"), "changed\n").unwrap();
    std::fs::remove_file(root.join("gone.txt")).unwrap();
    let current = link.join("gone.txt");
    let files = repository.files_for_document(
        &BaseKind::Head, &current, &[link.join("a.txt")], Some(root), &CancellationToken::new(),
    ).unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].path, link.join("a.txt"));
    assert_eq!(files[0].change, FileChange::Modified);
    assert_eq!(files[1].path, current);
    assert_eq!(files[1].base_path, current);
    assert_eq!(files[1].change, FileChange::Deleted);
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

#[test]
fn folder_relative_loads_need_no_repository_discovery() {
    let Some((directory, _)) = repository("relative") else { return };
    let root = std::fs::canonicalize(&directory.0).unwrap();
    std::fs::create_dir(root.join("gone dir")).unwrap();
    std::fs::write(root.join("gone dir/inner.txt"), "inner\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-q", "-m", "inner"]);
    std::fs::write(root.join("a.txt"), "worktree\n").unwrap();
    let token = CancellationToken::new();
    let load = |kind: &BaseKind, file: &Path| GitRepository::load_file(Path::new("git"), kind, file, ProcessLimits::default(), &token);
    assert_eq!(load(&BaseKind::Unstaged, &root.join("a.txt")).unwrap().bytes, b"one\ntwo\n");
    // A deleted folder's file still has its committed version.
    std::fs::remove_dir_all(root.join("gone dir")).unwrap();
    assert_eq!(load(&BaseKind::Head, &root.join("gone dir/inner.txt")).unwrap().bytes, b"inner\n");
    // Directories and names git does not know are absent, not errors.
    assert!(!load(&BaseKind::Head, &root.join("new.txt")).unwrap().exists);
    std::fs::create_dir(root.join("folder")).unwrap();
    assert!(!load(&BaseKind::Unstaged, &root.join("folder")).unwrap().exists);
    let outside = TempDirectory::new("outside");
    std::fs::write(outside.0.join("x.txt"), "x\n").unwrap();
    assert!(matches!(load(&BaseKind::Unstaged, &outside.0.join("x.txt")), Err(BaseError::Unavailable(_))));
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(matches!(
        GitRepository::load_file(Path::new("git"), &BaseKind::Head, &root.join("a.txt"), ProcessLimits::default(), &cancelled),
        Err(BaseError::Cancelled)
    ));
}
