import AppKit
import CViemCore
import ImageIO
import UniformTypeIdentifiers
import ViemAppShell
import XCTest
@testable import ViemCoreTextProvider
@testable import ViemEditor

/// Opt-in interactive measurements with real Core Text shaping, drawing and
/// main run-loop servicing between frames. Run optimized, as described in
/// docs/performance.md:
/// `VIEM_RUST_PROFILE=release VIEM_PRELAYOUT_BENCHMARK=1 scripts/test-mac.sh -c release -Xswiftc -enable-testing --filter EVPrelayoutBenchmarkTests`.
/// Reports are printed as `PRELAYOUT_BENCH` lines; assertions live in the
/// ordinary regression tests.
@MainActor
final class EVPrelayoutBenchmarkTests: XCTestCase {
    override func setUpWithError() throws {
        try XCTSkipUnless(ProcessInfo.processInfo.environment["VIEM_PRELAYOUT_BENCHMARK"] == "1",
                          "Set VIEM_PRELAYOUT_BENCHMARK=1 to collect interactive measurements")
    }

    func testMarkdownScrolling() async throws {
        let images = try EVPrelayoutFixtures.images()
        let source = EVPrelayoutFixtures.markdown(images: images, targetBytes: 95_000)
        for mode in EVPrelayoutBenchmark.Mode.allCases {
            try await EVPrelayoutBenchmark(label: "markdown-95k", source: source, typeName: EVDocument.markdownType,
                filename: nil, mode: mode, test: self).run()
        }
    }

    /// Attribute remaining input-thread shaping to tables, figures or code.
    func testMarkdownComponents() async throws {
        let images = try EVPrelayoutFixtures.images()
        for (label, source) in [
            ("markdown-95k-no-tables", EVPrelayoutFixtures.markdown(images: images, targetBytes: 95_000, tables: false)),
            ("markdown-95k-no-figures", EVPrelayoutFixtures.markdown(images: images, targetBytes: 95_000, figures: false)),
            ("markdown-95k-no-code", EVPrelayoutFixtures.markdown(images: images, targetBytes: 95_000, code: false)),
        ] {
            try await EVPrelayoutBenchmark(label: label, source: source, typeName: EVDocument.markdownType,
                filename: nil, mode: .idle, test: self).run()
        }
    }

    func testCodeScrolling() async throws {
        let source = try EVPrelayoutFixtures.rust(targetBytes: 95_000)
        for mode in EVPrelayoutBenchmark.Mode.allCases {
            try await EVPrelayoutBenchmark(label: "rust-95k", source: source, typeName: EVDocument.plainTextType,
                filename: "bench.rs", mode: mode, test: self).run()
        }
    }

    func testLargeMarkdownScrolling() async throws {
        let images = try EVPrelayoutFixtures.images()
        let source = EVPrelayoutFixtures.markdown(images: images, targetBytes: 1_000_000)
        for mode in [EVPrelayoutBenchmark.Mode.immediate, .idle] {
            try await EVPrelayoutBenchmark(label: "markdown-1m", source: source, typeName: EVDocument.markdownType,
                filename: nil, mode: mode, test: self, maximumSteps: 900).run()
        }
    }
}

/// Measures main-thread work between run-loop waits. Each busy span is the
/// latency an input event arriving at its start would experience. Spans that
/// contain a synthetic input are reported separately from background work.
@MainActor
final class EVMainThreadBusyMonitor {
    private var observer: CFRunLoopObserver?
    private var awake: CFAbsoluteTime?
    private var sawInput = false
    private(set) var background: [Double] = []
    private(set) var input: [Double] = []
    /// Background spans over 10 ms: offset from start and duration, in ms.
    private(set) var long: [(Double, Double)] = []
    private var started: CFAbsoluteTime = 0

    func start() {
        background = []; input = []; long = []; sawInput = false
        awake = CFAbsoluteTimeGetCurrent(); started = awake!
        let observer = CFRunLoopObserverCreateWithHandler(nil,
            CFRunLoopActivity.beforeWaiting.rawValue | CFRunLoopActivity.afterWaiting.rawValue, true, 0) { [weak self] _, activity in
            MainActor.assumeIsolated {
                guard let self else { return }
                let now = CFAbsoluteTimeGetCurrent()
                if activity == .afterWaiting { self.awake = now; return }
                if let awake = self.awake {
                    let span = (now - awake) * 1000
                    if self.sawInput { self.input.append(span) } else {
                        self.background.append(span)
                        if span > 10 { self.long.append(((awake - self.started) * 1000, span)) }
                    }
                }
                self.awake = nil; self.sawInput = false
            }
        }
        CFRunLoopAddObserver(CFRunLoopGetMain(), observer, .commonModes)
        self.observer = observer
    }

