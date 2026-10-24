//! Portable register context and session file-mark locations. Hosts own opening
//! files; mark ownership, rebasing and cursor targets remain in the core.
use super::*;

#[derive(Clone, Debug)]
pub(crate) struct FileMarkTarget {
    pub generation: u64,
    pub document: DocumentId,
    pub path: Option<crate::document::ArtifactPath>,
    pub location: Option<(usize, usize)>,
    pub deleted: bool,
}

impl<P: TextMeasurementProvider> Core<P> {
    pub(crate) fn sync_register_filename(&mut self) {
        let filename = self.buffer_filename();
        let path =
            (!filename.is_empty()).then(|| crate::document::ArtifactPath::from_utf8(filename));
        for view in self.views.values_mut() {
            view.commands.set_current_artifact(path.clone());
        }
    }
    pub fn set_alternate_artifact(
        &mut self,
        view: ViewId,
        path: Option<crate::document::ArtifactPath>,
    ) -> Result<(), CoreError> {
        self.views
            .get_mut(&view)
            .ok_or(CoreError::UnknownView(view))?
            .commands
            .set_alternate_artifact(path);
        Ok(())
    }
    pub fn set_drop_register(
        &mut self,
        view: ViewId,
        value: crate::command::RegisterValue,
    ) -> Result<(), CoreError> {
        if !self.views.contains_key(&view) {
            return Err(CoreError::UnknownView(view));
        }
        for view in self.views.values_mut() {
            view.commands.set_drop_register(value.clone());
        }
        self.publish_buffer_commands(view);
        Ok(())
    }
    /// This buffer's copy of the application-wide register bank.
    pub(crate) fn registers(&self) -> &crate::command::Registers {
        &self.buffer_commands.registers
    }
    /// Install a newer application register bank in the buffer and each view.
    /// Special registers kept by the buffer or view (`%`, `#`, `:`, `/`, `=`,
    /// `~`) and an in-progress recording are untouched.
    pub(crate) fn adopt_registers(&mut self, registers: &crate::command::Registers) {
        self.buffer_commands.registers.clone_from(registers);
        for view in self.views.values_mut() {
            view.commands.install_registers(registers);
        }
    }
    pub(crate) fn set_file_mark_ownership(&mut self, mask: u32) {
        for view in self.views.values_mut() {
            view.commands.set_file_mark_ownership(mask);
        }
    }
    pub(crate) fn file_mark_targets(&self) -> Vec<(char, FileMarkTarget)> {
        self.buffer_commands
            .file_mark_generations
            .iter()
            .map(|(name, generation)| {
                let offset = self.buffer_commands.marks.get(name).copied();
                let location = offset.and_then(|offset| {
                    let source = self
                        .document
                        .projection()
                        .source_insertion_point(offset, true)?;
                    let line = self.document.physical_line_index_at_source(source).ok()?;
                    let (_, content) = self.document.physical_line_ranges(line).ok()?;
                    Some((line, source - content.start))
                });
                (
                    *name,
                    FileMarkTarget {
                        generation: *generation,
                        document: self.document.id(),
                        path: self
                            .document
                            .artifact_binding()
                            .map(|binding| binding.path().clone())
                            .or_else(|| {
                                (!self.buffer_filename().is_empty()).then(|| {
                                    crate::document::ArtifactPath::from_utf8(self.buffer_filename())
                                })
                            }),
                        location,
                        deleted: offset.is_none(),
                    },
                )
            })
            .collect()
    }
    pub(crate) fn jump_file_mark(
        &mut self,
        view: ViewId,
        name: char,
        target: &FileMarkTarget,
        linewise: bool,
    ) -> Result<CoreOutcome, CoreError> {
        if target.deleted {
            return Err(CoreError::Document(DocumentError::AmbiguousProjection));
        }
        let commands = &self
            .views
            .get(&view)
            .ok_or(CoreError::UnknownView(view))?
            .commands;
        let offset = if target.document == self.document.id() {
            commands
                .mark_offset(name)
                .ok_or(CoreError::Document(DocumentError::AmbiguousProjection))?
        } else {
            // A reopened file has a fresh document identity. Resolve the saved
            // physical file location in that artifact; never reuse a TextPoint.
            let path = self
                .document
                .artifact_binding()
                .map(|binding| binding.path().clone())
                .or_else(|| {
                    (!self.buffer_filename().is_empty())
                        .then(|| crate::document::ArtifactPath::from_utf8(self.buffer_filename()))
                });
            if target.path.as_ref() != path.as_ref() {
                return Err(CoreError::Document(DocumentError::AmbiguousProjection));
            }
            let (line, column) = target
                .location
                .ok_or(CoreError::Document(DocumentError::AmbiguousProjection))?;
            let (_, content) = self.document.physical_line_ranges(line)?;
            let source = content
                .start
                .checked_add(column)
                .filter(|source| *source <= content.end)
                .ok_or(CoreError::Document(DocumentError::AmbiguousProjection))?;
            self.document
                .map_source_point(
                    self.document.source_point(source)?,
                    BoundaryAffinity::Downstream,
                )
                .map_err(|_| CoreError::Document(DocumentError::AmbiguousProjection))?
                .text()
                .offset()
        };
        let marked_offset = offset;
        let offset = commands.mark_destination(&self.document, offset, linewise);
        let origin = commands.cursor();
        let outcome = self.handle(
            view,
            CoreEvent::PlaceCursor {
                document_revision: self.document.revision(),
                text_offset: offset,
                affinity: BoundaryAffinity::Downstream,
                extend_selection: false,
            },
        )?;
        let commands = &mut self.views.get_mut(&view).expect("validated view").commands;
        commands.restore_file_mark(
            name,
            if target.document == self.document.id() {
                target.generation
            } else {
                0
            },
            marked_offset,
        );
        commands.note_external_jump(&self.document, origin);
        self.publish_buffer_commands(view);
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::MockTextMeasurementProvider;
    fn keys(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, text: &str) {
        for ch in text.chars() {
            core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char(ch))))
                .unwrap();
        }
    }
    #[test]
    fn adopted_register_banks_reach_every_view_and_leave_buffer_registers_local() {
        let mut first = Core::new(Document::new("alpha\nbeta\n"));
        let source = first.add_view(MockTextMeasurementProvider::new(), 600., 500.);
        first
            .set_drop_register(source, crate::command::RegisterValue::characterwise("dropped"))
            .unwrap();
        keys(&mut first, source, "yyj\"qyy");
        let mut second = Core::new(Document::new("gamma\n"));
        let left = second.add_view(MockTextMeasurementProvider::new(), 600., 500.);
        let right = second.add_view(MockTextMeasurementProvider::new(), 300., 500.);

        second.adopt_registers(first.registers());
        assert_eq!(second.registers().revision(), first.registers().revision());
        for view in [left, right] {
            let commands = second.command_state(view).unwrap();
            assert_eq!(commands.register('q').unwrap().text, "beta\n");
            assert_eq!(commands.register('0').unwrap().text, "alpha\n");
            assert!(!commands.stored_register_names().contains(&'~'));
        }
        keys(&mut second, right, "\"0p");
        assert_eq!(second.document().text(), "gamma\nalpha\n");
        assert!(first.command_state(source).unwrap().stored_register_names().contains(&'~'));
    }

    #[test]
    fn file_mark_snapshots_do_not_decode_or_materialize_huge_lines() {
        let mut core = Core::new(Document::new("x".repeat(2_000_000)));
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 500.);
        keys(&mut core, view, "lmA");
        let (marks, work) = crate::document::measure_document_work(|| core.file_mark_targets());
        assert_eq!(marks[0].1.location, Some((0, 1)));
        assert_eq!(work.source_full_materializations, 0, "{work:?}");
        assert_eq!(work.source_decoded_bytes, 0, "{work:?}");
        assert!(work.source_range_materialized_bytes <= 8, "{work:?}");
    }

    #[test]
    fn reopened_file_marks_resolve_fresh_physical_coordinates_and_reject_short_files() {
        use crate::document::{
            ArtifactBinding, ArtifactIdentity, ArtifactPath, Encoding, Format,
            LineEndingOpenPolicy, LoadedArtifact,
        };
        let open = |text: &[u8]| {
            let document = Document::from_loaded_artifact(
                LoadedArtifact::new(
                    ArtifactBinding::new(
                        ArtifactPath::from_utf8("/tmp/file-mark.txt"),
                        ArtifactIdentity::new(b"file".to_vec()),
                    ),
                    text.to_vec(),
                ),
                Encoding::Utf8,
                Format::PlainText,
                LineEndingOpenPolicy::default(),
            )
            .unwrap();
            let mut core = Core::new(document);
            let view = core.add_view(MockTextMeasurementProvider::new(), 600., 500.);
            (core, view)
        };
        let (mut original, view) = open(b"one\r\n  two");
        keys(&mut original, view, "jllmB");
        let target = original.file_mark_targets().pop().unwrap().1;
        drop(original);
        let (mut reopened, view) = open(b"one\r\n  two");
        reopened.jump_file_mark(view, 'B', &target, false).unwrap();
        assert_eq!(reopened.command_state(view).unwrap().cursor(), 6);
        assert!(reopened.file_mark_targets().pop().unwrap().1.generation > target.generation);
        let (mut shortened, view) = open(b"one");
        let before = shortened.document().source_bytes();
        assert!(shortened.jump_file_mark(view, 'B', &target, false).is_err());
        assert_eq!(shortened.command_state(view).unwrap().cursor(), 0);
        assert_eq!(shortened.document().source_bytes(), before);
    }

    #[test]
    fn file_marks_rebase_share_ownership_and_resolve_live_targets() {
        let mut core = Core::new(Document::new("one\ntwo"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 600., 500.);
        keys(&mut core, view, "jllmA");
        let initial = core.file_mark_targets().pop().unwrap().1;
        assert_eq!(initial.location, Some((1, 2)));
        keys(&mut core, view, "ggix");
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        core.jump_file_mark(view, 'A', &initial, false).unwrap();
        assert_eq!(core.command_state(view).unwrap().cursor(), 7);
        let second = core.add_view(MockTextMeasurementProvider::new(), 300., 400.);
        core.jump_file_mark(second, 'A', &initial, true).unwrap();
        assert_eq!(core.command_state(second).unwrap().cursor(), 5);
        assert_eq!(
            core.file_mark_targets().pop().unwrap().1.location,
            Some((1, 2)),
            "linewise jumping does not overwrite the mark column"
        );
        core.set_file_mark_ownership(0);
        assert!(core.command_state(view).unwrap().mark_offset('A').is_none());
        keys(&mut core, view, "mA");
        assert!(core.file_mark_targets().pop().unwrap().1.generation > initial.generation);
    }
}
