//! Source-only history and the projection lineages derived from it.
//!
//! The undo tree is over the persisted artifact alone. Each history node may
//! also carry the exact projection a lineage had there, as a disposable cache:
//! a node reached without one is followed from the current projection by
//! reprojecting its source. Attaching or dropping a cached projection replaces
//! the node's small record through history's journal, so command rollback and
//! the memory ledgers stay exact.
use super::history_memory::MemoryVisitor;
use super::history_source::SourceRetentionVisitor;
use super::*;
use std::sync::Arc;

/// The persisted artifact at one history node.
#[derive(Clone)]
pub(super) struct SourceState {
    pub(super) source: SourceSnapshot,
    pub(super) encoding: Encoding,
    pub(super) file_format: FileFormat,
    pub(super) file_format_origin: FileFormatOrigin,
    pub(super) line_ending_evidence: LineEndingEvidence,
    pub(super) has_bom: bool,
}

impl SourceState {
    pub(super) fn of(state: &DocumentState) -> Self {
        Self {
            source: state.source.clone(),
            encoding: state.encoding,
            file_format: state.file_format,
            file_format_origin: state.file_format_origin,
            line_ending_evidence: state.line_ending_evidence,
            has_bom: state.has_bom,
        }
    }

    /// Whether `state` is a projection of exactly this artifact and its
    /// interpretation.
    pub(super) fn describes(&self, state: &DocumentState) -> bool {
        self.source.identity() == state.source.identity()
            && self.encoding == state.encoding
            && self.file_format == state.file_format
            && self.file_format_origin == state.file_format_origin
            && self.line_ending_evidence == state.line_ending_evidence
            && self.has_bom == state.has_bom
    }

    /// Interpretation metadata other than the bytes themselves.
    pub(super) fn same_interpretation(&self, state: &DocumentState) -> bool {
        self.encoding == state.encoding
            && self.file_format == state.file_format
            && self.has_bom == state.has_bom
    }
}

/// One history node: its source, and the projections recorded there while a
/// lineage of that kind was live.
#[derive(Clone)]
pub(super) struct HistoryState {
    pub(super) source: SourceState,
    literal: Option<Arc<DocumentState>>,
    preview: Option<Arc<DocumentState>>,
}

impl HistoryState {
    pub(super) fn recording(state: &Arc<DocumentState>) -> Self {
        let mut record = Self { source: SourceState::of(state), literal: None, preview: None };
        *record.slot_mut(state.kind) = Some(state.clone());
        record
    }

    fn slot_mut(&mut self, kind: ProjectionKind) -> &mut Option<Arc<DocumentState>> {
        match kind {
            ProjectionKind::Literal => &mut self.literal,
            ProjectionKind::Preview => &mut self.preview,
        }
    }

    /// A record of every live lineage's projection of one source.
    pub(super) fn recording_all<'a>(states: impl IntoIterator<Item = &'a Arc<DocumentState>>) -> Self {
        let mut states = states.into_iter();
        let first = states.next().expect("at least one lineage is live");
        let mut record = Self::recording(first);
        for state in states { *record.slot_mut(state.kind) = Some(state.clone()); }
        record
    }

    /// The exact projection recorded here for `kind`, if still cached.
    pub(super) fn projection(&self, kind: ProjectionKind) -> Option<&Arc<DocumentState>> {
        match kind {
            ProjectionKind::Literal => self.literal.as_ref(),
            ProjectionKind::Preview => self.preview.as_ref(),
        }
        .filter(|state| self.source.describes(state))
    }

    /// This record with `state` cached for its kind.
    pub(super) fn with_projection(&self, state: &Arc<DocumentState>) -> Self {
        let mut record = self.clone();
        *record.slot_mut(state.kind) = Some(state.clone());
        record
    }

    /// This record without a cached projection for `kind`.
    pub(super) fn without_projection(&self, kind: ProjectionKind) -> Option<Self> {
        self.projection(kind)?;
        let mut record = self.clone();
        *record.slot_mut(kind) = None;
        Some(record)
    }

    fn projections(&self) -> impl Iterator<Item = &Arc<DocumentState>> {
        self.literal.iter().chain(self.preview.iter())
    }
}

