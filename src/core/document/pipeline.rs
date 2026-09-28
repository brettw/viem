//! Observable contracts for the currently fused document transformation
//! pipeline.
//!
//! The plain-text and Markdown implementations deliberately perform several
//! stages in one optimized code path.  This module keeps their individual
//! identities, edit responsibilities, capability decisions, and invalidation
//! behavior visible so fusion is an implementation detail rather than an
//! architectural shortcut.

use super::{
    Document, DocumentError, DocumentId, Encoding, FileFormat, Format, HardLineSnapshot,
    PositionDomain, PositionError, Revision, SemanticInlineStyle, StyleId, StyleSheetRevision,
    TextRange,
};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

/// Stable identity for a transformation implementation. Versions change when
/// the same implementation identity changes observable projection or reverse-
/// edit semantics.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub struct TransformationIdentity {
    pub name: &'static str,
    pub version: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransformationStageRole {
    EncodingProjection,
    LineEndingInterpretation,
    LosslessFormatProjection,
    FormattedDocumentAssembly,
}

/// Directionality promised by one stage. The enum leaves room for generated
/// transformations even though the two initial pipelines contain only editable
/// stages.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransformationDirectionality {
    Reversible,
    EditableProjection,
    OneWayGenerated,
}

/// Configuration values which can change a projection without changing source
/// bytes.  This value is part of every stage snapshot identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PipelineConfigurationIdentity {
    pub encoding: Encoding,
    pub format: Format,
    pub file_format: FileFormat,
    pub style_sheet_revision: StyleSheetRevision,
}

