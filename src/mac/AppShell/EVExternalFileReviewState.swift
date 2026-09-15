import CViemCore
import Foundation

struct EVExternalFileReview {
    let canReload: Bool
    let discardsUnsavedChanges: Bool
}

/// Owns one portable observation/review state. File monitoring and document
/// revision validation remain the host's responsibility.
@MainActor
final class EVExternalFileReviewState {
    private let handle: ViemExternalFileReviewHandle

    init() {
        var handle: ViemExternalFileReviewHandle = 0
        let status = viem_external_file_review_create(&handle)
        precondition(status == VIEM_STATUS_OK && handle != 0, "Cannot create external file review state")
        self.handle = handle
    }

    deinit {
        let status = viem_external_file_review_destroy(handle)
        assert(status == VIEM_STATUS_OK, "Cannot destroy external file review state")
    }

    func reset() {
        let status = viem_external_file_review_reset(handle)
        precondition(status == VIEM_STATUS_OK, "Cannot reset external file review state")
    }

    /// The file returned to its baseline; leave any open review pending.
    func clearAcknowledged() {
        let status = viem_external_file_review_clear_acknowledged(handle)
        precondition(status == VIEM_STATUS_OK, "Cannot clear external file acknowledgement")
    }

    func begin(token: Data, canReload: Bool, isDirty: Bool) -> EVExternalFileReview? {
        var flags: UInt32 = 0
        let status = token.withUnsafeBytes { bytes in
            viem_external_file_review_begin(
                handle, bytes.bindMemory(to: UInt8.self).baseAddress, UInt64(bytes.count),
                canReload ? 1 : 0, isDirty ? 1 : 0, &flags
            )
        }
        precondition(status == VIEM_STATUS_OK, "Cannot begin external file review")
        guard flags & UInt32(VIEM_EXTERNAL_FILE_REVIEW_PRESENT) != 0 else { return nil }
        return EVExternalFileReview(
            canReload: flags & UInt32(VIEM_EXTERNAL_FILE_REVIEW_CAN_RELOAD) != 0,
            discardsUnsavedChanges: flags & UInt32(VIEM_EXTERNAL_FILE_REVIEW_DISCARDS_UNSAVED_CHANGES) != 0
        )
    }

    func finish(token: Data, acknowledge: Bool) {
        let status = token.withUnsafeBytes { bytes in
            viem_external_file_review_finish(
                handle, bytes.bindMemory(to: UInt8.self).baseAddress, UInt64(bytes.count),
                acknowledge ? 1 : 0
            )
        }
        precondition(status == VIEM_STATUS_OK, "Cannot finish external file review")
    }
}
