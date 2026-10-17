//! Background reprojection for switching Markdown Preview on or off.
//!
//! A switch reprojects the whole unchanged source in the other format and
//! diffs the two projections to map positions. Both steps depend only on the
//! current immutable state, so after editing pauses one worker prepares them
//! for the next revision. The switch then validates the exact state and
//! revision it was prepared from; any edit, configuration change or other
//! target discards it, and the switch falls back to preparing synchronously.

use super::{build_state_from_decoded, Document, DocumentState, Format, Revision, TextEdit};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// A preparation retains a second complete projection until the next edit or
/// switch: about 30 times the source for a Markdown Preview and 12 for Code.
/// Larger sources keep the synchronous switch rather than retain more.
pub(crate) const VIEW_FORMAT_PREFETCH_MAXIMUM_SOURCE_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PrefetchKey {
    /// The exact state, kept alive by the job while it runs.
    state: usize,
    revision: Revision,
    next_revision: u64,
    target: Format,
}

pub(super) struct PrefetchedViewFormat {
    pub(super) candidate: DocumentState,
    pub(super) edits: Vec<TextEdit>,
}

struct Job {
    key: PrefetchKey,
    cancelled: Arc<AtomicBool>,
    result: Arc<Mutex<Option<Option<PrefetchedViewFormat>>>>,
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

/// At most one job; replacing or clearing it cancels the previous one.
#[derive(Default)]
pub(crate) struct ViewFormatPrefetch {
    job: Mutex<Option<Job>>,
}

impl Document {
    fn view_format_prefetch_key(&self, target: Format) -> Option<PrefetchKey> {
        // An application theme overlays only styles on the history snapshot:
        // text and provenance, which the switch reads, are identical. A theme
        // change advances `next_revision`, so it still invalidates the key.
        if target == self.format() {
            return None;
        }
        Some(PrefetchKey {
            state: Arc::as_ptr(self.history.current()) as usize,
            revision: self.revision(),
            next_revision: self.next_revision,
            target,
        })
    }

    /// Start preparing a switch to `target` unless one for this exact state
    /// already exists. The worker owns an immutable snapshot only.
    pub(crate) fn prefetch_view_format(&self, target: Format) {
        let Some(key) = self.view_format_prefetch_key(target) else { return };
        if self.state().source.len() > VIEW_FORMAT_PREFETCH_MAXIMUM_SOURCE_BYTES {
            return;
        }
        let mut slot = self.view_format_prefetch.job.lock().unwrap_or_else(|e| e.into_inner());
        if slot.as_ref().is_some_and(|job| job.key == key) {
            return;
        }
        let state = Arc::clone(self.history.current());
        let cancelled = Arc::new(AtomicBool::new(false));
        let result = Arc::new(Mutex::new(None));
        let (worker_cancelled, output) = (cancelled.clone(), result.clone());
        let revision = Revision(key.next_revision);
        let spawned = std::thread::Builder::new()
            .name("viem-view-format-prefetch".into())
            .spawn(move || {
                let prepared = (|| {
                    let decoded = state.encoding.decode(&state.source.bytes()).ok()?;
                    if worker_cancelled.load(Ordering::Relaxed) { return None; }
                    let candidate = build_state_from_decoded(
                        state.source.clone(), decoded, target, state.file_format,
                        state.file_format_origin, state.line_ending_evidence, revision,
                    ).ok()?;
                    if worker_cancelled.load(Ordering::Relaxed) { return None; }
                    let edits = super::transaction::source_backed_reprojection_edits(
                        &state.projection, &candidate.projection);
                    Some(PrefetchedViewFormat { candidate, edits })
                })();
                *output.lock().unwrap_or_else(|e| e.into_inner()) = Some(prepared);
            });
        *slot = spawned.ok().map(|_| Job { key, cancelled, result });
    }

