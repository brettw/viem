//! Host-independent source-artifact persistence contracts.
//!
//! The core prepares an immutable, revision-tagged write before any external
//! I/O. A host can execute [`AtomicArtifactWrite`] on any suitable executor and
//! later return a small [`ArtifactWriteCompletion`] to the serial document
//! coordinator. No document borrow, lock, or derived projection is needed
//! while the storage provider runs.

use super::{DocumentId, HistoryLocation, Revision};
use std::collections::BTreeMap;
use std::fmt;
use std::ops::Range;
use std::sync::Arc;

/// An owned, length-delimited provider path.
///
/// Paths deliberately are not `String` or `std::path::PathBuf`: a future C ABI
/// can exchange the exact byte sequence without a sentinel terminator or an
/// assumption about the host's path encoding.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ArtifactPath(Box<[u8]>);

impl ArtifactPath {
    pub fn new(bytes: impl Into<Vec<u8>>) -> Self {
        Self(bytes.into().into_boxed_slice())
    }

    pub fn from_utf8(path: impl AsRef<str>) -> Self {
        Self::new(path.as_ref().as_bytes().to_vec())
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.0.into_vec()
    }
}

impl From<Vec<u8>> for ArtifactPath {
    fn from(value: Vec<u8>) -> Self {
        Self::new(value)
    }
}

impl From<&str> for ArtifactPath {
    fn from(value: &str) -> Self {
        Self::from_utf8(value)
    }
}

impl From<String> for ArtifactPath {
    fn from(value: String) -> Self {
        Self::new(value.into_bytes())
    }
}

/// An owned, provider-defined logical artifact identity.
///
/// The identity is opaque to the editor. Providers should keep it stable for
/// atomic replacement of the same logical artifact.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ArtifactIdentity(Box<[u8]>);

impl ArtifactIdentity {
    pub fn new(bytes: impl Into<Vec<u8>>) -> Self {
        Self(bytes.into().into_boxed_slice())
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.0.into_vec()
    }
}

/// The persisted artifact to which a document is currently bound.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactBinding {
    path: ArtifactPath,
    identity: ArtifactIdentity,
}

impl ArtifactBinding {
    pub fn new(path: ArtifactPath, identity: ArtifactIdentity) -> Self {
        Self { path, identity }
    }

    pub fn path(&self) -> &ArtifactPath {
        &self.path
    }

    pub fn identity(&self) -> &ArtifactIdentity {
        &self.identity
    }
}

/// Exact bytes and logical identity returned by a provider read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadedArtifact {
    binding: ArtifactBinding,
    bytes: Arc<[u8]>,
}

impl LoadedArtifact {
    pub fn new(binding: ArtifactBinding, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            binding,
            bytes: Arc::from(bytes.into()),
        }
    }

    pub fn binding(&self) -> &ArtifactBinding {
        &self.binding
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(crate) fn into_parts(self) -> (ArtifactBinding, Vec<u8>) {
        (self.binding, self.bytes.as_ref().to_vec())
    }
}

/// Whether the provider may replace an artifact already at the destination.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtifactOverwrite {
    RefuseExisting,
    ReplaceExisting,
}

/// Byte extent captured by an ordinary alternate-path write.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArtifactWriteScope {
    WholeArtifact,
    /// Exact byte range in the authoritative primary source part. Ex hard-line
    /// ranges must be translated to this source extent before preparation.
    PrimarySourceBytes(Range<usize>),
}

/// Persistence meaning of a requested write.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArtifactWriteIntent {
    /// Persist the whole current source to its existing binding and establish
    /// a save point. An unbound document cannot prepare this request.
    Save { overwrite: ArtifactOverwrite },
    /// Persist the whole source, establish a save point, and adopt the
    /// successful receipt's destination and identity.
    SaveAs {
        destination: ArtifactPath,
        overwrite: ArtifactOverwrite,
    },
    /// Write exact source bytes elsewhere without changing the document's file
    /// identity or save point. This also represents ranged `:write` requests.
    WriteAlternate {
        destination: ArtifactPath,
        scope: ArtifactWriteScope,
        overwrite: ArtifactOverwrite,
    },
}

