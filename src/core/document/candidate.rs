//! Explicit outcomes for text-edit candidate strategies.
//!
//! Preparing an edit tries bounded strategies before complete reprojection.
//! Each attempt has exactly one of three outcomes:
//!
//! - **Verified:** the strategy built a candidate and proved it.
//! - **Declined:** the strategy cannot prove locality for this edit. This is
//!   expected; a broader strategy must try. A decline is never an error that a
//!   caller has to recognize.
//! - **Rejected:** the strategy's proof covers the complete change and its
//!   result contradicts the requested intention, or an invariant failed. No
//!   broader strategy can make the edit valid, so preparation stops.
//!
//! Strategies record the specific reason for a decline with [`decline`]. The
//! opt-in [`trace_candidate_strategies`] collects outcomes so locality tests and
//! the consistency checker can explain why an edit fell back to complete
//! reprojection, and which strategy produced a result.
//!
//! Shadow verification rebuilds each regionally verified candidate with
//! complete reprojection and compares their semantic projections. It is off by
//! default; [`verify_regional_candidates`] enables it for one operation, and
//! `VIEM_SHADOW_PROJECTION=panic|log` enables it for a whole process (for
//! example `cargo test`). Shadow work is excluded from work statistics and
//! candidate traces, so locality assertions remain meaningful while enabled.
use std::cell::{Cell, RefCell};
use std::sync::OnceLock;

use super::ModelTransactionError;

/// One bounded or complete way to build a text-edit candidate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum CandidateStrategy {
    /// Several literal edits proved independently in Text/Code.
    DisjointLiteral,
    /// A Markdown table row stub or row group.
    TableRow,
    /// A Markdown code or list paragraph reprojected with its owner's style.
    StructuredLocal,
    /// The edited hard lines, with retained list/quote context.
    LineLocal,
    /// A list owner and its independent neighbors.
    ListParagraph,
    /// Independently delimited paragraphs and guarded structural owners.
    IndependentParagraph,
    /// A table and its surrounding paragraphs after a source-intent change.
    TableLocal,
    /// The whole document, verified against the expected formatted text.
    Complete,
    /// A policy that skips every regional strategy for this edit.
    Regional,
}

impl CandidateStrategy {
    pub fn name(self) -> &'static str {
        match self {
            Self::DisjointLiteral => "disjoint-literal",
            Self::TableRow => "table-row",
            Self::StructuredLocal => "structured-local",
            Self::LineLocal => "line-local",
            Self::ListParagraph => "list-paragraph",
            Self::IndependentParagraph => "independent-paragraph",
            Self::TableLocal => "table-local",
            Self::Complete => "complete",
            Self::Regional => "regional",
        }
    }

    pub fn is_regional(self) -> bool {
        self != Self::Complete
    }
}

/// What one strategy concluded about one edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandidateOutcome {
    Verified,
    /// The strategy could not prove locality; the reason is a short static
    /// description of the failed gate.
    Declined(&'static str),
    Rejected,
    /// Shadow verification found that this verified regional candidate differs
    /// from complete reprojection of the same source.
    ShadowMismatch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CandidateEvent {
    pub strategy: CandidateStrategy,
    pub outcome: CandidateOutcome,
}

/// A regional candidate that differs from complete reprojection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShadowMismatch {
    pub strategy: CandidateStrategy,
    pub detail: String,
}

const TRACE_LIMIT: usize = 4096;