pub(super) fn visit_history_state_source_buffers(state: &HistoryState, visitor: &mut SourceRetentionVisitor<'_>) {
    state.source.source.visit_source_retention(visitor);
}

pub(super) fn history_state_source_digest(state: &HistoryState) -> SourceArtifactDigest {
    state.source.source.artifact_digest()
}

pub(super) fn visit_history_state_memory(state: &HistoryState, visitor: &mut MemoryVisitor<'_>) {
    state.source.source.visit_retained_memory(visitor);
    for projection in state.projections() {
        visitor.arc(projection, |_| {});
        super::visit_document_state_memory(projection, visitor);
    }
}

/// Each lineage's exact formatted transition across one history edge,
/// recorded only for lineages that were live when the edge was committed.
#[derive(Clone)]
pub(super) struct LineageMaps {
    literal: Option<PositionMap>,
    preview: Option<PositionMap>,
}

impl LineageMaps {
    pub(super) fn single(kind: ProjectionKind, map: PositionMap) -> Self {
        match kind {
            ProjectionKind::Literal => Self { literal: Some(map), preview: None },
            ProjectionKind::Preview => Self { literal: None, preview: Some(map) },
        }
    }

    pub(super) fn with(mut self, kind: ProjectionKind, map: PositionMap) -> Self {
        match kind {
            ProjectionKind::Literal => self.literal = Some(map),
            ProjectionKind::Preview => self.preview = Some(map),
        }
        self
    }

    pub(super) fn into_kind(self, kind: ProjectionKind) -> Option<PositionMap> {
        match kind {
            ProjectionKind::Literal => self.literal,
            ProjectionKind::Preview => self.preview,
        }
    }

    pub(super) fn visit_retained_memory(maps: &Self, visitor: &mut MemoryVisitor<'_>) {
        for map in maps.literal.iter().chain(maps.preview.iter()) {
            PositionMap::visit_retained_memory(map, visitor);
        }
    }

    fn zip(&self, other: &Self, combine: impl Fn(&PositionMap, &PositionMap) -> Result<PositionMap, PositionError>)
        -> Result<Self, PositionError> {
        let pair = |left: &Option<PositionMap>, right: &Option<PositionMap>| match (left, right) {
            (Some(left), Some(right)) => combine(left, right).map(Some),
            _ => Ok(None),
        };
        Ok(Self { literal: pair(&self.literal, &other.literal)?, preview: pair(&self.preview, &other.preview)? })
    }
}

impl super::history::HistoryEdgeMap for LineageMaps {
    fn then(&self, next: &Self) -> Result<Self, PositionError> {
        self.zip(next, |left, right| left.then(right))
    }

    fn inverted(&self) -> Result<Self, PositionError> {
        Ok(Self {
            literal: self.literal.as_ref().map(PositionMap::inverted).transpose()?,
            preview: self.preview.as_ref().map(PositionMap::inverted).transpose()?,
        })
    }

    fn check_then(&self, next: &Self) -> Result<(), PositionError> {
        self.zip(next, |left, right| left.check_then(right).map(|()| left.clone())).map(|_| ())
    }
}

/// One live projection lineage: its state at the current history node and
/// the disposable caches derived from it.
#[derive(Clone)]
pub(crate) struct LiveLineage {
    pub(super) state: Arc<DocumentState>,
    /// The state with the application theme overlaid, never history.
    pub(super) configuration_state: Option<DocumentState>,
    pub(super) code_presentation: Option<super::code_presentation::CodePresentation>,
    /// The Literal structure with the Code sheet and no spans.
    pub(super) code_base: Option<FormattedDocument>,
    /// Exact formatted change of recent committed transactions, so a view's
    /// layout cache can rebase across a revision instead of discarding.
    pub(super) layout_changes: std::collections::VecDeque<LayoutChangeRecord>,
    /// This lineage's part of the coordinator's per-event capture.
    pub(super) position_map_capture: Option<PositionMap>,
    /// The map the last completed capture produced for this lineage.
    pub(super) last_capture: Option<PositionMap>,
    /// The map of the last publication that moved this lineage as a follower.
    pub(super) last_follow: Option<PositionMap>,
}

