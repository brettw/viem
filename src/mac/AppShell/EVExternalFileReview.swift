import AppKit
import CryptoKit

enum EVExternalFileDecision {
  case keepBuffer, loadFile
}

/// One observed disk state. The token identifies content and file identity,
/// rather than just a timestamp or the kind of change.
struct EVExternalFileObservation: Sendable {
  let change: EVExternalFileChange
  let fingerprint: EVFileFingerprint?

  var token: Data {
    let identity: String
    if let fingerprint {
      identity = "file:\(fingerprint.device.map(String.init) ?? "-"):\(fingerprint.inode.map(String.init) ?? "-"):\(fingerprint.digest?.base64EncodedString() ?? "missing")"
    } else {
      identity = "error:\(change.message)"
    }
    return Data(SHA256.hash(data: Data(identity.utf8)))
  }

  var canReload: Bool {
    switch change {
    case .modified, .replaced: true
    case .deleted, .unreadable: false
    }
  }
}

@MainActor
extension EVDocument {
  func resetExternalFileReview() {
    externalFileObservation = nil
    externalFileReviewState.reset()
  }

  func stopExternalFileMonitoring() {
    externalFileMonitor?.stop()
    externalFileMonitor = nil
    externalFileMonitorURL = nil
    for observer in externalFileMonitorObservers { NotificationCenter.default.removeObserver(observer) }
    externalFileMonitorObservers.removeAll()
  }

  func updateExternalFileMonitor(at url: URL?) {
    guard !externalFileMonitoringClosed, let url, url.isFileURL, fileBaseline != nil else {
      stopExternalFileMonitoring()
      return
    }
    let target = url.standardizedFileURL
    guard target != externalFileMonitorURL else { return }
    stopExternalFileMonitoring()
    externalFileMonitorURL = target
    externalFileMonitor = EVFileChangeMonitor(url: target) { [weak self] in
      self?.requestExternalFileReview()
    }
    let center = NotificationCenter.default
    externalFileMonitorObservers.append(center.addObserver(
      forName: NSApplication.didBecomeActiveNotification, object: nil, queue: .main
    ) { [weak self] _ in
      MainActor.assumeIsolated { self?.requestExternalFileReview() }
    })
    externalFileMonitorObservers.append(center.addObserver(
      forName: NSWindow.didEndSheetNotification, object: nil, queue: .main
    ) { [weak self] notification in
      MainActor.assumeIsolated {
        guard let self, let window = notification.object as? NSWindow,
              self.windowForSheet === window else { return }
        // Wait until AppKit has removed the previous sheet before offering
        // another buffer's change in a shared split window.
        DispatchQueue.main.async { [weak self] in self?.requestExternalFileReview() }
      }
    })
  }

  func updateExternalFileMonitorForBaseline(at url: URL) {
    // Keep watching the selected alias as well as its current vnode. A save
    // receipt names the canonical target, but must not stop detecting a later
    // retarget of the user's symlink. Save As adopts its new path separately.
    if let selected = fileURL,
       EVDocumentIdentity.canonicalURL(selected) == EVDocumentIdentity.canonicalURL(url) {
      updateExternalFileMonitor(at: selected)
    } else { updateExternalFileMonitor(at: url) }
  }

  func beginExternalFileWrite() { externalFileWriteCount += 1 }

  func endExternalFileWrite() {
    externalFileWriteCount -= 1
    if externalFileWriteCount == 0 {
      // The save receipt installs its baseline first. Thus our own atomic
      // replacement cannot produce a spurious external-change prompt.
      requestExternalFileReview()
    }
  }

  func requestExternalFileReview() {
    guard externalFileMonitor != nil, externalFileWriteCount == 0 else { return }
    checkForExternalChangesAndReview { [weak self] result in
      if case let .failure(error) = result { self?.presentError(error) }
    }
  }

  func checkForExternalChangesAndReview(
    completion: @escaping @MainActor (Result<String?, Error>) -> Void
  ) {
    guard externalFileWriteCount == 0 else { completion(.success(nil)); return }
    checkForExternalChanges { [weak self] change in
      guard let self else { completion(.success(nil)); return }
      guard change != nil else { completion(.success("File unchanged.")); return }
      guard let observation = self.externalFileObservation else {
        completion(.success(nil)); return
      }
      self.reviewExternalFileChange(observation, completion: completion)
    }
  }

