import AppKit
import CViemCore
import ViemAppShell
import XCTest

@testable import ViemEditor

@MainActor
final class EVBackgroundWorkTests: XCTestCase {
    func testSettledWorkCoalescesWakeupsAndSchedulesNoIdleTimer() {
        let queue = ManualQueue()
        let clock = ManualClock()
        var services = 0
        let work = EVBackgroundWork(enqueue: queue.enqueue, makeDeadline: clock.makeDeadline) {
            services += 1
            return self.status()
        }
        let callbacks = work.wake.callbacks
        for _ in 0..<100 { callbacks.wake?(callbacks.context) }
        XCTAssertEqual(queue.count, 1)
        XCTAssertTrue(queue.runNext())
        XCTAssertEqual(services, 1)
        XCTAssertFalse(work.hasScheduledService)
        XCTAssertTrue(clock.timers.isEmpty)
        XCTAssertFalse(queue.runNext())

        // Waiting on a worker has exactly the same scheduling state: only its
        // eventual completion wakes the owner, regardless of elapsed time.
        for _ in 0..<100 { clock.fireReadyTimers() }
        XCTAssertEqual(services, 1)
        callbacks.wake?(callbacks.context)
        XCTAssertTrue(queue.runNext())
        XCTAssertEqual(services, 2)
        XCTAssertFalse(work.hasScheduledService)
        XCTAssertTrue(clock.timers.isEmpty)
    }

    func testCompletionDuringServiceCannotLoseTheNextWake() {
        let queue = ManualQueue()
        let clock = ManualClock()
        var services = 0
        var wake: EVBackgroundWorkWake?
        let work = EVBackgroundWork(enqueue: queue.enqueue, makeDeadline: clock.makeDeadline) {
            services += 1
            if services == 1 { wake?.request() }
            return self.status()
        }
        wake = work.wake
        work.wake.request()
        XCTAssertTrue(queue.runNext())
        XCTAssertEqual(services, 1)
        XCTAssertEqual(queue.count, 1)
        XCTAssertTrue(queue.runNext())
        XCTAssertEqual(services, 2)
        XCTAssertFalse(work.hasScheduledService)
    }

    func testDeadlineFiresOnceAndACompletionCancelsTheOldDeadline() {
        let queue = ManualQueue()
        let clock = ManualClock()
        var services = 0
        let work = EVBackgroundWork(enqueue: queue.enqueue, makeDeadline: clock.makeDeadline) {
            services += 1
            return self.status(deadline: services == 1 ? 250 : services == 2 ? 400 : UInt32.max)
        }
        work.wake.request()
        XCTAssertTrue(queue.runNext())
        XCTAssertEqual(clock.delays, [250])
        let superseded = clock.timers[0]

        work.wake.request()
        XCTAssertTrue(queue.runNext())
        XCTAssertFalse(superseded.isValid)
        XCTAssertEqual(clock.delays, [250, 400])
        clock.fireReadyTimers()
        XCTAssertEqual(queue.count, 1)
        XCTAssertTrue(queue.runNext())
        XCTAssertEqual(services, 3)
        XCTAssertFalse(work.hasScheduledService)
        clock.fireReadyTimers()
        XCTAssertFalse(queue.runNext())
        XCTAssertEqual(services, 3)
    }

    func testMoreReadyYieldsToTheQueueWithoutStartingATimer() {
        let queue = ManualQueue()
        let clock = ManualClock()
        var services = 0
        let work = EVBackgroundWork(enqueue: queue.enqueue, makeDeadline: clock.makeDeadline) {
            services += 1
            return self.status(moreReady: services == 1)
        }
        work.wake.request()
        XCTAssertTrue(queue.runNext())
        XCTAssertEqual(services, 1)
        XCTAssertEqual(queue.count, 1)
        XCTAssertTrue(clock.timers.isEmpty)
        XCTAssertTrue(queue.runNext())
        XCTAssertEqual(services, 2)
        XCTAssertFalse(work.hasScheduledService)
    }

    func testCancellationDiscardsQueuedCompletionAndDeadline() {
        let queue = ManualQueue()
        let clock = ManualClock()
        var services = 0
        let work = EVBackgroundWork(enqueue: queue.enqueue, makeDeadline: clock.makeDeadline) {
            services += 1
            return self.status(deadline: 250)
        }
        work.wake.request()
        XCTAssertTrue(queue.runNext())
        work.wake.request()
        work.cancel()
        XCTAssertTrue(queue.runNext())
        XCTAssertFalse(clock.timers[0].isValid)
        work.wake.request()
        clock.fireReadyTimers()
        XCTAssertEqual(services, 1)
        XCTAssertFalse(work.hasScheduledService)
        XCTAssertFalse(queue.runNext())
    }

