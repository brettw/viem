import CViemCore
import Foundation
import ViemCoreTextProvider

/// Captures bounded immutable work on the main actor, shapes on one app-wide
/// worker, then installs only if all core dependencies still match.
@MainActor
final class EVTableWidthRefinement {
    private struct Dependencies: Equatable {
        let document, revision, configuration, environment, metrics: UInt64
        let top, left, width, height: Float
    }
    private static let worker = DispatchQueue(label: "Viem.TableWidths", qos: .utility)
    private weak var session: EVCoreViewSession?
    private var dependencies: Dependencies?
    private var active: Work?
    private var queued = false
    private var stopped = false
    private(set) var started = 0
    private(set) var installed = 0
    private(set) var discarded = 0
    private(set) var lastError: UInt32?
    var didInstall: (() -> Void)?
    var isIdle: Bool { !queued && active == nil }

    init(session: EVCoreViewSession) { self.session = session }
    deinit { if let active { _ = viem_layout_work_cancel(active.request) } }

    func update() {
        guard let session, session.viewID != 0, !session.hasActiveComposition,
              let viewport = try? session.viewportState(), let layout = try? session.layoutSnapshotInfo()
        else { cancel(); return }
        let next = Dependencies(document: viewport.document_id, revision: viewport.document_revision,
            configuration: viewport.configuration_generation, environment: viewport.measurement_environment_id,
            metrics: viewport.metrics_generation, top: viewport.top, left: viewport.left,
            width: layout.viewport_width, height: layout.viewport_height)
        guard next != dependencies else { return }
        active?.cancel(); dependencies = next; stopped = false; lastError = nil
        queue()
    }

    func cancel() {
        active?.cancel(); dependencies = nil; stopped = true
    }

    private func queue() {
        guard !queued, active == nil, !stopped, dependencies != nil else { return }
        queued = true
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.queued = false
            self.start()
        }
    }

    private func start() {
        guard !stopped, active == nil, let session, session.viewID != 0,
              !session.hasActiveComposition else { return }
        var request: UInt64 = 0
        let status = viem_core_view_prepare_table_refinement(session.document.core, session.viewID, &request)
        guard status == UInt32(VIEM_STATUS_OK) else { stopped = true; lastError = status; return }
        guard request != 0 else { stopped = true; return }
        let work = Work(request: request, provider: session.provider.makeWorkerProvider())
        active = work; started += 1
        Self.worker.async { [weak self] in
            var provider = work.provider.makeProviderTable(), result: UInt64 = 0
            let status = viem_layout_work_compute(work.request, &provider, &result)
            let finishedResult = result
            DispatchQueue.main.async { [weak self] in
                guard let self else {
                    if finishedResult != 0 { _ = viem_layout_work_release(finishedResult) }
                    return
                }
                self.complete(work, result: finishedResult, status: status)
            }
        }
    }

    private func complete(_ work: Work, result: UInt64, status: UInt32) {
        guard active === work else { if result != 0 { _ = viem_layout_work_release(result) }; return }
        active = nil
        guard !work.cancelled, !stopped, let session, session.viewID != 0 else {
            if result != 0 { _ = viem_layout_work_release(result) }
            discarded += 1; queue(); return
        }
        guard status == UInt32(VIEM_STATUS_OK) else {
            if result != 0 { _ = viem_layout_work_release(result) }
            stopped = true; lastError = status; return
        }
        guard result != 0 else { discarded += 1; queue(); return }
        var accepted: UInt8 = 0
        let status = viem_core_view_install_table_refinement(session.document.core, session.viewID, result, &accepted)
        guard status == UInt32(VIEM_STATUS_OK) else {
            discarded += 1; stopped = true; lastError = status; return
        }
        // Visible work can supersede a chunk without changing the dependency
        // values. Recapture once from current state; prepare returns zero when
        // no provisional widths remain, so stale rejection never starts polling.
        guard accepted != 0 else { discarded += 1; queue(); return }
        installed += 1
        session.clearPresentationExportCache()
        didInstall?()
        queue()
    }

    private final class Work: @unchecked Sendable {
        let request: UInt64
        let provider: CoreTextMeasurementProvider
        // Only accessed on the main actor. The worker cancellation authority is
        // the portable atomic token behind request, never this convenience flag.
        var cancelled = false
        init(request: UInt64, provider: CoreTextMeasurementProvider) {
            self.request = request; self.provider = provider
        }
        func cancel() { cancelled = true; _ = viem_layout_work_cancel(request) }
        deinit { _ = viem_layout_work_release(request) }
    }
}
