//! Diff mode services: comparing a base version of a document with its live
//! buffer. See `docs/diff_mode.md`.
//!
//! This module is portable logic. It doesn't depend on the document model,
//! command interpretation or layout; the coordinator adapts documents to
//! [`LineSource`] and installs the resulting [`SidePresentation`]s.

pub mod base;
pub mod inline;
pub mod presentation;
pub mod sequence;
pub mod state;

/// Unchanged lines kept visible beside a change: around collapsed context,
/// and above a change too tall to fit in the view when navigating to it.
pub const DIFF_CONTEXT_LINES: usize = 3;

pub use inline::InlineSpans;
pub use presentation::{CollapsibleRun, Filler, Side, SidePresentation};
pub use state::{DiffLimits, DiffState, Hunk, LineEdit, LinePair, LineSource, TextLines};