/// Normalized persistence effect retained by a prepared write.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtifactWritePurpose {
    Save,
    SaveAs,
    WriteAlternate,
}

/// Storage-only request extracted from a prepared document write.
///
/// This value owns its byte payload, so executing it never borrows mutable
/// model state and cannot accidentally serialize a newer document revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AtomicArtifactWrite {
    destination: ArtifactPath,
    bytes: Arc<[u8]>,
    overwrite: ArtifactOverwrite,
}

impl AtomicArtifactWrite {
    pub(crate) fn new(
        destination: ArtifactPath,
        bytes: Arc<[u8]>,
        overwrite: ArtifactOverwrite,
    ) -> Self {
        Self {
            destination,
            bytes,
            overwrite,
        }
    }

    pub fn destination(&self) -> &ArtifactPath {
        &self.destination
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn overwrite(&self) -> ArtifactOverwrite {
        self.overwrite
    }
}

/// Provider acknowledgement of one successful atomic write.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactWriteReceipt {
    destination: ArtifactPath,
    identity: ArtifactIdentity,
}

impl ArtifactWriteReceipt {
    pub fn new(destination: ArtifactPath, identity: ArtifactIdentity) -> Self {
        Self {
            destination,
            identity,
        }
    }

    pub fn destination(&self) -> &ArtifactPath {
        &self.destination
    }

    pub fn identity(&self) -> &ArtifactIdentity {
        &self.identity
    }

    pub(crate) fn into_binding(self) -> ArtifactBinding {
        ArtifactBinding::new(self.destination, self.identity)
    }
}

/// Narrow host service for exact reads and atomic whole-payload writes.
pub trait ArtifactStorageProvider {
    type Error;

    fn read_artifact(&mut self, path: &ArtifactPath) -> Result<LoadedArtifact, Self::Error>;

    /// On success the destination contains all request bytes; on failure its
    /// prior observable contents must remain intact.
    fn write_artifact_atomically(
        &mut self,
        request: &AtomicArtifactWrite,
    ) -> Result<ArtifactWriteReceipt, Self::Error>;
}

/// Execute only the external-I/O half of a prepared write.
///
/// This free function intentionally cannot access a [`super::Document`].
pub fn execute_prepared_artifact_write<P: ArtifactStorageProvider>(
    provider: &mut P,
    prepared: &PreparedArtifactWrite,
) -> Result<ArtifactWriteReceipt, P::Error> {
    provider.write_artifact_atomically(prepared.storage_request())
}

/// Read an artifact without constructing or mutating a document.
pub fn load_artifact<P: ArtifactStorageProvider>(
    provider: &mut P,
    path: &ArtifactPath,
) -> Result<LoadedArtifact, P::Error> {
    provider.read_artifact(path)
}

/// Non-reusing identity of one prepared write.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ArtifactWriteToken(u64);

impl ArtifactWriteToken {
    /// Construct a token received through a host boundary. The owning
    /// document still validates whether it is known and pending.
    pub const fn from_u64(value: u64) -> Self {
        Self(value)
    }

    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

/// Immutable document-side capture for one external write.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedArtifactWrite {
    pub(crate) token: ArtifactWriteToken,
    pub(crate) document: DocumentId,
    pub(crate) revision: Revision,
    pub(crate) history: HistoryLocation,
    pub(crate) purpose: ArtifactWritePurpose,
    pub(crate) storage: AtomicArtifactWrite,
}

impl PreparedArtifactWrite {
    pub fn token(&self) -> ArtifactWriteToken {
        self.token
    }

    pub fn document(&self) -> DocumentId {
        self.document
    }

    pub fn revision(&self) -> Revision {
        self.revision
    }

    pub fn history_location(&self) -> HistoryLocation {
        self.history
    }

    pub fn purpose(&self) -> ArtifactWritePurpose {
        self.purpose
    }

    pub fn storage_request(&self) -> &AtomicArtifactWrite {
        &self.storage
    }

    pub fn succeeded(&self, receipt: ArtifactWriteReceipt) -> ArtifactWriteCompletion {
        ArtifactWriteCompletion::succeeded(self.document, self.token, receipt)
    }