    func noteInput() { sawInput = true }

    func stop() {
        if let observer { CFRunLoopRemoveObserver(CFRunLoopGetMain(), observer, .commonModes) }
        observer = nil
    }
}

struct EVLatencySummary: CustomStringConvertible {
    let samples: [Double]
    init(_ samples: [Double]) { self.samples = samples.sorted() }
    func percentile(_ p: Double) -> Double {
        guard !samples.isEmpty else { return 0 }
        return samples[min(samples.count - 1, Int(Double(samples.count - 1) * p))]
    }
    var maximum: Double { samples.last ?? 0 }
    var total: Double { samples.reduce(0, +) }
    func over(_ limit: Double) -> Int { samples.filter { $0 > limit }.count }
    var description: String {
        String(format: "n=%d p50=%.2f p95=%.2f p99=%.2f max=%.2f >8ms=%d >16ms=%d",
               samples.count, percentile(0.5), percentile(0.95), percentile(0.99), maximum, over(8), over(16))
    }
}

@MainActor
struct EVPrelayoutBenchmark {
    enum Mode: String, CaseIterable {
        /// Scroll as soon as the first frame is drawn, competing with idle work.
        case immediate
        /// Wait for background preparation, then scroll.
        case idle
        /// Background preparation disabled entirely.
        case disabled
    }
    let label: String
    let source: String
    let typeName: String
    let filename: String?
    let mode: Mode
    let test: XCTestCase
    var maximumSteps = 2_000
    var frame: Double = 1.0 / 60.0
    var step: CGFloat = 120