impl LiveLineage {
    pub(super) fn new(state: Arc<DocumentState>) -> Self {
        Self {
            state,
            configuration_state: None,
            code_presentation: None,
            code_base: None,
            layout_changes: Default::default(),
            position_map_capture: None,
            last_capture: None,
            last_follow: None,
        }
    }
}

/// At most one live lineage per projection kind; at least one is live.
#[derive(Clone)]
pub(crate) struct Lineages {
    literal: Option<LiveLineage>,
    preview: Option<LiveLineage>,
}

impl Lineages {
    pub(super) fn single(lineage: LiveLineage) -> Self {
        let mut lineages = Self { literal: None, preview: None };
        lineages.insert(lineage);
        lineages
    }

    fn slot(&self, kind: ProjectionKind) -> &Option<LiveLineage> {
        match kind { ProjectionKind::Literal => &self.literal, ProjectionKind::Preview => &self.preview }
    }

    fn slot_mut(&mut self, kind: ProjectionKind) -> &mut Option<LiveLineage> {
        match kind { ProjectionKind::Literal => &mut self.literal, ProjectionKind::Preview => &mut self.preview }
    }

    pub(super) fn get(&self, kind: ProjectionKind) -> Option<&LiveLineage> { self.slot(kind).as_ref() }

    pub(super) fn get_mut(&mut self, kind: ProjectionKind) -> Option<&mut LiveLineage> { self.slot_mut(kind).as_mut() }

    pub(super) fn has(&self, kind: ProjectionKind) -> bool { self.slot(kind).is_some() }

    pub(super) fn insert(&mut self, lineage: LiveLineage) {
        let kind = lineage.state.kind;
        *self.slot_mut(kind) = Some(lineage);
    }

    pub(super) fn remove(&mut self, kind: ProjectionKind) -> Option<LiveLineage> {
        self.slot_mut(kind).take()
    }

    pub(super) fn kinds(&self) -> impl Iterator<Item = ProjectionKind> + '_ {
        [ProjectionKind::Literal, ProjectionKind::Preview].into_iter().filter(|kind| self.has(*kind))
    }
}

/// A history node's projection for the live lineage and the exact map from
/// the current projection to it.
pub(super) struct ResolvedHistoryNode {
    pub(super) state: Arc<DocumentState>,
    pub(super) map: PositionMap,
    pub(super) next_projected_block_id: u64,
    /// Not cached: reprojected from the node's source.
    pub(super) followed: bool,
    /// The exact bounded formatted change, when the follow was regional.
    pub(super) local: Option<LocalFormattedChange>,
}

/// `VIEM_SHADOW_LINEAGE=1` keeps the other projection kind live for every
/// document and checks each follower against a fresh projection of its
/// source, so the ordinary suites exercise the follower path.
pub(crate) fn shadow_lineage_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("VIEM_SHADOW_LINEAGE").is_some_and(|value| value == "1"))
}

fn verify_follower(state: &DocumentState) {
    let Ok(decoded) = state.encoding.decode(&state.source.bytes()) else { return };
    let Ok(fresh) = build_state_from_decoded(state.source.clone(), decoded, state.structure_format(),
        state.file_format, state.file_format_origin, state.line_ending_evidence, state.revision) else { return };
    if let Some(difference) = state.projection.semantic_difference(&fresh.projection) {
        panic!("shadow lineage mismatch in the {:?} follower: {difference}", state.kind);
    }
}