    func testWorkerCallbackOnlyEnqueuesMainActorService() async {
        let queue = ManualQueue()
        var services = 0
        let work = EVBackgroundWork(enqueue: queue.enqueue) {
            XCTAssertTrue(Thread.isMainThread)
            services += 1
            return self.status()
        }
        let wake = work.wake
        await withCheckedContinuation { continuation in
            DispatchQueue.global().async {
                let callbacks = wake.callbacks
                callbacks.wake?(callbacks.context)
                continuation.resume()
            }
        }
        XCTAssertEqual(services, 0)
        XCTAssertEqual(queue.count, 1)
        XCTAssertTrue(queue.runNext())
        XCTAssertEqual(services, 1)
    }

    func testReloadRebindsTheTransferredSchedulerAndRejectsOldQueuedWork() async throws {
        let backend = makeBackend()
        let oldWork = try XCTUnwrap(backend.backgroundWork)
        oldWork.wake.request()
        let oldServiceCount = oldWork.serviceCount
        try backend.readUntitled(source: Data("replacement".utf8), typeName: EVDocument.plainTextType)
        let replacementWork = try XCTUnwrap(backend.backgroundWork)
        XCTAssertFalse(oldWork === replacementWork)
        XCTAssertFalse(oldWork.hasScheduledService)
        let actualService = replacementWork.service
        var replacementServices = 0
        replacementWork.service = {
            replacementServices += 1
            return actualService()
        }
        oldWork.wake.request()
        replacementWork.wake.request()
        await drainMainQueue()
        XCTAssertEqual(oldWork.serviceCount, oldServiceCount)
        XCTAssertGreaterThan(replacementServices, 0)
        XCTAssertEqual(try backend.formattedText(), "replacement")
        XCTAssertFalse(replacementWork.hasScheduledService)
    }

    func testRetainedCallbackAfterDocumentCloseCannotKeepOwnerAliveOrQueueWork() throws {
        var backend: EVCoreDocumentBackend? = makeBackend()
        weak var weakBackend = backend
        weak var weakWork = backend?.backgroundWork
        weak var weakWake = backend?.backgroundWork?.wake
        let callbacks = try XCTUnwrap(backend?.backgroundWork?.wake.callbacks)
        callbacks.retain?(callbacks.context)
        backend = nil
        XCTAssertNil(weakBackend)
        XCTAssertNil(weakWork)
        XCTAssertNotNil(weakWake)
        callbacks.wake?(callbacks.context)
        XCTAssertEqual(weakWake?.hasQueuedDelivery, false)
        callbacks.release?(callbacks.context)
        XCTAssertNil(weakWake)
    }

    private func status(deadline: UInt32 = .max, moreReady: Bool = false) -> ViemBackgroundWorkStatus {
        var result = ViemBackgroundWorkStatus()
        result.next_deadline_ms = deadline
        result.more_ready = moreReady ? 1 : 0
        return result
    }

    private func makeBackend() -> EVCoreDocumentBackend {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-background-work-\(UUID().uuidString)")
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        return EVCoreDocumentBackend(configuration: EVConfigurationStore(directory: directory, legacyDefaults: nil))
    }

    private func drainMainQueue() async {
        await withCheckedContinuation { continuation in
            DispatchQueue.main.async { continuation.resume() }
        }
    }

    private final class ManualQueue: @unchecked Sendable {
        private let lock = NSLock()
        private var deliveries: [EVBackgroundWorkWake.Delivery] = []
        var count: Int { lock.withLock { deliveries.count } }

        func enqueue(_ delivery: @escaping EVBackgroundWorkWake.Delivery) {
            lock.withLock { deliveries.append(delivery) }
        }

        @MainActor func runNext() -> Bool {
            let next = lock.withLock { deliveries.isEmpty ? nil : deliveries.removeFirst() }
            guard let next else { return false }
            next()
            return true
        }
    }

    @MainActor private final class ManualClock {
        var timers: [Timer] = []
        var delays: [UInt32] = []

        func makeDeadline(_ milliseconds: UInt32, _ deliver: @escaping EVBackgroundWorkWake.Delivery) -> Timer {
            let timer = Timer(timeInterval: Double(milliseconds) / 1000, repeats: false) { _ in
                MainActor.assumeIsolated { deliver() }
            }
            timers.append(timer)
            delays.append(milliseconds)
            return timer
        }

        func fireReadyTimers() {
            for timer in timers where timer.isValid { timer.fire() }
        }
    }
}
