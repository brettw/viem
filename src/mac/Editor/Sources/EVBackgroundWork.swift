import CViemCore
import Foundation

/// A core owns the retained callback context; queued deliveries hold only a
/// weak reference. Workers may signal from any thread, but all service and
/// presentation work stays on the main actor.
final class EVBackgroundWorkWake: @unchecked Sendable {
    typealias Delivery = @MainActor @Sendable () -> Void
    typealias Enqueue = @Sendable (@escaping Delivery) -> Void

    private let lock = NSLock()
    private var active = true
    private var queued = false
    private let enqueue: Enqueue
    @MainActor var deliver: Delivery?

    init(enqueue: @escaping Enqueue = { delivery in DispatchQueue.main.async { delivery() } }) {
        self.enqueue = enqueue
    }

    var hasQueuedDelivery: Bool { lock.withLock { active && queued } }

    func request() {
        let shouldEnqueue = lock.withLock {
            guard active, !queued else { return false }
            queued = true
            return true
        }
        guard shouldEnqueue else { return }
        enqueue { [weak self] in
            guard let self, self.beginDelivery() else { return }
            self.deliver?()
        }
    }

    private func beginDelivery() -> Bool {
        lock.withLock {
            guard active, queued else { return false }
            // Clear before servicing: a completion racing with the drain must
            // enqueue another delivery, rather than disappear at its end.
            queued = false
            return true
        }
    }

    func invalidate() {
        lock.withLock { active = false; queued = false }
    }

    var callbacks: ViemBackgroundWakeCallbacks {
        var result = ViemBackgroundWakeCallbacks()
        result.context = Unmanaged.passUnretained(self).toOpaque()
        result.wake = { context in
            guard let context else { return }
            Unmanaged<EVBackgroundWorkWake>.fromOpaque(context).takeUnretainedValue().request()
        }
        result.retain = { context in
            guard let context else { return }
            _ = Unmanaged<EVBackgroundWorkWake>.fromOpaque(context).retain()
        }
        result.release = { context in
            guard let context else { return }
            Unmanaged<EVBackgroundWorkWake>.fromOpaque(context).release()
        }
        return result
    }
}

/// Services readiness notifications and explicit core deadlines. There is no
/// clock while a worker is running or after background work has settled.
@MainActor
final class EVBackgroundWork {
    typealias Service = @MainActor () -> ViemBackgroundWorkStatus?
    typealias MakeDeadline = @MainActor (UInt32, @escaping EVBackgroundWorkWake.Delivery) -> Timer

    let wake: EVBackgroundWorkWake
    var service: Service
    private let makeDeadline: MakeDeadline
    private var deadline: Timer?
    private var cancelled = false
    private(set) var serviceCount = 0
    var hasScheduledService: Bool { wake.hasQueuedDelivery || deadline != nil }

    init(enqueue: @escaping EVBackgroundWorkWake.Enqueue = { delivery in DispatchQueue.main.async { delivery() } },
         makeDeadline: @escaping MakeDeadline = EVBackgroundWork.makeTimer,
         service: @escaping Service) {
        self.service = service
        self.makeDeadline = makeDeadline
        wake = EVBackgroundWorkWake(enqueue: enqueue)
        wake.deliver = { [weak self] in self?.serviceNow() }
    }

    deinit {
        wake.invalidate()
        deadline?.invalidate()
    }

    func cancel() {
        cancelled = true
        wake.invalidate()
        deadline?.invalidate()
        deadline = nil
    }

    func serviceNow() {
        guard !cancelled else { return }
        deadline?.invalidate()
        deadline = nil
        serviceCount += 1
        guard let status = service() else { return }
        if status.more_ready != 0 || status.next_deadline_ms == 0 {
            wake.request()
        } else if status.next_deadline_ms != UInt32.max {
            deadline = makeDeadline(status.next_deadline_ms) { [weak self] in
                self?.wake.request()
            }
        }
    }

    private static func makeTimer(milliseconds: UInt32, deliver: @escaping EVBackgroundWorkWake.Delivery) -> Timer {
        let timer = Timer(timeInterval: Double(milliseconds) / 1000, repeats: false) { _ in
            MainActor.assumeIsolated { deliver() }
        }
        RunLoop.main.add(timer, forMode: .common)
        return timer
    }
}