/// Whether a follower's candidate source is exactly `target`. Both were
/// produced from the current source by the same changed extents, so only
/// those ranges (and the start, where BOM protection may add a patch) can
/// differ. A full digest would cost the whole source on every keystroke.
fn reaches_source(candidate: &SourceSnapshot, target: &SourceSnapshot, extents: &[(Range<usize>, Range<usize>)]) -> bool {
    if candidate.len() != target.len() { return false; }
    let head = 0..target.len().min(4);
    let reached = std::iter::once(head).chain(extents.iter().map(|(_, new)| new.clone()))
        .all(|range| candidate.bytes_in(range.clone()) == target.bytes_in(range));
    if reached && transaction::candidate::shadow_enabled(target.len()) {
        assert!(candidate.artifact_digest() == target.artifact_digest(),
            "a follower changed source outside its patches");
    }
    reached
}

/// Where followers move to: an edit's new source, or a history node.
pub(super) enum FollowTarget<'a> {
    Source(&'a SourceState),
    Node(HistoryNodeId),
}

/// A lineage other than the acting one, prepared to catch up with a
/// transaction before it publishes (synchronous followers, D6).
pub(super) struct Follower {
    pub(super) state: Arc<DocumentState>,
    pub(super) map: PositionMap,
    pub(super) next_projected_block_id: u64,
    /// The lineage's per-event capture composed with `map`.
    capture: Option<PositionMap>,
    followed: bool,
    local: Option<LocalFormattedChange>,
}

