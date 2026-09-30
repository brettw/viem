//! Filesystem discovery for literal Ex filename arguments. Prompt state and
//! cycling remain in the command interpreter; discovery never changes CWD.

use super::ex;
use std::fs;
use std::io;
use std::ops::Range;
use std::path::{self, Path};

#[derive(Debug, Eq, PartialEq)]
pub(super) struct FilenameCandidates {
    pub range: Range<usize>,
    pub values: Vec<String>,
}

pub(super) fn candidates(
    input: &str,
    cursor: usize,
    cwd: &Path,
    home: Option<&Path>,
) -> io::Result<Option<FilenameCandidates>> {
    let Some(argument) = ex::filename_argument(input, cursor) else {
        return Ok(None);
    };
    let fragment = &input[argument.range.clone()];
    let split = fragment
        .rfind(path::is_separator)
        .map_or(0, |index| index + 1);
    let (parent, prefix) = fragment.split_at(split);
    let home_parent = parent
        .strip_prefix('~')
        .filter(|suffix| suffix.starts_with(path::is_separator));
    let directory = if let Some(relative_home) = home_parent {
        let Some(home) = home else {
            return Ok(None);
        };
        home.join(relative_home.trim_start_matches(path::is_separator))
    } else {
        cwd.join(parent)
    };
    let folded_prefix = prefix.to_lowercase();
    let mut matches = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            // Lossy conversion could suggest a path that does not exist.
            continue;
        };
        if name.starts_with('.') && !prefix.starts_with('.') {
            continue;
        }
        let folded_name = name.to_lowercase();
        if !folded_name.starts_with(&folded_prefix) {
            continue;
        }
        let file_type = entry.file_type()?;
        let is_directory = if file_type.is_symlink() {
            match fs::metadata(entry.path()) {
                Ok(metadata) => metadata.is_dir(),
                // A dangling symlink remains a filename candidate.
                Err(error) if error.kind() == io::ErrorKind::NotFound => false,
                Err(error) => return Err(error),
            }
        } else {
            file_type.is_dir()
        };
        if argument.directories_only && !is_directory {
            continue;
        }
        // A literal entry named `~` (or `~user`) in CWD must not turn into a
        // home-directory expansion in the frontend or the next completion.
        let parent = if parent.is_empty() && name.starts_with('~') {
            "./"
        } else {
            parent
        };
        let value = format!("{parent}{name}{}", if is_directory { "/" } else { "" });
        // Ex currently trims the literal argument. Never offer a spelling
        // that would execute against a different path when Enter is pressed.
        if value.trim() != value {
            continue;
        }
        matches.push((folded_name, value));
    }
    matches.sort_unstable();
    Ok(Some(FilenameCandidates {
        range: argument.range,
        values: matches.into_iter().map(|(_, value)| value).collect(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "viem-filename-candidates-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed),
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn file(&self, name: &str) {
            fs::write(self.0.join(name), b"").unwrap();
        }

        fn directory(&self, name: &str) {
            fs::create_dir_all(self.0.join(name)).unwrap();
        }

        fn complete(&self, input: &str) -> FilenameCandidates {
            candidates(input, input.len(), &self.0, None)
                .unwrap()
                .expect("filename argument")
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn matches_case_insensitively_and_preserves_unicode_spaces_and_actual_spelling() {
        let fixture = Fixture::new();
        for name in ["zebra", "alpine", "Alpha", "École Notes.txt"] {
            fixture.file(name);
        }
        fixture.directory("Beta");
        assert_eq!(
            fixture.complete("edit ").values,
            ["Alpha", "alpine", "Beta/", "zebra", "École Notes.txt"]
        );
        assert_eq!(fixture.complete("e AL").values, ["Alpha", "alpine"]);
        assert_eq!(fixture.complete("e éc").values, ["École Notes.txt"]);
        assert_eq!(fixture.complete("e École N").values, ["École Notes.txt"]);
    }

    #[test]
    fn recognizes_only_file_arguments_from_the_ex_command_table() {
        let fixture = Fixture::new();
        fixture.file("File");
        for command in [
            "e", "ed", "edit", "E", "w", "write", "sav", "saveas", "wq", "x", "xit", "sp", "split",
            "vs", "vsplit", "e!", "E!", "w!", "saveas!", "wq!", "x!", "%w", "1,2write", ":  w",
            "  : e",
        ] {
            let input = format!("{command} f");
            let found = fixture.complete(&input);
            assert_eq!(found.values, ["File"], "{input:?}");
            assert_eq!(found.range, input.len() - 1..input.len(), "{input:?}");
        }
        for input in [
            "e",
            "edit",
            "w!",
            "sav",
            "eNotACommand f",
            "s f",
            "set f",
            "normal e f",
            "enew f",
            "update f",
            "wall f",
            "quit f",
            "pwd f",
            "1e f",
            "sp! f",
            "cd! f",
            "/f",
            "unknown f",
            "",
            ":",
            " :  ",
        ] {
            assert_eq!(
                candidates(input, input.len(), &fixture.0, None).unwrap(),
                None,
                "{input:?}"
            );
        }
    }

    #[test]
    fn cd_and_chdir_aliases_offer_only_directories() {
        let fixture = Fixture::new();
        fixture.file("Asset.txt");
        fixture.directory("Assets");
        for command in ["cd", "ch", "chd", "chdir"] {
            assert_eq!(
                fixture.complete(&format!("{command} a")).values,
                ["Assets/"]
            );
        }
        assert_eq!(fixture.complete("e a").values, ["Asset.txt", "Assets/"]);
    }

    #[test]
    fn relative_absolute_and_home_paths_keep_entered_parent_spelling() {
        let fixture = Fixture::new();
        fixture.directory("Parent/Nested");
        fixture.file("Parent/Note.txt");
        fixture.directory("Home/Private");
        fixture.file("Home/Private/Journal.txt");
        for parent in ["Parent/", "./Parent/", "Parent/Nested/../"] {
            assert_eq!(
                fixture.complete(&format!("e {parent}no")).values,
                [format!("{parent}Note.txt")]
            );
        }
        let absolute = format!("{}/Parent/", fixture.0.display());
        assert_eq!(
            fixture.complete(&format!("e {absolute}NO")).values,
            [format!("{absolute}Note.txt")]
        );
        let input = "e ~/Private/jo";
        let found = candidates(
            input,
            input.len(),
            &fixture.0,
            Some(&fixture.0.join("Home")),
        )
        .unwrap()
        .unwrap();
        assert_eq!(found.values, ["~/Private/Journal.txt"]);
        assert_eq!(found.range, 2..input.len());
        assert_eq!(
            candidates(input, input.len(), &fixture.0, None).unwrap(),
            None
        );
    }

    #[test]
    fn a_completed_directory_can_discover_its_children() {
        let fixture = Fixture::new();
        fixture.directory("Folder/Subfolder");
        fixture.file("Folder/Child.txt");
        assert_eq!(fixture.complete("e fo").values, ["Folder/"]);
        assert_eq!(
            fixture.complete("e Folder/").values,
            ["Folder/Child.txt", "Folder/Subfolder/"]
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_home_paths_accept_backslashes_and_mixed_separators() {
        let fixture = Fixture::new();
        fixture.directory("Home/Private");
        fixture.file("Home/Private/Journal.txt");
        let home = fixture.0.join("Home");
        for parent in [
            "~\\Private\\",
            "~\\Private/",
            "~/Private\\",
            "~\\\\Private\\",
            "~//Private/",
        ] {
            let input = format!("e {parent}jo");
            let found = candidates(&input, input.len(), &fixture.0, Some(&home))
                .unwrap()
                .unwrap();
            assert_eq!(found.values, [format!("{parent}Journal.txt")]);
            assert_eq!(found.range, 2..input.len());
            assert_eq!(
                candidates(&input, input.len(), &fixture.0, None).unwrap(),
                None
            );
        }
    }

    #[test]
    fn replaces_the_entire_filename_prefix_without_consuming_the_caret_suffix() {
        let fixture = Fixture::new();
        fixture.directory("Notes");
        fixture.file("Notes/École journal.txt");
        let input = ":e   Notes/éc.keep";
        let cursor = input.find(".keep").unwrap();
        let found = candidates(input, cursor, &fixture.0, None)
            .unwrap()
            .unwrap();
        assert_eq!(found.range, 5..cursor);
        assert_eq!(found.values, ["Notes/École journal.txt"]);
        let mut edited = input.to_owned();
        edited.replace_range(found.range, &found.values[0]);
        assert_eq!(edited, ":e   Notes/École journal.txt.keep");
        assert!(candidates(input, 2, &fixture.0, None).unwrap().is_none());
        assert!(candidates(input, cursor - 2, &fixture.0, None)
            .unwrap()
            .is_none());
        assert!(candidates(input, input.len() + 1, &fixture.0, None)
            .unwrap()
            .is_none());
    }

    #[test]
    fn hidden_entries_require_a_dot_prefix_and_no_matches_are_empty() {
        let fixture = Fixture::new();
        fixture.file("visible");
        fixture.file(".hidden");
        fixture.directory(".History");
        assert_eq!(fixture.complete("e ").values, ["visible"]);
        assert_eq!(fixture.complete("e .h").values, [".hidden", ".History/"]);
        assert!(fixture.complete("e missing").values.is_empty());
        assert!(candidates("e absent/file", 13, &fixture.0, None).is_err());
    }

    #[test]
    fn only_suggests_whitespace_names_that_the_literal_ex_argument_preserves() {
        let fixture = Fixture::new();
        fixture.file(" leading");
        fixture.file("interior space");
        assert_eq!(fixture.complete("e ").values, ["interior space"]);
        assert_eq!(
            fixture.complete("e ./").values,
            ["./ leading", "./interior space"]
        );
    }

    #[test]
    fn literal_tilde_entries_remain_relative_to_the_working_directory() {
        let fixture = Fixture::new();
        fixture.directory("~/Nested");
        fixture.file("~someone");
        assert_eq!(fixture.complete("e ~").values, ["./~/", "./~someone"]);
        assert_eq!(fixture.complete("e ./~/").values, ["./~/Nested/"]);
    }

    #[test]
    fn home_expansion_preserves_explicit_relative_and_embedded_tildes() {
        let fixture = Fixture::new();
        for directory in ["~/Nested", "~someone/Nested", "Folder/~/Nested", "Home/Other"] {
            fixture.directory(directory);
        }
        let home = fixture.0.join("Home");
        for parent in ["./~/", "~someone/", "Folder/~/"] {
            let input = format!("e {parent}ne");
            let found = candidates(&input, input.len(), &fixture.0, Some(&home))
                .unwrap()
                .unwrap();
            assert_eq!(found.values, [format!("{parent}Nested/")]);
        }
        #[cfg(windows)]
        for parent in [".\\~\\", "~someone\\", "Folder\\~\\"] {
            let input = format!("e {parent}ne");
            let found = candidates(&input, input.len(), &fixture.0, Some(&home))
                .unwrap()
                .unwrap();
            assert_eq!(found.values, [format!("{parent}Nested/")]);
        }
    }

    #[cfg(unix)]
    #[test]
    fn marks_directory_symlinks() {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new();
        fixture.file("File");
        fixture.directory("Target");
        symlink(fixture.0.join("Target"), fixture.0.join("Link")).unwrap();
        assert_eq!(fixture.complete("e ").values, ["File", "Link/", "Target/"]);
        assert_eq!(fixture.complete("cd l").values, ["Link/"]);
    }

    // APFS rejects non-UTF-8 filenames at creation; Unix byte filenames on
    // filesystems that allow them still must never produce lossy candidates.
    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn skips_non_utf8_names() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let fixture = Fixture::new();
        fixture.file("File");
        fs::write(fixture.0.join(OsString::from_vec(b"Bad\xff".to_vec())), b"").unwrap();
        assert_eq!(fixture.complete("e ").values, ["File"]);
    }
}