    pub fn failed(&self) -> ArtifactWriteCompletion {
        ArtifactWriteCompletion::failed(self.document, self.token)
    }
}

/// Result sent back to the serial document coordinator after external I/O.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactWriteCompletion {
    document: DocumentId,
    token: ArtifactWriteToken,
    result: ArtifactWriteResult,
}

impl ArtifactWriteCompletion {
    pub fn succeeded(
        document: DocumentId,
        token: ArtifactWriteToken,
        receipt: ArtifactWriteReceipt,
    ) -> Self {
        Self {
            document,
            token,
            result: ArtifactWriteResult::Succeeded(receipt),
        }
    }

    pub fn failed(document: DocumentId, token: ArtifactWriteToken) -> Self {
        Self {
            document,
            token,
            result: ArtifactWriteResult::Failed,
        }
    }

    pub fn document(&self) -> DocumentId {
        self.document
    }

    pub fn token(&self) -> ArtifactWriteToken {
        self.token
    }

    pub(crate) fn into_result(self) -> ArtifactWriteResult {
        self.result
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ArtifactWriteResult {
    Succeeded(ArtifactWriteReceipt),
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArtifactWriteCompletionStatus {
    Succeeded {
        token: ArtifactWriteToken,
        written_revision: Revision,
        save_point: Option<HistoryLocation>,
        file_identity_changed: bool,
        document_is_dirty: bool,
    },
    Failed {
        token: ArtifactWriteToken,
        written_revision: Revision,
        document_is_dirty: bool,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PersistenceError {
    ReadOnly,
    NoCurrentArtifact,
    InvalidSourceRange {
        start: usize,
        end: usize,
        source_length: usize,
    },
    DestinationWritePending(ArtifactPath),
    WriteIdentityExhausted,
    WrongDocument {
        expected: DocumentId,
        actual: DocumentId,
    },
    UnknownCompletion(ArtifactWriteToken),
    DuplicateCompletion(ArtifactWriteToken),
    ReceiptDestinationMismatch,
    ReceiptIdentityMismatch,
    StaleCompletion(ArtifactWriteToken),
}

impl fmt::Display for PersistenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReadOnly => {
                formatter.write_str("E45: readonly option is set (use ! to override)")
            }
            Self::NoCurrentArtifact => {
                formatter.write_str("the document has no current persisted artifact")
            }
            Self::InvalidSourceRange {
                start,
                end,
                source_length,
            } => write!(
                formatter,
                "source write range {start}..{end} is invalid for {source_length} bytes"
            ),
            Self::DestinationWritePending(path) => {
                write!(
                    formatter,
                    "artifact path {path:?} already has a pending write"
                )
            }
            Self::WriteIdentityExhausted => {
                formatter.write_str("artifact write identities were exhausted")
            }
            Self::WrongDocument { expected, actual } => write!(
                formatter,
                "write completion belongs to document {actual:?}, not {expected:?}"
            ),
            Self::UnknownCompletion(token) => {
                write!(formatter, "artifact write {token:?} is not pending")
            }
            Self::DuplicateCompletion(token) => {
                write!(formatter, "artifact write {token:?} was already completed")
            }
            Self::ReceiptDestinationMismatch => {
                formatter.write_str("write receipt names a different destination")
            }
            Self::ReceiptIdentityMismatch => {
                formatter.write_str("ordinary save changed the logical artifact identity")
            }
            Self::StaleCompletion(token) => {
                write!(formatter, "artifact write {token:?} was superseded")
            }
        }
    }
}

impl std::error::Error for PersistenceError {}

#[derive(Clone, Debug)]
pub(crate) enum PendingArtifactWriteKind {
    Save {
        captured_binding: ArtifactBinding,
        sequence: u64,
    },
    SaveAs {
        sequence: u64,
    },
    Alternate,
}

#[derive(Clone, Debug)]
pub(crate) struct PendingArtifactWrite {
    pub prepared: PreparedArtifactWrite,
    pub kind: PendingArtifactWriteKind,
}

/// Deterministic provider used by core unit and integration tests.
///
/// It is also useful to embedders that need a no-filesystem harness. Logical
/// identities are stable across replacement and assigned in insertion order.
#[derive(Clone, Debug)]
pub struct InMemoryArtifactStorage {
    artifacts: BTreeMap<ArtifactPath, MemoryArtifact>,
    next_identity: u64,
    fail_next_write: bool,
}

impl Default for InMemoryArtifactStorage {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug)]
struct MemoryArtifact {
    identity: ArtifactIdentity,
    bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InMemoryStorageError {
    NotFound(ArtifactPath),
    AlreadyExists(ArtifactPath),
    InjectedWriteFailure,
    IdentityExhausted,
}

impl fmt::Display for InMemoryStorageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(path) => write!(formatter, "artifact path {path:?} was not found"),
            Self::AlreadyExists(path) => {
                write!(formatter, "artifact path {path:?} already exists")
            }
            Self::InjectedWriteFailure => formatter.write_str("injected atomic-write failure"),
            Self::IdentityExhausted => formatter.write_str("artifact identities were exhausted"),
        }
    }
}

