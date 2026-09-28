//! Disposable, snapshot-bound syntax analysis. This domain has normalized
//! decoded UTF-8 coordinates, never physical source or frontend UTF-16 offsets.
use super::{FormattedTextError, FormattedTextTree};
use std::ops::Range;

pub mod detection;
pub mod languages;
pub mod service;
pub mod treesitter;
pub mod vim;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SyntaxInputIdentity {
    pub document: u64,
    pub revision: u64,
    pub generation: u64,
}

#[derive(Clone, Debug)]
pub struct SyntaxInputSnapshot {
    identity: SyntaxInputIdentity,
    text: FormattedTextTree,
}

impl SyntaxInputSnapshot {
    pub fn new(identity: SyntaxInputIdentity, text: FormattedTextTree) -> Self {
        Self { identity, text }
    }
    pub fn identity(&self) -> SyntaxInputIdentity {
        self.identity
    }
    pub fn text_tree(&self) -> &FormattedTextTree {
        &self.text
    }
    pub fn byte_len(&self) -> usize {
        self.text.byte_len()
    }
    pub fn chunk_at(&self, offset: usize) -> &[u8] {
        self.text.byte_chunk_at(offset)
    }
    pub fn slice(&self, range: Range<usize>) -> Result<String, FormattedTextError> {
        self.text.slice(range)
    }
}

/// A canonical syntax style name. Names are shared between the many runs
/// that reference them, so a run copies two pointers rather than two strings.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SyntaxStyleName(pub std::sync::Arc<str>);

impl SyntaxStyleName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::ops::Deref for SyntaxStyleName {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

impl PartialEq<str> for SyntaxStyleName {
    fn eq(&self, other: &str) -> bool {
        &*self.0 == other
    }
}

impl PartialEq<&str> for SyntaxStyleName {
    fn eq(&self, other: &&str) -> bool {
        &*self.0 == *other
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyntaxRun {
    pub range: Range<usize>,
    pub name: SyntaxStyleName,
    pub origin: std::sync::Arc<str>,
    pub priority: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Coverage {
    Exact,
    Provisional,
    Missing,
}