    func run() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-prelayout-bench-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: directory) }
        var checkout = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { checkout.deleteLastPathComponent() }
        let resources = checkout.appendingPathComponent(".build/Viem.app/Contents/Resources")
        let backend = EVCoreDocumentBackend(configuration:
            EVConfigurationStore(directory: directory, legacyDefaults: nil, bundleResourceURL: resources))
        let opened = CFAbsoluteTimeGetCurrent()
        if let filename {
            try backend.read(source: Data(source.utf8), typeName: typeName, origin: .file(path: filename, automaticCode: true))
        } else {
            try backend.readUntitled(source: Data(source.utf8), typeName: typeName)
        }
        let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
        surface.loadViewIfNeeded()
        let window = EVTestFocusWindow(contentRect: NSRect(x: 0, y: 0, width: 900, height: 700),
                                       styleMask: [.titled], backing: .buffered, defer: false)
        defer { withExtendedLifetime(window) {} }
        window.contentViewController = surface
        let session = try XCTUnwrap(surface.session)
        EVBenchHooks.setEnabled(mode != .disabled, session: session)
        surface.view.frame = NSRect(x: 0, y: 0, width: 900, height: 700)
        surface.viewDidLayout()
        surface.refreshPresentation()
        let view = surface.editorView
        // A Retina backing store: two device pixels per point.
        let context = try XCTUnwrap(CGContext(data: nil, width: Int(view.bounds.width) * 2, height: Int(view.bounds.height) * 2,
            bitsPerComponent: 8, bytesPerRow: Int(view.bounds.width) * 8,
            space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
        context.scaleBy(x: 2, y: 2)
        draw(view, context, [view.bounds])
        let firstFrame = (CFAbsoluteTimeGetCurrent() - opened) * 1000

        let monitor = EVMainThreadBusyMonitor()
        var idleMilliseconds = 0.0
        var idleSpans: [Double] = []
        if mode == .idle {
            monitor.start()
            let start = CFAbsoluteTimeGetCurrent()
            var quiet = 0
            while CFAbsoluteTimeGetCurrent() - start < 30 {
                try await Task.sleep(nanoseconds: UInt64(frame * 1e9))
                if EVBenchHooks.isIdle(session: session, backend: backend) { quiet += 1 } else { quiet = 0 }
                // Includes the deferred extent publication after preparation.
                if quiet >= 15 { break }
            }
            idleMilliseconds = (CFAbsoluteTimeGetCurrent() - start) * 1000
            monitor.stop()
            idleSpans = monitor.background
            print("PRELAYOUT_BENCH \(label) mode=\(mode.rawValue) long_preparation_spans " + monitor.long.map { String(format: "@%.0f:%.1f", $0.0, $0.1) }.joined(separator: " "))
        }

        let mainShapesBefore = session.provider.shapeBatchCallCount
        var inputTimes: [Double] = []
        var coreTimes: [Double] = []
        var drawTimes: [Double] = []
        var extentChanges = 0
        var extentDelta: Double = 0
        var lastMaximum = Double(surface.viewportState.maximum_top)
        monitor.start()
        var steps = 0
        while steps < maximumSteps {
            let before = surface.viewportState
            monitor.noteInput()
            let start = CFAbsoluteTimeGetCurrent()
            view.scrollWheel(with: EVBenchWheelEvent(deltaY: -step))
            let drawStart = CFAbsoluteTimeGetCurrent()
            draw(view, context, view.presentationDamageRects.isEmpty ? [view.bounds] : view.presentationDamageRects)
            let elapsed = CFAbsoluteTimeGetCurrent() - start
            inputTimes.append(elapsed * 1000)
            coreTimes.append((drawStart - start) * 1000)
            drawTimes.append((CFAbsoluteTimeGetCurrent() - drawStart) * 1000)
            let after = surface.viewportState
            if abs(Double(after.maximum_top) - lastMaximum) > 0.5 {
                extentChanges += 1; extentDelta += abs(Double(after.maximum_top) - lastMaximum)
            }
            lastMaximum = Double(after.maximum_top)
            steps += 1
            try await Task.sleep(nanoseconds: UInt64(max(0.001, frame - elapsed) * 1e9))
            if after.top >= after.maximum_top - 0.5 && before.top == after.top { break }
        }
        monitor.stop()
        let backgroundSpans = monitor.background
        let mainShapes = session.provider.shapeBatchCallCount - mainShapesBefore

        // Keystrokes in the middle of the document while background work runs.
        surface.performInput { _ = try session.sendTestKeys("50%") }
        try await Task.sleep(nanoseconds: UInt64(frame * 1e9))
        surface.performInput { _ = try session.sendTestKeys("o") }
        var typing: [Double] = []
        var typingCore: [Double] = []
        monitor.start()
        for character in "the quick brown fox jumps" {
            monitor.noteInput()
            let start = CFAbsoluteTimeGetCurrent()
            surface.performInput { _ = try session.sendText(String(character)) }
            typingCore.append((CFAbsoluteTimeGetCurrent() - start) * 1000)
            draw(view, context, view.presentationDamageRects.isEmpty ? [view.bounds] : view.presentationDamageRects)
            typing.append((CFAbsoluteTimeGetCurrent() - start) * 1000)
            try await Task.sleep(nanoseconds: UInt64(frame * 1e9))
        }
        monitor.stop()
        let typingBackground = monitor.background
        print(String(format: "PRELAYOUT_BENCH %@ mode=%@ bytes=%d first_frame_ms=%.1f idle_ms=%.0f steps=%d main_shapes=%llu extent_changes=%d extent_delta=%.0f",
                     label, mode.rawValue, source.utf8.count, firstFrame, idleMilliseconds, steps, mainShapes, extentChanges, extentDelta))
        print("PRELAYOUT_BENCH \(label) mode=\(mode.rawValue) scroll_input_ms \(EVLatencySummary(inputTimes))")
        print("PRELAYOUT_BENCH \(label) mode=\(mode.rawValue) scroll_core_ms \(EVLatencySummary(coreTimes))")
        print("PRELAYOUT_BENCH \(label) mode=\(mode.rawValue) scroll_draw_ms \(EVLatencySummary(drawTimes))")
        print("PRELAYOUT_BENCH \(label) mode=\(mode.rawValue) scroll_busy_ms \(EVLatencySummary(backgroundSpans))")
        if mode == .idle { print("PRELAYOUT_BENCH \(label) mode=\(mode.rawValue) preparation_busy_ms \(EVLatencySummary(idleSpans)) total=\(String(format: "%.0f", EVLatencySummary(idleSpans).total))") }
        print("PRELAYOUT_BENCH \(label) mode=\(mode.rawValue) typing_ms \(EVLatencySummary(typing))")
        print("PRELAYOUT_BENCH \(label) mode=\(mode.rawValue) typing_core_ms \(EVLatencySummary(typingCore))")
        print("PRELAYOUT_BENCH \(label) mode=\(mode.rawValue) typing_busy_ms \(EVLatencySummary(typingBackground))")
        print("PRELAYOUT_BENCH \(label) mode=\(mode.rawValue) \(EVBenchHooks.summary(session: session))")
        surface.detachFromCore()
        try await Task.sleep(nanoseconds: 50_000_000)
    }

    private func draw(_ view: EVEditorView, _ context: CGContext, _ damage: [NSRect]) {
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = NSGraphicsContext(cgContext: context, flipped: true)
        for rect in damage { view.draw(rect) }
        NSGraphicsContext.restoreGraphicsState()
    }
}

