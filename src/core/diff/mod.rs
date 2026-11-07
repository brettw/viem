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

pub use inline::InlineSpans;
pub use presentation::{Filler, Side, SidePresentation};
pub use state::{DiffLimits, DiffState, Hunk, LineEdit, LinePair, LineSource, TextLines};