impl Document {
    /// Advance every live lineage other than the focused one to `target`,
    /// allocating revisions from `revision` and block identities from
    /// `next_block`. Nothing is published. A follower never rejects the
    /// edit: its last resort is complete reprojection of the new source.
    ///
    /// Typing with a Code view and a Preview view of one large Markdown file
    /// open pays one bounded Preview follow per keystroke, or a complete one
    /// when the regional strategies decline. If that proves slow, the known
    /// improvement is a lagging follower that a worker reprojects from the
    /// immutable source snapshot; see docs/performance.md (open work).
    pub(super) fn prepare_followers(&self, target: FollowTarget<'_>, mut revision: u64, mut next_block: u64)
        -> Result<Vec<Follower>, ModelTransactionError> {
        let acting = self.focus_kind();
        let kinds = self.lineages.kinds().filter(|kind| *kind != acting).collect::<Vec<_>>();
        let mut followers = Vec::with_capacity(kinds.len());
        for kind in kinds {
            let resolved = self.with_kind_focus(kind, |document| match &target {
                FollowTarget::Source(source) if source.describes(document.state()) => Ok(ResolvedHistoryNode {
                    state: document.live().state.clone(),
                    map: document.identity_map(),
                    next_projected_block_id: next_block,
                    followed: false,
                    local: None,
                }),
                FollowTarget::Source(source) => document.follow_to_source(source, Revision(revision), next_block),
                FollowTarget::Node(node) => document.resolve_history_node_at(*node, Revision(revision), next_block),
            })?;
            let capture = self.lineages.get(kind).and_then(|lineage| lineage.position_map_capture.as_ref())
                .map(|capture| capture.then(&resolved.map)).transpose()?;
            if resolved.state.revision.0 >= revision { revision = resolved.state.revision.0 + 1; }
            next_block = next_block.max(resolved.next_projected_block_id);
            if resolved.followed {
                super::work_statistics::record(|stats| stats.follower_reprojections += 1);
            }
            followers.push(Follower {
                state: resolved.state,
                map: resolved.map,
                next_projected_block_id: resolved.next_projected_block_id,
                capture,
                followed: resolved.followed,
                local: resolved.local,
            });
        }
        Ok(followers)
    }

    /// Install prepared followers. Infallible; called while publishing.
    pub(super) fn publish_followers(&mut self, followers: Vec<Follower>, cache_on_node: bool) {
        if shadow_lineage_enabled() {
            for follower in &followers { verify_follower(&follower.state); }
        }
        for follower in followers {
            let kind = follower.state.kind;
            if follower.state.revision.0 >= self.next_revision { self.next_revision = follower.state.revision.0 + 1; }
            self.next_projected_block_id = self.next_projected_block_id.max(follower.next_projected_block_id);
            if cache_on_node { self.install_live_state(follower.state.clone()); }
            self.with_kind_focus_mut(kind, |document| {
                let before = document.live().state.revision;
                let lineage = document.live_mut();
                lineage.state = follower.state.clone();
                lineage.configuration_state = None;
                lineage.position_map_capture = follower.capture;
                lineage.last_follow = Some(follower.map.clone());
                document.advance_code_presentation(&follower.map);
                if before != follower.state.revision {
                    document.record_layout_change(LayoutChangeRecord {
                        before, after: follower.state.revision, local: follower.local.clone(),
                    });
                }
            });
            let _ = follower.followed;
        }
    }

    /// The live lineage's projection at `target`: the cached record when
    /// present, otherwise the current projection followed to the node's
    /// source. Never interprets a projection recorded for another source.
    pub(super) fn resolve_history_node(&self, target: HistoryNodeId) -> Result<ResolvedHistoryNode, ModelTransactionError> {
        self.resolve_history_node_at(target, Revision(self.next_revision), self.next_projected_block_id)
    }

    /// [`Self::resolve_history_node`] allocating a followed projection's
    /// revision and block identities from the given starting values.
    pub(super) fn resolve_history_node_at(&self, target: HistoryNodeId, revision: Revision, next_block: u64)
        -> Result<ResolvedHistoryNode, ModelTransactionError> {
        let record = self.history.state_at_node(target)
            .ok_or(HistoryError::NodeNotFound(target))?;
        let kind = self.state().kind;
        let current = self.history.status().current.node;
        if target == current {
            return Ok(ResolvedHistoryNode {
                state: self.live().state.clone(),
                map: self.identity_map(),
                next_projected_block_id: next_block,
                followed: false,
                local: None,
            });
        }
        if let Some(cached) = record.projection(kind) {
            // Recorded edge maps compose only between the projections they
            // were recorded for. After the lineage was replaced (a switch away
            // and back) the live projection is another snapshot, so compare
            // the two persistent snapshots instead.
            let composed = self.history
                .map_between(current, target, LineageMaps::single(kind, self.identity_map()))
                .ok()
                .and_then(|maps| maps.into_kind(kind))
                .filter(|map| map.target_revision() == cached.revision);
            let map = match composed {
                Some(map) => map,
                None => self.snapshot_difference_map(cached)?,
            };
            return Ok(ResolvedHistoryNode {
                state: cached.clone(),
                map,
                next_projected_block_id: next_block,
                followed: false,
                local: None,
            });
        }
        self.follow_to_source(&record.source, revision, next_block)
    }

    fn identity_map(&self) -> PositionMap {
        PositionMap::identity(self.id, PositionDomain::FormattedText, self.revision(),
            self.projection().text_tree().byte_len())
    }

    /// An exact map from the current projection to `target`, a persistent
    /// snapshot sharing unchanged structure: only the changed boundary path
    /// is compared.
    fn snapshot_difference_map(&self, target: &DocumentState) -> Result<PositionMap, ModelTransactionError> {
        let current = self.projection();
        let edits = match current.text_tree().changed_extent(target.projection.text_tree()) {
            None => Vec::new(),
            Some((old, new)) => vec![TextEdit {
                range: old,
                replacement: target.projection.text_tree().slice(new).map_err(DocumentError::FormattedTextStorage)?,
            }],
        };
        let splices = transaction::grapheme_closed_snapshot_map_splices(current, &target.projection, &edits)?;
        Ok(PositionMap::for_text_snapshots(self.id, self.revision(), target.revision, current,
            &target.projection, splices)?)
    }

    /// Follow the live projection to another source state: reproject only
    /// what the source difference needs, falling back to complete
    /// reprojection, which cannot fail for a source history accepted.
    pub(super) fn follow_to_source(&self, target: &SourceState, revision: Revision, next_block: u64)
        -> Result<ResolvedHistoryNode, ModelTransactionError> {
        if target.same_interpretation(self.state()) && self.state().kind == ProjectionKind::Literal {
            if let Some(resolved) = self.follow_literal_patches(target, revision, next_block) {
                return Ok(resolved);
            }
        }
        if target.same_interpretation(self.state()) {
            let extents = self.state().source.changed_extents(&target.source);
            let patches = extents.iter().cloned()
                .map(|(old, new)| SourcePatch::primary(old, target.source.bytes_in(new).expect("validated source difference")))
                .collect::<Vec<_>>();
            let mut scratch = self.scratch_document();
            scratch.next_revision = revision.0;
            scratch.next_projected_block_id = next_block;
            let prepared = transaction::candidate::unobserved(|| scratch.prepare_reprojected_source_patches(patches));
            if let Ok(prepared) = prepared {
                let local = self.follow_layout_change(&prepared);
                if let transaction::PreparedPublication::State(mut candidate) = prepared.publication {
                    if reaches_source(&candidate.source, &target.source, &extents) {
                        candidate.source = target.source.clone();
                        candidate.file_format_origin = target.file_format_origin;
                        candidate.line_ending_evidence = target.line_ending_evidence;
                        return Ok(ResolvedHistoryNode {
                            state: Arc::new(candidate),
                            map: prepared.text_position_map,
                            next_projected_block_id: prepared.next_projected_block_id_after,
                            followed: true,
                            local,
                        });
                    }
                }
            }
        }
        self.reproject_source_state(target, revision, next_block)
    }

    /// Literal text is a divisible function of decoded source: a UTF-8 patch
    /// whose ends map exactly and whose replacement holds no line-ending CR
    /// is the same literal text edit. Run it through the ordinary regional
    /// literal strategies, accepting only a result whose source is exactly
    /// `target`. `None` declines to the general follow.
    fn follow_literal_patches(&self, target: &SourceState, revision: Revision, next_block: u64) -> Option<ResolvedHistoryNode> {
        if self.state().encoding != Encoding::Utf8 { return None; }
        let projection = &self.state().projection;
        let mut patches = Vec::new();
        let mut edits = Vec::new();
        let extents = self.state().source.changed_extents(&target.source);
        for (old, new) in extents.iter().cloned() {
            let bytes = target.source.bytes_in(new)?;
            let replacement = String::from_utf8(bytes.clone()).ok()?;
            if replacement.contains('\r') { return None; }
            // The source and literal lengths of the patched range must agree
            // byte for byte (checked below), and the result's source is
            // compared with `target` exactly.
            let boundary = |offset, affinity| projection.map_source_boundary(projection.revision(), offset, affinity).ok()
                .map(|boundary| boundary.formatted_offset);
            let start = boundary(old.start, BoundaryAffinity::Downstream)?;
            let end = boundary(old.end, BoundaryAffinity::Upstream)?;
            if end < start || end - start != old.len() { return None; }
            edits.push(TextEdit { range: start..end, replacement });
            patches.push(SourcePatch::primary(old, bytes));
        }
        let mut scratch = self.scratch_document();
        scratch.next_revision = revision.0;
        scratch.next_projected_block_id = next_block;
        let prepared = transaction::candidate::unobserved(|| scratch.prepare_text_edits_for_follow(edits, patches)).ok()?;
        let local = self.follow_layout_change(&prepared);
        let transaction::PreparedPublication::State(mut candidate) = prepared.publication else { return None };
        if !reaches_source(&candidate.source, &target.source, &extents) { return None; }
        candidate.source = target.source.clone();
        candidate.file_format_origin = target.file_format_origin;
        candidate.line_ending_evidence = target.line_ending_evidence;
        Some(ResolvedHistoryNode {
            state: Arc::new(candidate),
            map: prepared.text_position_map,
            next_projected_block_id: prepared.next_projected_block_id_after,
            followed: true,
            local,
        })
    }

    /// The exact bounded layout change of a regional follow, so the follower
    /// views' caches rebase instead of being discarded.
    fn follow_layout_change(&self, prepared: &transaction::PreparedModelTransaction) -> Option<LocalFormattedChange> {
        let transaction::PreparedPublication::State(candidate) = &prepared.publication else { return None };
        transaction::local_layout_change(&prepared.summary, prepared.layout_invalidation.as_ref(),
            self.projection().presentation_line_count(), candidate.projection.presentation_line_count())
    }

    /// Complete reprojection of `target` in the live lineage's kind, with
    /// block identities reconciled against the current projection.
    fn reproject_source_state(&self, target: &SourceState, revision: Revision, next_block: u64)
        -> Result<ResolvedHistoryNode, ModelTransactionError> {
        let decoded = target.encoding.decode(&target.source.bytes())?;
        let mut candidate = build_state_from_decoded(target.source.clone(), decoded,
            self.state().structure_format(), target.file_format, target.file_format_origin,
            target.line_ending_evidence, revision)?;
        let current = self.projection();
        let edits = match current.text_tree().changed_extent(candidate.projection.text_tree()) {
            None => Vec::new(),
            Some((old, new)) => vec![TextEdit {
                range: old,
                replacement: candidate.projection.text_tree().slice(new).map_err(DocumentError::FormattedTextStorage)?,
            }],
        };
        candidate.projection.install_persistent_text_edits(current, &edits)
            .map_err(DocumentError::FormattedTextStorage)?;
        let splices = transaction::grapheme_closed_snapshot_map_splices(current, &candidate.projection, &edits)?;
        let map = PositionMap::for_text_snapshots(self.id, self.revision(), revision, current,
            &candidate.projection, splices)?;
        let next_projected_block_id = candidate.projection
            .install_reconciled_source_block_ids(current, &edits, &map, next_block)
            .map_err(block_identity_document_error)?;
        Ok(ResolvedHistoryNode { state: Arc::new(candidate), map, next_projected_block_id, followed: true, local: None })
    }

    /// Translate a formatted offset between two live projections of the
    /// current source through its source boundary. Hidden syntax has no caret
    /// stop, so such an offset lands on the nearest visible boundary.
    pub(crate) fn translate_offset(&self, from: ProjectionKind, to: ProjectionKind, offset: usize) -> Option<usize> {
        if from == to { return Some(offset); }
        if !self.lineages.has(from) || !self.lineages.has(to) { return None; }
        let source = self.with_kind_focus(from, |document| document.projection().source_boundary_downstream(offset))?;
        self.with_kind_focus(to, |document| {
            let projection = document.projection();
            projection.map_source_boundary(projection.revision(), source, BoundaryAffinity::Downstream)
                .ok().map(|boundary| boundary.formatted_offset)
        })
    }

    /// Install `state` as the live lineage at the current history node and
    /// cache it on that node.
    pub(super) fn install_live_state(&mut self, state: Arc<DocumentState>) {
        let node = self.history.status().current.node;
        if let Some(record) = self.history.state_at_node(node) {
            if record.projection(state.kind).is_none_or(|cached| !Arc::ptr_eq(cached, &state)) {
                let record = record.with_projection(&state);
                self.history.replace_node_state(node, record);
            }
        }
        match self.lineages.get_mut(state.kind) {
            Some(lineage) => {
                lineage.state = state;
                lineage.configuration_state = None;
            }
            None => self.lineages.insert(LiveLineage::new(state)),
        }
    }

    /// Stop maintaining the lineage of `kind` and drop the projections it
    /// cached in history. The last live lineage is never released.
    pub(crate) fn release_lineage(&mut self, kind: ProjectionKind) -> bool {
        if !self.lineages.has(kind) || self.lineages.kinds().count() < 2 { return false; }
        self.lineages.remove(kind);
        self.release_lineage_cache(kind);
        true
    }

    /// Replace the history with a transient one holding only `state`, for
    /// scratch planning documents.
    pub(super) fn install_transient_state(&mut self, state: DocumentState) {
        let state = Arc::new(state);
        self.history = super::history::History::transient(HistoryState::recording(&state));
        let code_base = self.lineages.get(state.kind).and_then(|lineage| lineage.code_base.clone());
        let mut lineage = LiveLineage::new(state);
        lineage.code_base = code_base;
        self.lineages = Lineages::single(lineage);
    }

    /// How many history nodes cache a projection of `kind`.
    #[cfg(test)]
    pub(crate) fn cached_projection_count(&self, kind: ProjectionKind) -> usize {
        self.history.node_ids().into_iter()
            .filter(|node| self.history.state_at_node(*node).is_some_and(|record| record.projection(kind).is_some()))
            .count()
    }

    /// Drop every cached projection of `kind`, once no view shows it.
    pub(super) fn release_lineage_cache(&mut self, kind: ProjectionKind) {
        for node in self.history.node_ids() {
            let Some(record) = self.history.state_at_node(node).and_then(|record| record.without_projection(kind)) else { continue };
            self.history.replace_node_state(node, record);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn markdown(source: &str) -> Document {
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap()
    }

    fn cached_kinds(document: &Document) -> Vec<(bool, bool)> {
        document.history.node_ids().into_iter().map(|node| {
            let record = document.history.state_at_node(node).unwrap();
            (record.projection(ProjectionKind::Literal).is_some(), record.projection(ProjectionKind::Preview).is_some())
        }).collect()
    }

    #[test]
    fn switching_projection_records_no_history_node_or_dirty_state() {
        let mut document = markdown("# Title\n\n*body*\n");
        document.insert(0, "x").unwrap();
        document.mark_saved();
        let nodes = document.history_status().node_count;
        document.set_view_format(Format::Code).unwrap();
        assert_eq!(document.history_status().node_count, nodes);
        assert!(!document.is_dirty());
        assert_eq!(document.projection_kind(), ProjectionKind::Literal);
    }

    #[test]
    fn a_switch_releases_the_previous_lineage_from_history() {
        let mut document = markdown("# Title\n\nbody\n");
        document.insert(9, "more ").unwrap();
        assert!(cached_kinds(&document).iter().all(|&(literal, preview)| !literal && preview));
        document.set_view_format(Format::Code).unwrap();
        let kinds = cached_kinds(&document);
        assert!(kinds.iter().all(|&(_, preview)| !preview), "{kinds:?}");
        assert_eq!(kinds.iter().filter(|&&(literal, _)| literal).count(), 1, "only the current node is cached");
    }

    #[test]
    fn undo_to_an_uncached_node_follows_its_source_exactly() {
        let source = "# Title\n\nfirst paragraph *text*\n\nsecond\n";
        let mut document = markdown(source);
        let at = document.text().find("second").unwrap();
        document.insert(at, "edited ").unwrap();
        document.set_view_format(Format::Code).unwrap();
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert_eq!(document.projection_kind(), ProjectionKind::Literal);
        let fresh = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::PlainText).unwrap();
        assert_eq!(document.text(), fresh.text());
        assert!(document.projection().semantic_difference(fresh.projection()).is_none());
        assert!(document.redo());
        assert!(document.text().contains("edited second"));
    }

    #[test]
    fn a_followed_node_is_cached_for_the_next_visit() {
        let mut document = markdown("one\n\ntwo\n");
        document.insert(0, "x").unwrap();
        document.set_view_format(Format::Code).unwrap();
        assert!(document.undo());
        let root = document.history_status().current.node;
        assert!(document.history.state_at_node(root).unwrap().projection(ProjectionKind::Literal).is_some());
        let revision = document.revision();
        assert!(document.redo());
        assert!(document.undo());
        assert_eq!(document.revision(), revision, "the cached projection is restored, not followed again");
    }

    #[test]
    fn a_followed_node_takes_a_new_revision() {
        let mut document = markdown("one\n\ntwo\n");
        document.insert(0, "x").unwrap();
        document.set_view_format(Format::Code).unwrap();
        let switched = document.revision();
        assert!(document.undo());
        assert!(document.revision() > switched);
    }

    #[test]
    fn command_rollback_restores_the_live_lineage() {
        let mut document = markdown("one\n\ntwo\n");
        let kind = document.projection_kind();
        let revision = document.revision();
        let checkpoint = document.begin_command_checkpoint();
        document.set_view_format(Format::Code).unwrap();
        document.rollback_command_checkpoint(checkpoint);
        assert_eq!(document.projection_kind(), kind);
        assert_eq!(document.revision(), revision);
        assert!(cached_kinds(&document).iter().all(|&(literal, preview)| !literal && preview));
    }
}
