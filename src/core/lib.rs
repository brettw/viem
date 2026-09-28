//! Portable core for Viem.
//!
//! The crate intentionally exposes a small facade. Document state, Vim command
//! interpretation, and layout are separate modules with one-way dependencies.

pub mod command;
pub mod document;
pub mod ffi;
pub mod layout;

mod coordinator;

pub use command::clipboard::{
    ClipboardCommandContext, ClipboardContent, ClipboardContentError, ClipboardGeneration,
    ClipboardProvider, ClipboardSnapshot, ClipboardTarget, ClipboardWriteRequest,
    MemoryClipboardError, MemoryClipboardProvider,
};
pub use command::composition::{
    CompositionCommit, CompositionCommitRequest, CompositionError, CompositionEvent,
    CompositionOverlay, CompositionRestoration, CompositionSession, CompositionTarget,
    CompositionUpdate,
};
pub use command::layout_motion::{LayoutDemand, LayoutDemandEdge};
pub use coordinator::{
    DocumentMode, DocumentModeState, CompletionPopupAnchor, CompositionCancelReason, Core, CoreError, CoreEvent, CoreIdentifierKind, CoreOutcome,
    LogicalSelectionIdentity, LogicalSelectionKind, SemanticStylePresentation, SemanticStyleState,
    StyleEditGroup, StyleEditGroupError, StyleEditGroupId, ViewCompositionChange,
    ViewCompositionOutcome, ViewId, ViewRemovalOutcome, ViewRestoration, ViewportState,
};
pub use document::{Document, DocumentError, Encoding, Format, Revision};