impl std::error::Error for InMemoryStorageError {}

impl InMemoryArtifactStorage {
    pub fn new() -> Self {
        Self {
            artifacts: BTreeMap::new(),
            next_identity: 1,
            fail_next_write: false,
        }
    }

    pub fn insert(
        &mut self,
        path: ArtifactPath,
        bytes: impl Into<Vec<u8>>,
    ) -> Result<ArtifactBinding, InMemoryStorageError> {
        if self.artifacts.contains_key(&path) {
            return Err(InMemoryStorageError::AlreadyExists(path));
        }
        let identity = self.allocate_identity()?;
        self.artifacts.insert(
            path.clone(),
            MemoryArtifact {
                identity: identity.clone(),
                bytes: bytes.into(),
            },
        );
        Ok(ArtifactBinding::new(path, identity))
    }

    pub fn fail_next_write(&mut self) {
        self.fail_next_write = true;
    }

    pub fn bytes(&self, path: &ArtifactPath) -> Option<&[u8]> {
        self.artifacts
            .get(path)
            .map(|artifact| artifact.bytes.as_slice())
    }

    pub fn identity(&self, path: &ArtifactPath) -> Option<&ArtifactIdentity> {
        self.artifacts.get(path).map(|artifact| &artifact.identity)
    }

    fn allocate_identity(&mut self) -> Result<ArtifactIdentity, InMemoryStorageError> {
        let value = self.next_identity;
        self.next_identity = self
            .next_identity
            .checked_add(1)
            .ok_or(InMemoryStorageError::IdentityExhausted)?;
        Ok(ArtifactIdentity::new(value.to_be_bytes().to_vec()))
    }
}

impl ArtifactStorageProvider for InMemoryArtifactStorage {
    type Error = InMemoryStorageError;

    fn read_artifact(&mut self, path: &ArtifactPath) -> Result<LoadedArtifact, Self::Error> {
        let artifact = self
            .artifacts
            .get(path)
            .ok_or_else(|| InMemoryStorageError::NotFound(path.clone()))?;
        Ok(LoadedArtifact::new(
            ArtifactBinding::new(path.clone(), artifact.identity.clone()),
            artifact.bytes.clone(),
        ))
    }

    fn write_artifact_atomically(
        &mut self,
        request: &AtomicArtifactWrite,
    ) -> Result<ArtifactWriteReceipt, Self::Error> {
        if self.fail_next_write {
            self.fail_next_write = false;
            return Err(InMemoryStorageError::InjectedWriteFailure);
        }
        if request.overwrite == ArtifactOverwrite::RefuseExisting
            && self.artifacts.contains_key(&request.destination)
        {
            return Err(InMemoryStorageError::AlreadyExists(
                request.destination.clone(),
            ));
        }

        let identity = match self.artifacts.get(&request.destination) {
            Some(existing) => existing.identity.clone(),
            None => self.allocate_identity()?,
        };
        let replacement = MemoryArtifact {
            identity: identity.clone(),
            bytes: request.bytes.as_ref().to_vec(),
        };
        self.artifacts
            .insert(request.destination.clone(), replacement);
        Ok(ArtifactWriteReceipt::new(
            request.destination.clone(),
            identity,
        ))
    }
}