    /// Read a finished preparation for exactly this state without taking it,
    /// so work keyed to the revision the switch installs can start early.
    pub(crate) fn peek_prefetched_view_format<R>(
        &self, target: Format, read: impl FnOnce(Revision, &super::FormattedDocument) -> R,
    ) -> Option<R> {
        let key = self.view_format_prefetch_key(target)?;
        let slot = self.view_format_prefetch.job.lock().unwrap_or_else(|e| e.into_inner());
        let job = slot.as_ref().filter(|job| job.key == key)?;
        let result = job.result.lock().unwrap_or_else(|e| e.into_inner());
        let prepared = result.as_ref()?.as_ref()?;
        (prepared.candidate.revision == key.revision_after())
            .then(|| read(prepared.candidate.revision, &prepared.candidate.projection))
    }

    /// Discard any prepared or running switch.
    pub(crate) fn cancel_view_format_prefetch(&self) {
        self.view_format_prefetch.job.lock().unwrap_or_else(|e| e.into_inner()).take();
    }

    /// Whether a completed preparation matches the current state.
    pub fn view_format_prefetch_ready(&self, target: Format) -> bool {
        let Some(key) = self.view_format_prefetch_key(target) else { return false };
        let slot = self.view_format_prefetch.job.lock().unwrap_or_else(|e| e.into_inner());
        slot.as_ref().is_some_and(|job| job.key == key
            && job.result.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(Option::is_some))
    }

    /// The finished preparation for exactly this state and target, if any.
    /// A job still running is abandoned rather than awaited.
    pub(super) fn take_prefetched_view_format(&self, target: Format) -> Option<PrefetchedViewFormat> {
        let key = self.view_format_prefetch_key(target)?;
        let job = self.view_format_prefetch.job.lock().unwrap_or_else(|e| e.into_inner()).take()?;
        if job.key != key {
            return None;
        }
        let prepared = job.result.lock().unwrap_or_else(|e| e.into_inner()).take().flatten();
        prepared.filter(|prepared| prepared.candidate.revision == key.revision_after())
    }
}

impl PrefetchKey {
    fn revision_after(&self) -> Revision {
        Revision(self.next_revision)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{measure_document_work, Encoding};
    use std::time::{Duration, Instant};

    fn markdown() -> Document {
        let text = "# Title\n\nSome **bold** prose with a [link](x.md).\n\n- item\n- item\n\n".repeat(200);
        Document::from_bytes(text.into_bytes(), Encoding::Utf8, Format::Markdown).unwrap()
    }

    fn wait_until_ready(document: &Document, target: Format) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !document.view_format_prefetch_ready(target) {
            assert!(Instant::now() < deadline, "the preparation finishes");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn a_prepared_switch_matches_the_synchronous_switch_without_decoding_or_reprojecting() {
        for target in [Format::Code, Format::PlainText] {
            let mut prepared = markdown();
            let mut synchronous = markdown();
            prepared.prefetch_view_format(target);
            wait_until_ready(&prepared, target);
            let (result, work) = measure_document_work(|| prepared.set_view_format(target));
            result.unwrap();
            assert_eq!(work.source_decoded_bytes, 0, "{target:?}");
            let (result, work) = measure_document_work(|| synchronous.set_view_format(target));
            result.unwrap();
            assert!(work.source_decoded_bytes > 0, "{target:?}");
            assert_eq!(prepared.format(), target);
            assert_eq!(prepared.text(), synchronous.text());
            assert_eq!(prepared.source_bytes(), synchronous.source_bytes());
            assert_eq!(prepared.revision(), synchronous.revision());
            // Switching back restores the original presentation exactly.
            assert!(prepared.undo());
            assert_eq!(prepared.text(), markdown().text());
        }
    }

    #[test]
    fn an_edit_or_another_target_discards_the_preparation() {
        let mut document = markdown();
        document.prefetch_view_format(Format::Code);
        wait_until_ready(&document, Format::Code);
        assert!(!document.view_format_prefetch_ready(Format::PlainText));
        assert!(!document.view_format_prefetch_ready(Format::Markdown));
        document.replace(0..0, "x").unwrap();
        assert!(!document.view_format_prefetch_ready(Format::Code));
        let (result, work) = measure_document_work(|| document.set_view_format(Format::Code));
        result.unwrap();
        assert!(work.source_decoded_bytes > 0, "a stale preparation is never used");
        assert!(document.text().starts_with("# xTitle"), "{:?}", &document.text()[..12]);
    }
}