/// Deterministic documents resembling ordinary writing: prose, lists, quotes,
/// tables, fenced code and local images.
enum EVPrelayoutFixtures {
    private static let words = ("the editor keeps a modal keyboard first model while presenting proportional typography "
        + "with mixed styles soft wrapping and responsive layout for long documents written by people who care about "
        + "careful prose tables images code and quotations flow between paragraphs headings lists and figures").split(separator: " ")

    static func images() throws -> [URL] {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("viem-prelayout-images", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let sizes = [(640, 360), (1200, 800), (300, 300), (900, 1200), (1600, 500), (480, 640), (1024, 768), (200, 120)]
        return try sizes.enumerated().map { index, size in
            let url = directory.appendingPathComponent("figure-\(index)-\(size.0)x\(size.1).png")
            if FileManager.default.fileExists(atPath: url.path) { return url }
            let context = try XCTUnwrap(CGContext(data: nil, width: size.0, height: size.1, bitsPerComponent: 8, bytesPerRow: 0,
                space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
            for band in 0..<16 {
                context.setFillColor(CGColor(red: CGFloat(band) / 16, green: CGFloat(index) / 8, blue: 0.6, alpha: 1))
                context.fill(CGRect(x: band * size.0 / 16, y: 0, width: size.0 / 16 + 1, height: size.1))
            }
            let image = try XCTUnwrap(context.makeImage())
            let destination = try XCTUnwrap(CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil))
            CGImageDestinationAddImage(destination, image, nil)
            XCTAssertTrue(CGImageDestinationFinalize(destination))
            return url
        }
    }

    static func markdown(images: [URL], targetBytes: Int, tables: Bool = true, figures: Bool = true, code: Bool = true) -> String {
        var random: UInt64 = 0x9E37_79B9_7F4A_7C15
        func next(_ bound: Int) -> Int {
            random ^= random << 13; random ^= random >> 7; random ^= random << 17
            return Int(random % UInt64(bound))
        }
        func sentence(_ count: Int) -> String {
            var parts: [String] = []
            for index in 0..<count {
                var word = String(words[next(words.count)])
                switch next(23) {
                case 0: word = "**\(word)**"
                case 1: word = "*\(word)*"
                case 2: word = "`\(word)`"
                case 3: word = "[\(word)](https://example.com/\(word))"
                default: break
                }
                parts.append(index == 0 ? word.prefix(1).uppercased() + word.dropFirst() : word)
            }
            return parts.joined(separator: " ") + "."
        }
        func paragraph() -> String { (0..<(3 + next(4))).map { _ in sentence(8 + next(14)) }.joined(separator: " ") }
        let languages = ["rust", "python", "swift", "json"]
        var output = "# Prelayout benchmark document\n\n"
        var section = 0
        while output.utf8.count < targetBytes {
            section += 1
            output += "## Section \(section): \(sentence(4).dropLast())\n\n"
            for _ in 0..<(2 + next(3)) { output += paragraph() + "\n\n" }
            if figures, section % 2 == 0 {
                output += "![Figure \(section)](\(images[section % images.count].path))\n\n"
            }
            output += (0..<(4 + next(4))).map { _ in "- " + sentence(4 + next(10)) }.joined(separator: "\n") + "\n\n"
            if tables, section % 3 == 0 {
                let columns = 3 + next(4)
                output += "| " + (0..<columns).map { "Column \($0)" }.joined(separator: " | ") + " |\n"
                output += "|" + String(repeating: " --- |", count: columns) + "\n"
                for _ in 0..<(6 + next(14)) {
                    output += "| " + (0..<columns).map { _ in sentence(1 + next(next(3) == 0 ? 12 : 3)).dropLast() }.joined(separator: " | ") + " |\n"
                }
                output += "\n"
            }
            if section % 4 == 1 {
                output += "> " + paragraph() + "\n\n"
            }
            if code, section % 3 == 1 {
                let language = languages[section % languages.count]
                output += "```\(language)\n"
                for line in 0..<(8 + next(16)) {
                    switch language {
                    case "rust": output += "    let value_\(line) = compute(\"\(words[next(words.count)])\", \(line)); // note\n"
                    case "python": output += "    value_\(line) = compute('\(words[next(words.count)])', \(line))  # note\n"
                    case "swift": output += "    let value\(line) = compute(\"\(words[next(words.count)])\", \(line)) // note\n"
                    default: output += "  \"key_\(line)\": [\(line), \"\(words[next(words.count)])\", true],\n"
                    }
                }
                output += "```\n\n"
            }
            if section % 5 == 0 {
                output += (1...(3 + next(4))).map { "\($0). " + sentence(5 + next(8)) }.joined(separator: "\n") + "\n\n"
            }
        }
        return output
    }

    static func rust(targetBytes: Int) throws -> String {
        var checkout = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { checkout.deleteLastPathComponent() }
        let sources = ["src/core/layout/tables.rs", "src/core/layout/height_index.rs", "src/core/coordinator/search.rs",
                       "src/core/layout/unwrapped.rs"]
        var output = ""
        for path in sources where output.utf8.count < targetBytes {
            output += try String(contentsOf: checkout.appendingPathComponent(path), encoding: .utf8)
        }
        var bytes = Array(output.utf8.prefix(targetBytes))
        while let last = bytes.last, last != UInt8(ascii: "\n") { bytes.removeLast() }
        return String(decoding: bytes, as: UTF8.self)
    }
}

final class EVBenchWheelEvent: NSEvent {
    private let verticalDelta: CGFloat
    init(deltaY: CGFloat) { verticalDelta = deltaY; super.init() }
    @available(*, unavailable)
    required init?(coder _: NSCoder) { fatalError("init(coder:) is unavailable") }
    override var type: NSEvent.EventType { .scrollWheel }
    override var scrollingDeltaX: CGFloat { 0 }
    override var scrollingDeltaY: CGFloat { verticalDelta }
    override var isDirectionInvertedFromDevice: Bool { true }
    override var hasPreciseScrollingDeltas: Bool { true }
    override var phase: NSEvent.Phase { [] }
    override var momentumPhase: NSEvent.Phase { [] }
}

/// Scheduler access for the benchmark; kept separate so the measurements can
/// be compared across scheduler implementations.
@MainActor
enum EVBenchHooks {
    static func setEnabled(_ enabled: Bool, session: EVCoreViewSession) {
        session.backgroundLayout.isEnabled = enabled
        session.tableWidthRefinement.isEnabled = enabled
    }
    static func isIdle(session: EVCoreViewSession, backend: EVCoreDocumentBackend) -> Bool {
        session.backgroundLayout.isIdle && session.tableWidthRefinement.isIdle && session.provider.pendingImageLoadCount == 0
    }
    static func summary(session: EVCoreViewSession) -> String {
        let layout = session.backgroundLayout, tables = session.tableWidthRefinement
        return "prelayout started=\(layout.started) installed=\(layout.installed) discarded=\(layout.discarded) "
            + String(format: "worker_ms=%.0f longest_chunk_ms=%.1f ", layout.workerSeconds * 1000, layout.longestWorkerSeconds * 1000)
            + "tables started=\(tables.started) installed=\(tables.installed) discarded=\(tables.discarded) "
            + "image_loads=\(session.provider.localImageLoadCountForTesting) "
            + "image_probes=\(session.provider.imageDimensionProbeCountForTesting) "
            + "image_rescales=\(session.provider.imageScaledRenderCountForTesting)"
    }
}