/// Only configuration inputs relevant to one stage participate in that
/// stage's cache identity. A style-sheet edit, for example, must not make
/// decoding or lossless syntax parsing stale.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransformationStageConfiguration {
    Encoding {
        encoding: Encoding,
    },
    LineEndings {
        file_format: FileFormat,
    },
    Format {
        format: Format,
    },
    FormattedDocument {
        format: Format,
        style_sheet_revision: StyleSheetRevision,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransformationStageSnapshot {
    pub identity: TransformationIdentity,
    pub role: TransformationStageRole,
    pub directionality: TransformationDirectionality,
    pub source_revision: Revision,
    pub configuration: TransformationStageConfiguration,
    /// Stages with the same nonzero execution group are currently evaluated by
    /// one fused implementation. Their contracts and reports remain distinct.
    pub fused_execution_group: u32,
}

/// A typed normalized edit category queried independently of command spelling.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PipelineEditIntent {
    ReplaceText {
        replacement: String,
    },
    InsertHardBreak,
    SetSemanticInlineStyle {
        style: SemanticInlineStyle,
        enabled: bool,
    },
    AssignBlockStyle {
        style: StyleId,
    },
    AssignCharacterStyle {
        style: StyleId,
    },
    SetStrikethrough {
        enabled: bool,
    },
    EditConfigurationBlockStyleDefinition {
        style: StyleId,
    },
    EditConfigurationCharacterStyleDefinition {
        style: StyleId,
    },
    ConfigureDocumentStyle {
        style: StyleId,
    },
    ConfigureDocumentCanvas,
    ConfigureDocumentDefaultCharacter,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnsupportedEditReason {
    PlainTextHasNoRichStyleStorage,
    FormatHasNoNamedStyleStorage,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PipelinePolicyRequest {
    /// The replacement cannot be represented exactly in the authoritative
    /// encoding. The caller must choose conversion, a format escape when one is
    /// semantically exact, or rejection.
    UnrepresentableCharacter { encoding: Encoding, character: char },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StageEditDisposition {
    Translated,
    PassThrough,
    Unsupported(UnsupportedEditReason),
    NeedsPolicy(PipelinePolicyRequest),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StageCapabilityReport {
    pub stage: TransformationIdentity,
    pub disposition: StageEditDisposition,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PipelineCapabilityDecision {
    Supported,
    Unsupported {
        blocking_stage: TransformationIdentity,
        reason: UnsupportedEditReason,
    },
    NeedsPolicy {
        blocking_stage: TransformationIdentity,
        request: PipelinePolicyRequest,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PipelineCapabilityReport {
    pub document: DocumentId,
    pub revision: Revision,
    pub range: TextRange,
    pub stages: Vec<StageCapabilityReport>,
    pub decision: PipelineCapabilityDecision,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PipelineInvalidationReason {
    SourceBytesChanged,
    DecoderStateMayChange,
    HardLineInterpretationMayChange,
    FormatStateMayChange,
    ConfigurationChanged,
    UpstreamProjectionChanged,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PipelineInvalidationScope {
    None,
    /// Exact source ranges are independently restartable.
    SourceRanges(Vec<Range<usize>>),
    /// Restart at or before this source offset and continue until stage state
    /// converges with an unchanged checkpoint.
    RestartUntilConvergence {
        source_offset: usize,
    },
    FullProjection,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StageInvalidation {
    pub stage: TransformationIdentity,
    pub scope: PipelineInvalidationScope,
    pub reasons: Vec<PipelineInvalidationReason>,
}

/// Facts about an input transition supplied to the pipeline invalidator. The
/// format adapter, rather than layout or a frontend, decides how far a restart
/// must propagate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PipelineInputChange {
    pub source_ranges: Vec<Range<usize>>,
    pub decoder_state_may_change: bool,
    pub hard_lines_may_change: bool,
    pub configuration_changed: bool,
}

impl PipelineInputChange {
    pub fn source_ranges(source_ranges: Vec<Range<usize>>) -> Self {
        Self {
            source_ranges,
            decoder_state_may_change: false,
            hard_lines_may_change: false,
            configuration_changed: false,
        }
    }
}

/// Immutable, cheap snapshot of the pipeline contract and the formatted
/// boundaries against which capability requests are validated.
#[derive(Clone, Debug)]
pub struct TransformationPipelineSnapshot {
    document: DocumentId,
    revision: Revision,
    configuration: PipelineConfigurationIdentity,
    stages: Vec<TransformationStageSnapshot>,
    hard_lines: HardLineSnapshot,
    /// Persistent regional context distinguishes escapable Markdown prose from
    /// literal code without materializing the document's compatibility text.
    markdown_projection: Option<super::FormattedDocument>,
}

impl TransformationPipelineSnapshot {
    pub fn document(&self) -> DocumentId {
        self.document
    }

    pub fn revision(&self) -> Revision {
        self.revision
    }

    pub fn configuration(&self) -> PipelineConfigurationIdentity {
        self.configuration
    }

    pub fn stages(&self) -> &[TransformationStageSnapshot] {
        &self.stages
    }

    /// Compose each stage's reverse-edit decision. The first blocking stage in
    /// reverse-projection order determines the result, while the full report
    /// remains available for diagnostics and UI affordances.
    pub fn capabilities(
        &self,
        range: TextRange,
        intent: &PipelineEditIntent,
    ) -> Result<PipelineCapabilityReport, PositionError> {
        self.validate_range(range)?;
        let mut reports = Vec::with_capacity(self.stages.len());
        let mut decision = PipelineCapabilityDecision::Supported;

        // Reverse projection walks the conceptual stages from output to source.
        for stage in self.stages.iter().rev() {
            let disposition = self.stage_disposition(stage.role, range, intent);
            reports.push(StageCapabilityReport {
                stage: stage.identity,
                disposition,
            });
            match disposition {
                StageEditDisposition::Unsupported(reason)
                    if decision == PipelineCapabilityDecision::Supported =>
                {
                    decision = PipelineCapabilityDecision::Unsupported {
                        blocking_stage: stage.identity,
                        reason,
                    };
                }
                StageEditDisposition::NeedsPolicy(request)
                    if decision == PipelineCapabilityDecision::Supported =>
                {
                    decision = PipelineCapabilityDecision::NeedsPolicy {
                        blocking_stage: stage.identity,
                        request,
                    };
                }
                _ => {}
            }
        }

        Ok(PipelineCapabilityReport {
            document: self.document,
            revision: self.revision,
            range,
            stages: reports,
            decision,
        })
    }

    /// Propagate one input change through each fused-but-observable stage.
    pub fn invalidate(&self, change: &PipelineInputChange) -> Vec<StageInvalidation> {
        if change.configuration_changed {
            return self
                .stages
                .iter()
                .map(|stage| StageInvalidation {
                    stage: stage.identity,
                    scope: PipelineInvalidationScope::FullProjection,
                    reasons: vec![PipelineInvalidationReason::ConfigurationChanged],
                })
                .collect();
        }
        if change.source_ranges.is_empty() {
            return self
                .stages
                .iter()
                .map(|stage| StageInvalidation {
                    stage: stage.identity,
                    scope: PipelineInvalidationScope::None,
                    reasons: Vec::new(),
                })
                .collect();
        }

        let first_changed = change
            .source_ranges
            .iter()
            .map(|range| range.start)
            .min()
            .expect("a nonempty range list has a first offset");
        self.stages
            .iter()
            .map(|stage| {
                let (scope, mut reasons) = match stage.role {
                    TransformationStageRole::EncodingProjection => {
                        if change.decoder_state_may_change {
                            (
                                PipelineInvalidationScope::RestartUntilConvergence {
                                    source_offset: first_changed,
                                },
                                vec![
                                    PipelineInvalidationReason::SourceBytesChanged,
                                    PipelineInvalidationReason::DecoderStateMayChange,
                                ],
                            )
                        } else {
                            (
                                PipelineInvalidationScope::SourceRanges(
                                    change.source_ranges.clone(),
                                ),
                                vec![PipelineInvalidationReason::SourceBytesChanged],
                            )
                        }
                    }
                    TransformationStageRole::LineEndingInterpretation => {
                        if change.decoder_state_may_change || change.hard_lines_may_change {
                            let mut reasons =
                                vec![PipelineInvalidationReason::UpstreamProjectionChanged];
                            if change.hard_lines_may_change {
                                reasons.push(
                                    PipelineInvalidationReason::HardLineInterpretationMayChange,
                                );
                            }
                            (
                                PipelineInvalidationScope::RestartUntilConvergence {
                                    source_offset: first_changed,
                                },
                                reasons,
                            )
                        } else {
                            (
                                PipelineInvalidationScope::SourceRanges(
                                    change.source_ranges.clone(),
                                ),
                                vec![PipelineInvalidationReason::UpstreamProjectionChanged],
                            )
                        }
                    }
                    TransformationStageRole::LosslessFormatProjection => {
                        if self.configuration.format != Format::PlainText
                            || change.decoder_state_may_change
                            || change.hard_lines_may_change
                        {
                            let mut reasons =
                                vec![PipelineInvalidationReason::UpstreamProjectionChanged];
                            if self.configuration.format != Format::PlainText {
                                reasons.push(PipelineInvalidationReason::FormatStateMayChange);
                            }
                            (
                                PipelineInvalidationScope::RestartUntilConvergence {
                                    source_offset: first_changed,
                                },
                                reasons,
                            )
                        } else {
                            (
                                PipelineInvalidationScope::SourceRanges(
                                    change.source_ranges.clone(),
                                ),
                                vec![PipelineInvalidationReason::UpstreamProjectionChanged],
                            )
                        }
                    }
                    TransformationStageRole::FormattedDocumentAssembly => (
                        if self.configuration.format != Format::PlainText
                            || change.decoder_state_may_change
                            || change.hard_lines_may_change
                        {
                            PipelineInvalidationScope::RestartUntilConvergence {
                                source_offset: first_changed,
                            }
                        } else {
                            PipelineInvalidationScope::SourceRanges(change.source_ranges.clone())
                        },
                        vec![PipelineInvalidationReason::UpstreamProjectionChanged],
                    ),
                };
                reasons.dedup();
                StageInvalidation {
                    stage: stage.identity,
                    scope,
                    reasons,
                }
            })
            .collect()
    }

    fn validate_range(&self, range: TextRange) -> Result<(), PositionError> {
        let start = range.start();
        let end = range.end();
        if start.document() != self.document {
            return Err(PositionError::WrongDocument {
                expected: self.document,
                actual: start.document(),
            });
        }
        if start.revision() != self.revision {
            return Err(PositionError::WrongSnapshot {
                expected: self.revision,
                actual: start.revision(),
            });
        }
        for offset in [start.offset(), end.offset()] {
            if offset > self.hard_lines.text_length() {
                return Err(PositionError::InvalidBoundary {
                    domain: PositionDomain::FormattedText,
                    offset,
                    length: self.hard_lines.text_length(),
                });
            }
            if !self.hard_lines.is_grapheme_boundary(offset) {
                return Err(PositionError::InvalidUnicodeBoundary { offset });
            }
        }
        Ok(())
    }

    fn stage_disposition(
        &self,
        role: TransformationStageRole,
        range: TextRange,
        intent: &PipelineEditIntent,
    ) -> StageEditDisposition {
        match role {
            TransformationStageRole::FormattedDocumentAssembly => StageEditDisposition::Translated,
            TransformationStageRole::LosslessFormatProjection => {
                self.format_stage_disposition(intent)
            }
            TransformationStageRole::LineEndingInterpretation => match intent {
                PipelineEditIntent::InsertHardBreak => StageEditDisposition::Translated,
                _ => StageEditDisposition::PassThrough,
            },
            TransformationStageRole::EncodingProjection => match intent {
                PipelineEditIntent::ReplaceText { replacement } => {
                    let literal = self.markdown_literal_replacement_text(range, replacement);
                    match self
                        .configuration
                        .encoding
                        .encode_fragment(literal.as_deref().unwrap_or(replacement))
                    {
                        Ok(_) => StageEditDisposition::Translated,
                        Err(DocumentError::UnrepresentableCharacter {
                            encoding,
                            character,
                        }) => StageEditDisposition::NeedsPolicy(
                            PipelinePolicyRequest::UnrepresentableCharacter {
                                encoding,
                                character,
                            },
                        ),
                        Err(_) => {
                            // `encode_fragment` currently has only the exact
                            // unrepresentable-text failure. Keep a deterministic
                            // conservative result if that contract grows.
                            StageEditDisposition::PassThrough
                        }
                    }
                }
                _ => StageEditDisposition::PassThrough,
            },
        }
    }

    /// Mirror the Markdown translator's line-local source-run distribution.
    /// Prose segments become character references before encoding; only code
    /// segments still require their literal characters to be representable.
    fn markdown_literal_replacement_text(
        &self,
        range: TextRange,
        replacement: &str,
    ) -> Option<String> {
        if self.configuration.encoding != Encoding::Latin1
            || replacement.chars().all(|ch| ch as u32 <= 0xff)
        {
            return None;
        }
        let projection = self.markdown_projection.as_ref()?;
        let range = range.start().offset()..range.end().offset();
        if !replacement.contains('\n') {
            if let Some(runs) = projection.line_local_visible_source_runs(range.clone()) {
                let graphemes = replacement.graphemes(true).collect::<Vec<_>>();
                let mut at = 0;
                let mut literal = String::new();
                let last = runs.len() - 1;
                for (index, run) in runs.into_iter().enumerate() {
                    let take = if index == last {
                        graphemes.len() - at
                    } else {
                        projection
                            .text_tree()
                            .slice(run.formatted.clone())
                            .ok()?
                            .graphemes(true)
                            .count()
                            .min(graphemes.len() - at)
                    };
                    if projection.markdown_replacement_begins_in_code(&run.formatted) {
                        for grapheme in &graphemes[at..at + take] {
                            literal.push_str(grapheme);
                        }
                    }
                    at += take;
                }
                return Some(literal);
            }
        }
        Some(if projection.markdown_replacement_begins_in_code(&range) {
            replacement.to_owned()
        } else {
            String::new()
        })
    }

    fn format_stage_disposition(&self, intent: &PipelineEditIntent) -> StageEditDisposition {
        match intent {
            PipelineEditIntent::ReplaceText { .. } | PipelineEditIntent::InsertHardBreak => {
                StageEditDisposition::Translated
            }
            PipelineEditIntent::EditConfigurationBlockStyleDefinition { .. }
            | PipelineEditIntent::EditConfigurationCharacterStyleDefinition { .. }
            | PipelineEditIntent::ConfigureDocumentStyle { .. }
            | PipelineEditIntent::ConfigureDocumentCanvas
            | PipelineEditIntent::ConfigureDocumentDefaultCharacter => {
                StageEditDisposition::PassThrough
            }
            PipelineEditIntent::SetSemanticInlineStyle { .. }
            | PipelineEditIntent::SetStrikethrough { .. } => {
                if self.configuration.format.is_markdown() {
                    StageEditDisposition::Translated
                } else {
                    StageEditDisposition::Unsupported(
                        UnsupportedEditReason::PlainTextHasNoRichStyleStorage,
                    )
                }
            }
            PipelineEditIntent::AssignBlockStyle { style }
                if self.configuration.format.is_markdown()
                    && (matches!(style.0.as_str(), "Paragraph" | "Block quote" | "Code Block")
                        || style
                            .0
                            .strip_prefix("Heading")
                            .and_then(|level| level.parse::<u8>().ok())
                            .is_some_and(|level| (1..=6).contains(&level))) =>
            {
                StageEditDisposition::Translated
            }
            PipelineEditIntent::AssignCharacterStyle { style }
                if self.configuration.format.is_markdown()
                    && matches!(style.0.as_str(), "Code" | "") =>
            {
                StageEditDisposition::Translated
            }
            PipelineEditIntent::AssignBlockStyle { .. }
            | PipelineEditIntent::AssignCharacterStyle { .. } => StageEditDisposition::Unsupported(
                if self.configuration.format == Format::PlainText {
                    UnsupportedEditReason::PlainTextHasNoRichStyleStorage
                } else {
                    UnsupportedEditReason::FormatHasNoNamedStyleStorage
                },
            ),
        }
    }
}

impl Document {
    /// Capture the current transformation graph without retaining mutable
    /// buffer state. The returned snapshot remains queryable after this
    /// document advances; requests must use ranges from the captured revision.
    pub fn transformation_pipeline_snapshot(&self) -> TransformationPipelineSnapshot {
        let configuration = PipelineConfigurationIdentity {
            encoding: self.encoding(),
            format: self.format(),
            file_format: self.file_format(),
            style_sheet_revision: self.projection().style_sheet().revision,
        };
        let source_revision = self.revision();
        let stages = [
            (
                TransformationIdentity {
                    name: "builtin.encoding",
                    version: 1,
                },
                TransformationStageRole::EncodingProjection,
                TransformationStageConfiguration::Encoding {
                    encoding: self.encoding(),
                },
            ),
            (
                TransformationIdentity {
                    name: "builtin.line-endings",
                    version: 1,
                },
                TransformationStageRole::LineEndingInterpretation,
                TransformationStageConfiguration::LineEndings {
                    file_format: self.file_format(),
                },
            ),
            (
                TransformationIdentity {
                    name: match self.format() {
                        Format::PlainText => "builtin.plain-text",
                        Format::Code => "builtin.code",
                        Format::Markdown => "builtin.markdown",
                        Format::MarkdownSource => "builtin.markdown-source",
                    },
                    version: 1,
                },
                TransformationStageRole::LosslessFormatProjection,
                TransformationStageConfiguration::Format {
                    format: self.format(),
                },
            ),
            (
                TransformationIdentity {
                    name: "builtin.formatted-document",
                    version: 1,
                },
                TransformationStageRole::FormattedDocumentAssembly,
                TransformationStageConfiguration::FormattedDocument {
                    format: self.format(),
                    style_sheet_revision: self.projection().style_sheet().revision,
                },
            ),
        ]
        .into_iter()
        .map(
            |(identity, role, configuration)| TransformationStageSnapshot {
                identity,
                role,
                directionality: TransformationDirectionality::EditableProjection,
                source_revision,
                configuration,
                fused_execution_group: 1,
            },
        )
        .collect();

        TransformationPipelineSnapshot {
            document: self.id(),
            revision: source_revision,
            configuration,
            stages,
            hard_lines: self.hard_line_snapshot(),
            markdown_projection: (self.format() == Format::Markdown)
                .then(|| self.projection().clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn whole_document(document: &Document) -> TextRange {
        TextRange::new(
            document.text_point(0).unwrap(),
            document.text_point(document.text().len()).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn fused_pipeline_exposes_deterministic_stage_contracts() {
        let document = Document::new("alpha\nbeta");
        let first = document.transformation_pipeline_snapshot();
        let second = document.transformation_pipeline_snapshot();
        assert_eq!(first.document(), document.id());
        assert_eq!(first.revision(), document.revision());
        assert_eq!(first.configuration(), second.configuration());
        assert_eq!(first.stages(), second.stages());
        assert_eq!(first.stages().len(), 4);
        assert!(first.stages().iter().all(|stage| {
            stage.directionality == TransformationDirectionality::EditableProjection
                && stage.fused_execution_group == 1
        }));
        assert_eq!(
            first
                .stages()
                .iter()
                .map(|stage| stage.role)
                .collect::<Vec<_>>(),
            vec![
                TransformationStageRole::EncodingProjection,
                TransformationStageRole::LineEndingInterpretation,
                TransformationStageRole::LosslessFormatProjection,
                TransformationStageRole::FormattedDocumentAssembly,
            ]
        );
    }

    #[test]
    fn stage_identities_include_only_relevant_configuration() {
        let utf8 = Document::from_bytes_with_file_format(
            b"same".to_vec(),
            Encoding::Utf8,
            Format::PlainText,
            FileFormat::Unix,
        )
        .unwrap()
        .transformation_pipeline_snapshot();
        let latin1 = Document::from_bytes_with_file_format(
            b"same".to_vec(),
            Encoding::Latin1,
            Format::PlainText,
            FileFormat::Unix,
        )
        .unwrap()
        .transformation_pipeline_snapshot();
        assert_ne!(utf8.stages()[0], latin1.stages()[0]);
        assert_eq!(&utf8.stages()[1..], &latin1.stages()[1..]);

        let dos = Document::from_bytes_with_file_format(
            b"same".to_vec(),
            Encoding::Utf8,
            Format::PlainText,
            FileFormat::Dos,
        )
        .unwrap()
        .transformation_pipeline_snapshot();
        assert_eq!(utf8.stages()[0], dos.stages()[0]);
        assert_ne!(utf8.stages()[1], dos.stages()[1]);
        assert_eq!(&utf8.stages()[2..], &dos.stages()[2..]);
    }

    #[test]
    fn capability_queries_do_not_materialize_unrelated_formatted_text() {
        let mut document = Document::new("line\n".repeat(20_000));
        document.insert(2, "X").unwrap();
        assert!(!document.projection().compatibility_text_is_materialized());
        let range = TextRange::new(
            document.text_point(0).unwrap(),
            document.text_point(5).unwrap(),
        )
        .unwrap();
        let pipeline = document.transformation_pipeline_snapshot();
        assert_eq!(
            pipeline
                .capabilities(
                    range,
                    &PipelineEditIntent::ReplaceText {
                        replacement: "other".to_owned(),
                    },
                )
                .unwrap()
                .decision,
            PipelineCapabilityDecision::Supported
        );
        assert!(!document.projection().compatibility_text_is_materialized());
    }

    #[test]
    fn capabilities_compose_format_and_encoding_policy() {
        let plain = Document::new("plain");
        let plain_pipeline = plain.transformation_pipeline_snapshot();
        let plain_range = whole_document(&plain);
        let bold = plain_pipeline
            .capabilities(
                plain_range,
                &PipelineEditIntent::SetSemanticInlineStyle {
                    style: SemanticInlineStyle::Strong,
                    enabled: true,
                },
            )
            .unwrap();
        assert!(matches!(
            bold.decision,
            PipelineCapabilityDecision::Unsupported {
                reason: UnsupportedEditReason::PlainTextHasNoRichStyleStorage,
                ..
            }
        ));

        let markdown =
            Document::from_bytes(b"**bold**".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let markdown_pipeline = markdown.transformation_pipeline_snapshot();
        assert_eq!(
            markdown_pipeline
                .capabilities(
                    whole_document(&markdown),
                    &PipelineEditIntent::SetSemanticInlineStyle {
                        style: SemanticInlineStyle::Strong,
                        enabled: false,
                    },
                )
                .unwrap()
                .decision,
            PipelineCapabilityDecision::Supported
        );

        let latin1 =
            Document::from_bytes(b"plain".to_vec(), Encoding::Latin1, Format::PlainText).unwrap();
        let report = latin1
            .transformation_pipeline_snapshot()
            .capabilities(
                whole_document(&latin1),
                &PipelineEditIntent::ReplaceText {
                    replacement: "lambda: λ".to_owned(),
                },
            )
            .unwrap();
        assert!(matches!(
            report.decision,
            PipelineCapabilityDecision::NeedsPolicy {
                request: PipelinePolicyRequest::UnrepresentableCharacter {
                    encoding: Encoding::Latin1,
                    character: 'λ',
                },
                ..
            }
        ));
    }

    #[test]
    fn latin1_markdown_capabilities_distinguish_escaped_prose_and_literal_code() {
        for (format, source, selected, replacement, supported) in [
            (Format::Markdown, "**prose**", 0..5, "中", true),
            (
                Format::Markdown,
                "[link](https://example.test/)",
                0..4,
                "中",
                true,
            ),
            (Format::Markdown, "", 0..0, "中", true),
            (Format::Markdown, "`code`", 0..4, "中", false),
            (Format::Markdown, "```\ncode\n```", 0..4, "中", false),
            (Format::Markdown, "a `b`", 0..3, "中", true),
            (Format::Markdown, "a `b`", 0..3, "AA中", false),
            (Format::MarkdownSource, "**prose**", 2..7, "中", false),
            (Format::PlainText, "prose", 0..5, "中", false),
        ] {
            let mut document =
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Latin1, format).unwrap();
            let range = TextRange::new(
                document.text_point(selected.start).unwrap(),
                document.text_point(selected.end).unwrap(),
            )
            .unwrap();
            let snapshot = document.transformation_pipeline_snapshot();
            let report = snapshot
                .capabilities(
                    range,
                    &PipelineEditIntent::ReplaceText {
                        replacement: replacement.to_owned(),
                    },
                )
                .unwrap();
            assert_eq!(
                report.decision == PipelineCapabilityDecision::Supported,
                supported,
                "{format:?} {source} {replacement}: {:?}",
                report.decision
            );
            if !supported {
                assert!(matches!(
                    report.decision,
                    PipelineCapabilityDecision::NeedsPolicy {
                        request: PipelinePolicyRequest::UnrepresentableCharacter {
                            encoding: Encoding::Latin1,
                            character: '中'
                        },
                        ..
                    }
                ));
            }
            assert_eq!(
                document.replace(selected, replacement).is_ok(),
                supported,
                "{format:?} {source} {replacement}"
            );
            // The original context stays valid after a supported edit.
            assert_eq!(
                snapshot
                    .capabilities(
                        range,
                        &PipelineEditIntent::ReplaceText {
                            replacement: replacement.to_owned(),
                        }
                    )
                    .unwrap()
                    .decision,
                report.decision
            );
        }
    }

    #[test]
    fn capability_ranges_are_bound_to_the_captured_snapshot() {
        let first = Document::new("first");
        let second = Document::new("second");
        let pipeline = first.transformation_pipeline_snapshot();
        let error = pipeline
            .capabilities(
                whole_document(&second),
                &PipelineEditIntent::InsertHardBreak,
            )
            .unwrap_err();
        assert!(matches!(error, PositionError::WrongDocument { .. }));

        let mut evolving = Document::new("before");
        let old_range = whole_document(&evolving);
        let old_pipeline = evolving.transformation_pipeline_snapshot();
        evolving.insert(0, "after ").unwrap();
        let current_range = whole_document(&evolving);
        assert!(matches!(
            old_pipeline
                .capabilities(current_range, &PipelineEditIntent::InsertHardBreak)
                .unwrap_err(),
            PositionError::WrongSnapshot { .. }
        ));
        assert_eq!(
            old_pipeline
                .capabilities(old_range, &PipelineEditIntent::InsertHardBreak)
                .unwrap()
                .decision,
            PipelineCapabilityDecision::Supported
        );
    }

    #[test]
    fn invalidation_is_regional_for_plain_text_and_convergent_for_markdown() {
        let changed_range = 20..24;
        let change = PipelineInputChange {
            source_ranges: std::iter::once(changed_range.clone()).collect(),
            decoder_state_may_change: false,
            hard_lines_may_change: false,
            configuration_changed: false,
        };
        let plain = Document::new("plain").transformation_pipeline_snapshot();
        let plain_reports = plain.invalidate(&change);
        assert_eq!(plain_reports.len(), 4);
        assert!(plain_reports.iter().all(|report| matches!(
            report.scope,
            PipelineInvalidationScope::SourceRanges(ref ranges)
                if ranges.as_slice() == std::slice::from_ref(&changed_range)
        )));

        let markdown = Document::from_bytes(b"markdown".to_vec(), Encoding::Utf8, Format::Markdown)
            .unwrap()
            .transformation_pipeline_snapshot();
        let reports = markdown.invalidate(&change);
        let format = reports
            .iter()
            .find(|report| report.stage.name == "builtin.markdown")
            .unwrap();
        assert_eq!(
            format.scope,
            PipelineInvalidationScope::RestartUntilConvergence { source_offset: 20 }
        );

        let unchanged = plain.invalidate(&PipelineInputChange::source_ranges(Vec::new()));
        assert!(unchanged
            .iter()
            .all(|report| report.scope == PipelineInvalidationScope::None));
        let configuration = plain.invalidate(&PipelineInputChange {
            source_ranges: Vec::new(),
            decoder_state_may_change: false,
            hard_lines_may_change: false,
            configuration_changed: true,
        });
        assert!(configuration.iter().all(|report| {
            report.scope == PipelineInvalidationScope::FullProjection
                && report.reasons == [PipelineInvalidationReason::ConfigurationChanged]
        }));
    }

    #[test]
    fn upstream_stateful_changes_widen_every_dependent_stage() {
        let pipeline = Document::new("one\ntwo").transformation_pipeline_snapshot();
        let decoder_change = pipeline.invalidate(&PipelineInputChange {
            source_ranges: std::iter::once(4..5).collect(),
            decoder_state_may_change: true,
            hard_lines_may_change: false,
            configuration_changed: false,
        });
        assert!(decoder_change.iter().all(|report| {
            report.scope == PipelineInvalidationScope::RestartUntilConvergence { source_offset: 4 }
        }));
        assert!(decoder_change[0]
            .reasons
            .contains(&PipelineInvalidationReason::DecoderStateMayChange));
        assert!(decoder_change[1..].iter().all(|report| report
            .reasons
            .contains(&PipelineInvalidationReason::UpstreamProjectionChanged)));

        let hard_line_change = pipeline.invalidate(&PipelineInputChange {
            source_ranges: std::iter::once(3..4).collect(),
            decoder_state_may_change: false,
            hard_lines_may_change: true,
            configuration_changed: false,
        });
        assert!(matches!(
            hard_line_change[0].scope,
            PipelineInvalidationScope::SourceRanges(_)
        ));
        assert!(hard_line_change[1..].iter().all(|report| {
            report.scope == PipelineInvalidationScope::RestartUntilConvergence { source_offset: 3 }
        }));
        assert!(hard_line_change[1]
            .reasons
            .contains(&PipelineInvalidationReason::HardLineInterpretationMayChange));
    }
}

#[cfg(test)]
mod strikethrough_capability_tests {
    use super::*;

    #[test]
    fn strikethrough_is_advertised_only_for_markdown() {
        for format in [
            Format::Markdown,
            Format::MarkdownSource,
            Format::PlainText,
            Format::Code,
        ] {
            let document = Document::from_bytes(b"word".to_vec(), Encoding::Utf8, format).unwrap();
            let pipeline = document.transformation_pipeline_snapshot();
            let range = TextRange::new(
                document.text_point(0).unwrap(),
                document.text_point(4).unwrap(),
            )
            .unwrap();
            for enabled in [true, false] {
                let intent = PipelineEditIntent::SetStrikethrough { enabled };
                assert_eq!(
                    matches!(
                        pipeline.capabilities(range, &intent).unwrap().decision,
                        PipelineCapabilityDecision::Supported
                    ),
                    format.is_markdown(),
                    "{format:?}: {intent:?}"
                );
            }
        }
    }
}