thread_local! {
    static TRACE: RefCell<Option<Vec<CandidateEvent>>> = const { RefCell::new(None) };
    static PENDING_DECLINE: Cell<Option<(CandidateStrategy, &'static str)>> = const { Cell::new(None) };
    static SHADOW: RefCell<Option<Vec<ShadowMismatch>>> = const { RefCell::new(None) };
    static SUSPENDED: Cell<bool> = const { Cell::new(false) };
}

/// Collect the candidate strategy outcomes of all synchronous document work in
/// `operation` on this thread. Nested traces are included in the outer trace.
/// At most a few thousand events are kept; later events are dropped.
pub fn trace_candidate_strategies<T>(operation: impl FnOnce() -> T) -> (T, Vec<CandidateEvent>) {
    struct Scope(Option<Vec<CandidateEvent>>);
    impl Drop for Scope {
        fn drop(&mut self) {
            TRACE.with(|trace| {
                let inner = trace.replace(None).unwrap_or_default();
                if let Some(mut outer) = self.0.take() {
                    let room = TRACE_LIMIT.saturating_sub(outer.len());
                    outer.extend(inner.into_iter().take(room));
                    trace.replace(Some(outer));
                }
            });
        }
    }
    let scope = Scope(TRACE.with(|trace| trace.replace(Some(Vec::new()))));
    let result = operation();
    let events = TRACE.with(|trace| trace.borrow().clone().unwrap_or_default());
    drop(scope);
    (result, events)
}

/// Run `operation` with shadow verification enabled on this thread and return
/// every regional candidate that differed from complete reprojection, instead
/// of following the process-wide `VIEM_SHADOW_PROJECTION` policy.
pub fn verify_regional_candidates<T>(operation: impl FnOnce() -> T) -> (T, Vec<ShadowMismatch>) {
    struct Scope(Option<Vec<ShadowMismatch>>);
    impl Drop for Scope {
        fn drop(&mut self) {
            SHADOW.with(|shadow| {
                let inner = shadow.replace(None).unwrap_or_default();
                if let Some(mut outer) = self.0.take() {
                    outer.extend(inner);
                    shadow.replace(Some(outer));
                }
            });
        }
    }
    let scope = Scope(SHADOW.with(|shadow| shadow.replace(Some(Vec::new()))));
    let result = operation();
    let mismatches = SHADOW.with(|shadow| shadow.borrow().clone().unwrap_or_default());
    drop(scope);
    (result, mismatches)
}

fn tracing() -> bool {
    !SUSPENDED.with(Cell::get) && TRACE.with(|trace| trace.borrow().is_some())
}

fn record(strategy: CandidateStrategy, outcome: CandidateOutcome) {
    if !tracing() {
        return;
    }
    TRACE.with(|trace| {
        if let Some(events) = trace.borrow_mut().as_mut() {
            if events.len() < TRACE_LIMIT {
                events.push(CandidateEvent { strategy, outcome });
            }
        }
    });
}

/// Note why `strategy` cannot prove the current edit and return `Ok(None)`,
/// the "not applicable" result of an optional strategy.
pub(super) fn decline<T, E>(strategy: CandidateStrategy, reason: &'static str) -> Result<Option<T>, E> {
    if tracing() {
        PENDING_DECLINE.with(|pending| pending.set(Some((strategy, reason))));
    }
    Ok(None)
}

fn take_decline_reason(strategy: CandidateStrategy, fallback: &'static str) -> &'static str {
    PENDING_DECLINE.with(|pending| match pending.take() {
        Some((noted, reason)) if noted == strategy => reason,
        _ => fallback,
    })
}

/// The explicit result of one strategy attempt.
pub(super) enum Attempt<T> {
    Verified(T),
    Declined(&'static str),
    Rejected(ModelTransactionError),
}

impl<T> Attempt<T> {
    /// Classify a strategy that reports "not applicable" as `None` and whose
    /// errors are final. A decline carries the reason noted by [`decline`].
    pub(super) fn optional(
        strategy: CandidateStrategy,
        result: Result<Option<T>, ModelTransactionError>,
    ) -> Self {
        match result {
            Ok(Some(value)) => Self::Verified(value),
            Ok(None) => Self::Declined(take_decline_reason(strategy, "not applicable")),
            Err(error) => Self::Rejected(error),
        }
    }

    /// Classify a strategy whose errors only mean that it could not prove
    /// the edit.
    pub(super) fn optional_or_declined(
        strategy: CandidateStrategy,
        result: Result<Option<T>, ModelTransactionError>,
        error_reason: &'static str,
    ) -> Self {
        match result {
            Ok(Some(value)) => Self::Verified(value),
            Ok(None) => Self::Declined(take_decline_reason(strategy, "not applicable")),
            Err(_) => Self::Declined(take_decline_reason(strategy, error_reason)),
        }
    }

    /// Record this final outcome of `strategy` in any active trace.
    pub(super) fn record(self, strategy: CandidateStrategy) -> Self {
        record(
            strategy,
            match &self {
                Self::Verified(_) => CandidateOutcome::Verified,
                Self::Declined(reason) => CandidateOutcome::Declined(reason),
                Self::Rejected(_) => CandidateOutcome::Rejected,
            },
        );
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShadowPolicy {
    Off,
    Log,
    Panic,
}

fn process_policy() -> ShadowPolicy {
    static POLICY: OnceLock<ShadowPolicy> = OnceLock::new();
    *POLICY.get_or_init(|| match std::env::var("VIEM_SHADOW_PROJECTION").as_deref() {
        Ok("log") => ShadowPolicy::Log,
        Ok("1" | "panic") => ShadowPolicy::Panic,
        _ => ShadowPolicy::Off,
    })
}

/// Shadow verification is limited to documents of at most this many source
/// bytes (default 1 MiB, `VIEM_SHADOW_PROJECTION_LIMIT` to override), so
/// large-document tests keep their bounded cost.
fn shadow_limit() -> usize {
    static LIMIT: OnceLock<usize> = OnceLock::new();
    *LIMIT.get_or_init(|| {
        std::env::var("VIEM_SHADOW_PROJECTION_LIMIT")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(1 << 20)
    })
}

/// Whether a regional candidate for a source of `source_len` bytes should be
/// compared with complete reprojection.
pub(super) fn shadow_enabled(source_len: usize) -> bool {
    !SUSPENDED.with(Cell::get)
        && source_len <= shadow_limit()
        && (SHADOW.with(|shadow| shadow.borrow().is_some()) || process_policy() != ShadowPolicy::Off)
}

/// Run shadow work without recording work statistics or candidate events.
pub(super) fn unobserved<T>(operation: impl FnOnce() -> T) -> T {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            SUSPENDED.with(|suspended| suspended.set(self.0));
        }
    }
    let _restore = Restore(SUSPENDED.with(|suspended| suspended.replace(true)));
    super::super::work_statistics::unobserved(operation)
}

/// Report a regional candidate that differs from complete reprojection.
pub(super) fn report_shadow_mismatch(strategy: CandidateStrategy, detail: String) {
    record(strategy, CandidateOutcome::ShadowMismatch);
    let collected = SHADOW.with(|shadow| {
        shadow
            .borrow_mut()
            .as_mut()
            .map(|mismatches| mismatches.push(ShadowMismatch { strategy, detail: detail.clone() }))
            .is_some()
    });
    if collected {
        return;
    }
    match process_policy() {
        ShadowPolicy::Panic => panic!(
            "shadow projection mismatch from the {} candidate strategy: {detail}",
            strategy.name()
        ),
        ShadowPolicy::Log => eprintln!(
            "viem: shadow projection mismatch from the {} candidate strategy: {detail}",
            strategy.name()
        ),
        ShadowPolicy::Off => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traces_nest_and_attach_noted_decline_reasons() {
        let ((), outer) = trace_candidate_strategies(|| {
            let _ = decline::<(), ()>(CandidateStrategy::LineLocal, "neighbor list");
            let _ = Attempt::<()>::optional(CandidateStrategy::LineLocal, Ok(None))
                .record(CandidateStrategy::LineLocal);
            let ((), inner) = trace_candidate_strategies(|| {
                let _ = Attempt::optional(CandidateStrategy::Complete, Ok(Some(())))
                    .record(CandidateStrategy::Complete);
            });
            assert_eq!(inner.len(), 1);
            let _ = Attempt::<()>::optional(CandidateStrategy::TableRow, Ok(None))
                .record(CandidateStrategy::TableRow);
        });
        assert_eq!(
            outer,
            vec![
                CandidateEvent {
                    strategy: CandidateStrategy::LineLocal,
                    outcome: CandidateOutcome::Declined("neighbor list"),
                },
                CandidateEvent { strategy: CandidateStrategy::Complete, outcome: CandidateOutcome::Verified },
                CandidateEvent {
                    strategy: CandidateStrategy::TableRow,
                    outcome: CandidateOutcome::Declined("not applicable"),
                },
            ]
        );
    }

    #[test]
    fn decline_reasons_do_not_leak_to_other_strategies_or_untraced_work() {
        let _ = decline::<(), ()>(CandidateStrategy::LineLocal, "untraced");
        let ((), events) = trace_candidate_strategies(|| {
            let _ = decline::<(), ()>(CandidateStrategy::LineLocal, "line reason");
            let _ = Attempt::<()>::optional(CandidateStrategy::IndependentParagraph, Ok(None))
                .record(CandidateStrategy::IndependentParagraph);
        });
        assert_eq!(events[0].outcome, CandidateOutcome::Declined("not applicable"));
    }

    #[test]
    fn scoped_shadow_verification_collects_instead_of_panicking() {
        let ((), mismatches) = verify_regional_candidates(|| {
            assert!(shadow_enabled(10));
            report_shadow_mismatch(CandidateStrategy::LineLocal, "differs".into());
            assert!(!unobserved(|| shadow_enabled(10)));
        });
        assert_eq!(mismatches.len(), 1);
        assert_eq!(mismatches[0].strategy, CandidateStrategy::LineLocal);
    }
}
