import CViemCore
import Foundation
import ViemCoreTextProvider

/// Captures bounded immutable work on the main actor, shapes on one app-wide
/// worker, then installs only if all core dependencies still match.
@MainActor
final class EVBackgroundLayout {
    enum Purpose { case viewport, tableWidths }
    private let purpose: Purpose
    private struct Dependencies: Equatable {
        let document, revision, configuration, environment, metrics: UInt64
        let width, height: Float
    }
    // Shared by table refinement and viewport pre-layout across every view.
    static let worker = DispatchQueue(label: "Viem.BackgroundLayout", qos: .utility)
    private weak var session: EVCoreViewSession?
    private var dependencies: Dependencies?
    private var active: Work?
    private var queued = false
    private var stopped = false
    private var top: Float = 0
    private var left: Float = 0
    private var direction: Int32 = 1
    /// Results discarded without progress since the last installation. Core
    /// ends speculation when its window is complete; this only guards against
    /// repeated stale captures under continuous change.
    private var consecutiveDiscards = 0
    private static let maximumConsecutiveDiscards = 8
    /// Installations since speculation was last idle.
    private var installedSinceIdle = 0
    var isEnabled = true {
        didSet { if !isEnabled { cancel() } }
    }
    private(set) var workerComputations = 0
    /// Worker time spent computing results, in seconds, and the longest one.
    private(set) var workerSeconds = 0.0
    private(set) var longestWorkerSeconds = 0.0
    private(set) var started = 0
    private(set) var installed = 0
    private(set) var discarded = 0
    private(set) var lastError: UInt32?
    var didInstall: (() -> Void)?
    /// Viewport speculation finished a window after installing new geometry.
    /// Exact heights may have changed the scroll extent since the last frame.
    var didFinish: (() -> Void)?
    var isIdle: Bool { !queued && active == nil }

    init(session: EVCoreViewSession, purpose: Purpose) {
        self.session = session; self.purpose = purpose
    }
    deinit { if let active { _ = viem_layout_work_cancel(active.request) } }

    func update() {
        guard isEnabled, let session, session.viewID != 0, !session.hasActiveComposition,
              let viewport = try? session.viewportState(),
              viewport.flags & UInt32(VIEM_VIEWPORT_STATE_HAS_LAYOUT) != 0,
              let layout = try? session.layoutSnapshotInfo()
        else { cancel(); return }
        let next = Dependencies(document: viewport.document_id, revision: viewport.document_revision,
            configuration: viewport.configuration_generation, environment: viewport.measurement_environment_id,
            metrics: viewport.metrics_generation, width: layout.viewport_width, height: layout.viewport_height)
        guard next != dependencies || top != viewport.top || left != viewport.left else { return }
        let nextDirection: Int32 = viewport.top > top ? 1 : viewport.top < top ? -1 : direction
        // In-flight viewport speculation stays useful across scrolling and
        // paint-only configuration changes: core validates its geometry
        // dependencies and window when it completes.
        let geometryChanged = dependencies.map {
            $0.document != next.document || $0.revision != next.revision || $0.environment != next.environment
                || $0.metrics != next.metrics || $0.width != next.width
        } ?? true
        if purpose == .viewport ? geometryChanged || left != viewport.left
            : next != dependencies || left != viewport.left || top != viewport.top {
            active?.cancel()
        }
        dependencies = next; top = viewport.top; left = viewport.left; direction = nextDirection
        consecutiveDiscards = 0
        stopped = false; lastError = nil
        queue()
    }

    func cancel() {
        active?.cancel(); dependencies = nil; stopped = true
    }

    private func queue() {
        guard isEnabled, !queued, active == nil, !stopped, dependencies != nil,
              consecutiveDiscards < Self.maximumConsecutiveDiscards else { return }
        queued = true
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.queued = false
            self.start()
        }
    }

    private func start() {
        guard isEnabled, !stopped, active == nil, let session, session.viewID != 0,
              !session.hasActiveComposition else { return }
        var request: UInt64 = 0
        let status = purpose == .tableWidths
            ? viem_core_view_prepare_table_refinement(session.document.core, session.viewID, &request)
            : viem_core_view_prepare_prelayout(session.document.core, session.viewID, direction, &request)
        guard status == UInt32(VIEM_STATUS_OK) else { stopped = true; lastError = status; return }
        guard request != 0 else {
            stopped = true
            if purpose == .viewport, installedSinceIdle > 0 {
                installedSinceIdle = 0
                didFinish?()
            }
            return
        }
        let work = Work(request: request, provider: session.provider.makeWorkerProvider())
        active = work; started += 1
        Self.worker.async { [weak self] in
            let begin = CFAbsoluteTimeGetCurrent()
            var provider = work.provider.makeProviderTable(), result: UInt64 = 0
            let status = viem_layout_work_compute(work.request, &provider, &result)
            let elapsed = CFAbsoluteTimeGetCurrent() - begin
            let finishedResult = result
            let computedOnWorker = !Thread.isMainThread
            DispatchQueue.main.async { [weak self] in
                guard let self else {
                    if finishedResult != 0 { _ = viem_layout_work_release(finishedResult) }
                    return
                }
                self.workerSeconds += elapsed
                self.longestWorkerSeconds = max(self.longestWorkerSeconds, elapsed)
                self.complete(work, result: finishedResult, status: status, computedOnWorker: computedOnWorker)
            }
        }
    }

    private func complete(_ work: Work, result: UInt64, status: UInt32, computedOnWorker: Bool) {
        guard active === work else { if result != 0 { _ = viem_layout_work_release(result) }; return }
        active = nil
        if computedOnWorker { workerComputations += 1 }
        guard isEnabled, !work.cancelled, !stopped, let session, session.viewID != 0 else {
            if result != 0 { _ = viem_layout_work_release(result) }
            discarded += 1; consecutiveDiscards += 1; queue(); return
        }
        guard status == UInt32(VIEM_STATUS_OK) else {
            if result != 0 { _ = viem_layout_work_release(result) }
            stopped = true; lastError = status; return
        }
        guard result != 0 else { discarded += 1; consecutiveDiscards += 1; queue(); return }
        var accepted: UInt8 = 0
        let status = purpose == .tableWidths
            ? viem_core_view_install_table_refinement(session.document.core, session.viewID, result, &accepted)
            : viem_core_view_install_prelayout(session.document.core, session.viewID, direction, result, &accepted)
        guard status == UInt32(VIEM_STATUS_OK) else {
            discarded += 1; stopped = true; lastError = status; return
        }
        // Visible work can supersede a chunk without changing dependencies.
        // Recapture from current state; preparation stops when the window or
        // table widths are ready, with bounded stale retries as a guard.
        guard accepted != 0 else { discarded += 1; consecutiveDiscards += 1; queue(); return }
        installed += 1; consecutiveDiscards = 0; installedSinceIdle += 1
        if purpose == .tableWidths {
            session.clearPresentationExportCache()
            didInstall?()
        }
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