  private func reviewExternalFileChange(
    _ observation: EVExternalFileObservation,
    completion: @escaping @MainActor (Result<String?, Error>) -> Void
  ) {
    let window = windowForSheet
    // File I/O continues in the background, but a prompt only appears while
    // the document is visible and the app is active. Activation retries it.
    guard externalFileWriteCount == 0,
          externalFileReviewDecisionHandler != nil
            || (NSApplication.shared.isActive && window?.isVisible == true && window?.attachedSheet == nil),
          let target = fileURL ?? fileBaselineURL,
          let review = externalFileReviewState.begin(token: observation.token,
            canReload: observation.canReload,
            isDirty: editorBackend.persistenceState.isDirty || isDocumentEdited)
    else { completion(.success(nil)); return }

    let generation = fileBaselineGeneration
    let state = editorBackend.persistenceState
    let decide: (EVExternalFileDecision) -> Void = { [weak self] decision in
      guard let self else { completion(.success(nil)); return }
      guard self.fileBaselineGeneration == generation,
            (self.fileURL ?? self.fileBaselineURL)?.standardizedFileURL == target.standardizedFileURL else {
        completion(.success(nil)); return
      }
      switch decision {
      case .keepBuffer:
        self.externalFileReviewState.finish(token: observation.token,
          acknowledge: self.externalFileObservation?.token == observation.token)
        // Acknowledgement suppresses repeat prompts only. The save baseline
        // remains intact, so a later overwrite still requires Save Anyway.
        completion(.success(nil))
        self.requestExternalFileReview()
      case .loadFile:
        guard review.canReload else {
          self.externalFileReviewState.finish(token: observation.token, acknowledge: true)
          completion(.success(nil)); return
        }
        let current = self.editorBackend.persistenceState
        guard current.documentID == state.documentID, current.documentRevision == state.documentRevision,
              current.isDirty == state.isDirty, self.externalFileWriteCount == 0 else {
          self.externalFileReviewState.finish(token: observation.token, acknowledge: false)
          completion(.success(nil))
          self.requestExternalFileReview()
          return
        }
        self.reloadReviewedFile(at: target, observation: observation, generation: generation,
          state: state, completion: completion)
      }
    }
    if let handler = externalFileReviewDecisionHandler {
      handler(observation.change, review, decide)
      return
    }
    guard let window else {
      externalFileReviewState.finish(token: observation.token, acknowledge: false)
      completion(.success(nil)); return
    }
    let alert = NSAlert()
    alert.alertStyle = review.discardsUnsavedChanges ? .critical : .warning
    alert.messageText = "“\(target.lastPathComponent)” changed outside Viem."
    var explanation = observation.change.message
    if review.canReload {
      explanation += "\n\nKeep Buffer continues editing the version in Viem. Load File replaces it with the version on disk."
      if review.discardsUnsavedChanges {
        explanation += "\n\nThis buffer has unsaved changes. Load File will discard them in every view of this buffer."
      }
    } else {
      explanation += "\n\nKeep Buffer preserves your text. You can save it to another location, or save to recreate the file if it was deleted."
    }
    alert.informativeText = explanation
    alert.addButton(withTitle: "Keep Buffer")
    if review.canReload { alert.addButton(withTitle: "Load File") }
    alert.beginSheetModal(for: window) { response in
      decide(response == .alertSecondButtonReturn ? .loadFile : .keepBuffer)
    }
  }

  private func reloadReviewedFile(
    at target: URL, observation: EVExternalFileObservation, generation: UInt64,
    state: EVDocumentPersistenceState,
    completion: @escaping @MainActor (Result<String?, Error>) -> Void
  ) {
    let readSnapshot = externalFileSnapshotReader
    DispatchQueue.global(qos: .utility).async { [weak self] in
      let observed = Result { try readSnapshot(target) }
      DispatchQueue.main.async {
        guard let self else { completion(.success(nil)); return }
        guard self.fileBaselineGeneration == generation else { completion(.success(nil)); return }
        let current = self.editorBackend.persistenceState
        guard current.documentID == state.documentID, current.documentRevision == state.documentRevision,
              current.isDirty == state.isDirty, self.externalFileWriteCount == 0 else {
          self.externalFileReviewState.finish(token: observation.token, acknowledge: false)
          completion(.success(nil))
          self.requestExternalFileReview(); return
        }
        do {
          let snapshot = try observed.get()
          guard snapshot.fingerprint == observation.fingerprint else {
            self.externalFileReviewState.finish(token: observation.token, acknowledge: false)
            completion(.success(nil))
            self.requestExternalFileReview(); return
          }
          // Install these verified bytes; reopening the path here could load
          // another writer's newer, unreviewed replacement.
          try self.installExternalFileSnapshot(snapshot, from: target)
          self.externalFileReviewState.finish(token: observation.token, acknowledge: true)
          completion(.success(nil))
          self.requestExternalFileReview()
        } catch {
          self.externalFileReviewState.finish(token: observation.token, acknowledge: false)
          completion(.failure(error))
        }
      }
    }
  }
}
