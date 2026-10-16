import CViemCore
import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers
import ViemCoreTextProvider

// Profiles real Core Text shaping through the portable core outside the
// hardened xctest host, where `sample` cannot collect stacks. It opens the
// same deterministic Markdown fixture as EVPrelayoutBenchmarkTests, prepares
// the complete document as background pre-layout does (computed on this
// thread), then types in the middle of the document.
//
//   VIEM_RUST_PROFILE=release swift build -c release --product viem-shape-bench
//   .build/release/viem-shape-bench [--iterations N] [--bytes N] [--typing N]
//
// Attach `sample <pid>` while it runs; use --iterations to lengthen the run.

struct Options {
    var iterations = 1
    var bytes = 95_000
    var typing = 25
    var figures = true
    var handleDigest = true
    /// Documents and views per document for a memory measurement.
    var memory: (documents: Int, views: Int)?

    init(_ arguments: [String]) {
        var index = 1
        func value() -> Int {
            index += 1
            guard index < arguments.count, let parsed = Int(arguments[index]) else {
                FileHandle.standardError.write(Data("missing numeric value for \(arguments[index - 1])\n".utf8))
                exit(64)
            }
            return parsed
        }
        while index < arguments.count {
            switch arguments[index] {
            case "--iterations": iterations = value()
            case "--bytes": bytes = value()
            case "--typing": typing = value()
            case "--no-figures": figures = false
            case "--no-handle-digest": handleDigest = false
            case "--memory":
                index += 1
                let parts = index < arguments.count ? arguments[index].split(separator: ",").compactMap { Int($0) } : []
                guard parts.count == 2 else {
                    FileHandle.standardError.write(Data("--memory takes DOCUMENTS,VIEWS\n".utf8))
                    exit(64)
                }
                memory = (parts[0], parts[1])
            default:
                FileHandle.standardError.write(Data("unknown argument \(arguments[index])\n".utf8))
                exit(64)
            }
            index += 1
        }
    }
}

/// Forwards to the real provider while counting callbacks and requests.
nonisolated(unsafe) private var forwardedShape: ViemShapeBatchCallback?
nonisolated(unsafe) private var shapeCalls = 0
nonisolated(unsafe) private var shapeRequests = 0
nonisolated(unsafe) private var shapeSeconds = 0.0

private func countingShape(
    _ context: UnsafeMutableRawPointer?, _ requests: UnsafePointer<ViemShapeRequestV1>?, _ count: UInt64,
    _ responses: UnsafeMutablePointer<ViemShapeResponseV1>?, _ capacity: UInt64
) -> UInt32 {
    shapeCalls += 1
    shapeRequests += Int(count)
    let start = CFAbsoluteTimeGetCurrent()
    let status = forwardedShape!(context, requests, count, responses, capacity)
    shapeSeconds += CFAbsoluteTimeGetCurrent() - start
    if status == UInt32(VIEM_STATUS_OK), let responses {
        for index in 0..<Int(count) { digest.add(responses[index]) }
    }
    return status
}

/// Order-sensitive digest of every response field, so provider changes can be
/// checked for identical output. Render handles cover their glyph signatures.
struct ResponseDigest {
    private(set) var value: UInt64 = 0xcbf2_9ce4_8422_2325
    mutating func add<T>(_ scalar: T) {
        withUnsafeBytes(of: scalar) { bytes in
            for byte in bytes { value = (value ^ UInt64(byte)) &* 0x0000_0100_0000_01b3 }
        }
    }
    mutating func add(_ slice: ViemUtf8Slice) {
        add(slice.length)
        if let data = slice.data { for byte in UnsafeBufferPointer(start: data, count: Int(slice.length)) { add(byte) } }
    }
    mutating func add(_ response: ViemShapeResponseV1) {
        add(response.text_start); add(response.text_end); add(response.cluster_count)
        add(response.default_metrics.ascent); add(response.default_metrics.descent); add(response.default_metrics.leading)
        for index in 0..<Int(response.cluster_count) {
            let cluster = response.clusters![index]
            add(cluster.text_start); add(cluster.text_end); add(cluster.advance)
            add(cluster.metrics.ascent); add(cluster.metrics.descent); add(cluster.metrics.leading)
            for bounds in [cluster.typographic_bounds, cluster.ink_bounds] {
                add(bounds.x); add(bounds.y); add(bounds.width); add(bounds.height)
            }
            add(cluster.bidi_level); add(cluster.has_render_run); add(cluster.fallback_font)
            add(cluster.caret_stop_count)
            for caret in 0..<Int(cluster.caret_stop_count) {
                let stop = cluster.caret_stops![caret]
                add(stop.text_offset); add(stop.inline_offset); add(stop.affinity)
            }
            // Inline-image signatures include a per-load identity.
            if cluster.has_render_run != 0, digestsHandles { add(cluster.render_run.identifier) }
        }
        add(response.visual_order_count)
        for index in 0..<Int(response.visual_order_count) { add(response.visual_order![index]) }
        add(response.diagnostic_count)
        for index in 0..<Int(response.diagnostic_count) {
            let diagnostic = response.diagnostics![index]
            add(diagnostic.text_start); add(diagnostic.text_end); add(diagnostic.message)
        }
    }
}
nonisolated(unsafe) private var digest = ResponseDigest()
nonisolated(unsafe) private var digestsHandles = true

private func counted(_ provider: CoreTextMeasurementProvider) -> ViemTextMeasurementProviderV1 {
    var table = provider.makeProviderTable()
    forwardedShape = table.shape_batch
    table.shape_batch = countingShape
    return table
}

private func check(_ status: UInt32, _ operation: String) {
    guard status != UInt32(VIEM_STATUS_OK) else { return }
    FileHandle.standardError.write(Data("\(operation) failed with status \(status)\n".utf8))
    exit(1)
}

private func milliseconds(_ seconds: Double) -> String { String(format: "%.1f", seconds * 1000) }

private func summary(_ samples: [Double]) -> String {
    let sorted = samples.sorted()
    guard !sorted.isEmpty else { return "n=0" }
    func percentile(_ p: Double) -> Double { sorted[min(sorted.count - 1, Int(Double(sorted.count - 1) * p))] * 1000 }
    return String(format: "n=%d p50=%.2f p95=%.2f max=%.2f total=%.1f",
                  sorted.count, percentile(0.5), percentile(0.95), sorted.last! * 1000, sorted.reduce(0, +) * 1000)
}

private func resetCounters() { shapeCalls = 0; shapeRequests = 0; shapeSeconds = 0 }

private func sendKey(_ core: UInt64, _ view: UInt64, _ scalar: Unicode.Scalar, kind: UInt32 = UInt32(VIEM_KEY_CHARACTER)) {
    var input = ViemKeyInputV1()
    input.struct_size = UInt32(MemoryLayout<ViemKeyInputV1>.size)
    input.kind = kind
    input.codepoint = kind == UInt32(VIEM_KEY_CHARACTER) ? scalar.value : 0
    var context = ViemCommandTurnContextV2()
    context.struct_size = UInt32(MemoryLayout<ViemCommandTurnContextV2>.size)
    var outcome = ViemCoreOutcomeV1()
    outcome.struct_size = UInt32(MemoryLayout<ViemCoreOutcomeV1>.size)
    var effects: ViemEffectBatchHandle = 0
    check(viem_core_view_send_key_with_host_context_v2(core, view, &input, &context, &outcome, &effects), "send key")
    if effects != 0 { _ = viem_effect_batch_release(effects) }
}

private func sendText(_ core: UInt64, _ view: UInt64, _ text: String) {
    var context = ViemCommandTurnContextV2()
    context.struct_size = UInt32(MemoryLayout<ViemCommandTurnContextV2>.size)
    var outcome = ViemCoreOutcomeV1()
    outcome.struct_size = UInt32(MemoryLayout<ViemCoreOutcomeV1>.size)
    var effects: ViemEffectBatchHandle = 0
    let bytes = Array(text.utf8)
    let status = bytes.withUnsafeBufferPointer {
        viem_core_view_send_text_with_host_context_v2(core, view, $0.baseAddress, UInt64($0.count), &context, &outcome, &effects)
    }
    check(status, "send text")
    if effects != 0 { _ = viem_effect_batch_release(effects) }
}

/// Physical footprint, the measure Activity Monitor reports as Memory.
private func footprintMiB() -> Double {
    var info = task_vm_info_data_t()
    var count = mach_msg_type_number_t(MemoryLayout<task_vm_info_data_t>.size / MemoryLayout<natural_t>.size)
    let status = withUnsafeMutablePointer(to: &info) {
        $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
            task_info(mach_task_self_, task_flavor_t(TASK_VM_INFO), $0, &count)
        }
    }
    return status == KERN_SUCCESS ? Double(info.phys_footprint) / 1_048_576 : -1
}

/// Bytes currently allocated in every malloc zone, excluding free pages the
/// allocator retains; less sensitive to allocation churn than the footprint.
private func liveMiB() -> Double {
    var stats = malloc_statistics_t()
    malloc_zone_statistics(nil, &stats)
    return Double(stats.size_in_use) / 1_048_576
}

/// A view laid out completely, as background pre-rendering leaves it.
private func addPrerenderedView(_ core: UInt64, provider: CoreTextMeasurementProvider) -> UInt64 {
    var table = counted(provider)
    var viewOptions = ViemViewOptionsV1()
    viewOptions.struct_size = UInt32(MemoryLayout<ViemViewOptionsV1>.size)
    viewOptions.execution_context = UInt32(VIEM_LAYOUT_EXECUTION_FRONTEND_MAIN)
    viewOptions.width = 900; viewOptions.height = 700
    viewOptions.padding_top = 20; viewOptions.padding_left = 40; viewOptions.padding_bottom = 20; viewOptions.padding_right = 40
    var view: UInt64 = 0
    var outcome = ViemCoreOutcomeV1()
    outcome.struct_size = UInt32(MemoryLayout<ViemCoreOutcomeV1>.size)
    check(viem_core_view_add(core, &viewOptions, &table, &view, &outcome), "add view")
    while true {
        var request: UInt64 = 0
        check(viem_core_view_prepare_prelayout(core, view, 1, &request), "prepare pre-layout")
        guard request != 0 else { break }
        let worker = provider.makeWorkerProvider()
        var workerTable = counted(worker)
        var result: UInt64 = 0
        check(viem_layout_work_compute(request, &workerTable, &result), "compute pre-layout")
        var installed: UInt8 = 0
        if result != 0 { check(viem_core_view_install_prelayout(core, view, 1, result, &installed), "install pre-layout") }
        _ = viem_layout_work_release(request)
        withExtendedLifetime(worker) {}
    }
    return view
}

let options = Options(CommandLine.arguments)
digestsHandles = options.handleDigest
let images = options.figures ? try ShapeBenchFixtures.images() : []
let source = Array(ShapeBenchFixtures.markdown(images: images, targetBytes: options.bytes, figures: options.figures).utf8)
print("source_bytes=\(source.count)")

if let memory = options.memory {
    // Main-queue blocks release worker render resources; drain them so
    // measurements count only retained memory.
    func settle() { for _ in 0..<5 { RunLoop.main.run(until: Date(timeIntervalSinceNow: 0.05)) } }
    settle()
    let baseline = footprintMiB()
    let liveBaseline = liveMiB()
    print(String(format: "memory baseline_mib=%.1f live_mib=%.1f", baseline, liveBaseline))
    var opened: [(core: UInt64, views: [(UInt64, CoreTextMeasurementProvider)])] = []
    for documentIndex in 0..<memory.documents {
        var documentOptions = ViemDocumentOptions()
        documentOptions.struct_size = UInt32(MemoryLayout<ViemDocumentOptions>.size)
        documentOptions.encoding = UInt32(VIEM_ENCODING_UTF8)
        documentOptions.format = UInt32(VIEM_FORMAT_MARKDOWN)
        var core: UInt64 = 0, revision: UInt64 = 0
        check(source.withUnsafeBufferPointer { viem_core_create($0.baseAddress, UInt64($0.count), &documentOptions, &core, &revision) },
              "create core")
        settle()
        let created = footprintMiB()
        var views: [(UInt64, CoreTextMeasurementProvider)] = []
        for viewIndex in 0..<memory.views {
            let before = footprintMiB()
            let liveBefore = liveMiB()
            let provider = CoreTextMeasurementProvider()
            views.append((addPrerenderedView(core, provider: provider), provider))
            settle()
            print(String(format: "memory document=%d view=%d view_mib=%.1f view_live_mib=%.1f total_mib=%.1f total_live_mib=%.1f",
                         documentIndex, viewIndex, footprintMiB() - before, liveMiB() - liveBefore,
                         footprintMiB() - baseline, liveMiB() - liveBaseline))
        }
        print(String(format: "memory document=%d document_mib=%.1f", documentIndex, footprintMiB() - created))
        opened.append((core, views))
    }
    print(String(format: "memory all_mib=%.1f", footprintMiB() - baseline))
    for (core, views) in opened {
        for (view, provider) in views {
            check(viem_core_view_remove(core, view), "remove view")
            provider.retireResources()
        }
        check(viem_core_destroy(core), "destroy core")
    }
    settle()
    print(String(format: "memory after_release_mib=%.1f after_release_live_mib=%.1f", footprintMiB() - baseline,
                 liveMiB() - liveBaseline))
    exit(0)
}

for iteration in 0..<options.iterations {
    var documentOptions = ViemDocumentOptions()
    documentOptions.struct_size = UInt32(MemoryLayout<ViemDocumentOptions>.size)
    documentOptions.encoding = UInt32(VIEM_ENCODING_UTF8)
    documentOptions.format = UInt32(VIEM_FORMAT_MARKDOWN)
    var core: UInt64 = 0, revision: UInt64 = 0
    check(source.withUnsafeBufferPointer { viem_core_create($0.baseAddress, UInt64($0.count), &documentOptions, &core, &revision) },
          "create core")

    let provider = CoreTextMeasurementProvider()
    var table = counted(provider)
    var viewOptions = ViemViewOptionsV1()
    viewOptions.struct_size = UInt32(MemoryLayout<ViemViewOptionsV1>.size)
    viewOptions.execution_context = UInt32(VIEM_LAYOUT_EXECUTION_FRONTEND_MAIN)
    viewOptions.width = 900; viewOptions.height = 700
    viewOptions.padding_top = 20; viewOptions.padding_left = 40; viewOptions.padding_bottom = 20; viewOptions.padding_right = 40
    var view: UInt64 = 0
    var outcome = ViemCoreOutcomeV1()
    outcome.struct_size = UInt32(MemoryLayout<ViemCoreOutcomeV1>.size)
    resetCounters()
    var start = CFAbsoluteTimeGetCurrent()
    check(viem_core_view_add(core, &viewOptions, &table, &view, &outcome), "add view")
    print("[\(iteration)] first_layout_ms=\(milliseconds(CFAbsoluteTimeGetCurrent() - start)) shape_calls=\(shapeCalls) requests=\(shapeRequests) shape_ms=\(milliseconds(shapeSeconds))")

    resetCounters()
    var chunks = 0
    var computeSeconds = 0.0
    start = CFAbsoluteTimeGetCurrent()
    while true {
        var request: UInt64 = 0
        check(viem_core_view_prepare_prelayout(core, view, 1, &request), "prepare pre-layout")
        guard request != 0 else { break }
        let worker = provider.makeWorkerProvider()
        var workerTable = counted(worker)
        var result: UInt64 = 0
        let computeStart = CFAbsoluteTimeGetCurrent()
        check(viem_layout_work_compute(request, &workerTable, &result), "compute pre-layout")
        computeSeconds += CFAbsoluteTimeGetCurrent() - computeStart
        var installed: UInt8 = 0
        if result != 0 {
            check(viem_core_view_install_prelayout(core, view, 1, result, &installed), "install pre-layout")
        }
        _ = viem_layout_work_release(request)
        chunks += 1
        withExtendedLifetime(worker) {}
    }
    let shapeRequestCount = max(shapeRequests, 1)
    print("[\(iteration)] prelayout_ms=\(milliseconds(CFAbsoluteTimeGetCurrent() - start)) compute_ms=\(milliseconds(computeSeconds)) chunks=\(chunks) shape_calls=\(shapeCalls) requests=\(shapeRequests) shape_ms=\(milliseconds(shapeSeconds)) per_request_us=\(String(format: "%.0f", shapeSeconds * 1e6 / Double(shapeRequestCount)))")

    if options.typing > 0 {
        for scalar in "50%".unicodeScalars { sendKey(core, view, scalar) }
        sendKey(core, view, "o")
        var samples: [Double] = []
        resetCounters()
        let text = String(repeating: "the quick brown fox jumps ", count: options.typing / 26 + 1).prefix(options.typing)
        for (index, character) in text.enumerated() {
            // Long runs start new paragraphs, keeping the edited paragraph
            // at an ordinary length. Only typed characters are timed.
            if index > 0, index % 40 == 0 { sendKey(core, view, " ", kind: UInt32(VIEM_KEY_ENTER)) }
            let keyStart = CFAbsoluteTimeGetCurrent()
            sendText(core, view, String(character))
            samples.append(CFAbsoluteTimeGetCurrent() - keyStart)
        }
        print("[\(iteration)] typing_ms \(summary(samples)) shape_calls=\(shapeCalls) requests=\(shapeRequests) shape_ms=\(milliseconds(shapeSeconds))")
        if samples.count >= 2000 {
            // Growth with session length: each consecutive thousand keystrokes.
            let segments = stride(from: 0, to: samples.count, by: 1000).map { start in
                let segment = samples[start..<min(start + 1000, samples.count)].sorted()
                return String(format: "%.2f", segment[segment.count / 2] * 1000)
            }
            print("[\(iteration)] typing_p50_per_1000 \(segments.joined(separator: " "))")
        }
    }

    print("[\(iteration)] response_digest=\(String(digest.value, radix: 16))")
    digest = ResponseDigest()
    check(viem_core_view_remove(core, view), "remove view")
    provider.retireResources()
    check(viem_core_destroy(core), "destroy core")
}

/// Deterministic documents resembling ordinary writing. Keep identical to
/// EVPrelayoutFixtures so measurements compare with the native benchmark.
enum ShapeBenchFixtures {
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
            guard let context = CGContext(data: nil, width: size.0, height: size.1, bitsPerComponent: 8, bytesPerRow: 0,
                space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
            else { throw CocoaError(.fileWriteUnknown) }
            for band in 0..<16 {
                context.setFillColor(CGColor(red: CGFloat(band) / 16, green: CGFloat(index) / 8, blue: 0.6, alpha: 1))
                context.fill(CGRect(x: band * size.0 / 16, y: 0, width: size.0 / 16 + 1, height: size.1))
            }
            guard let image = context.makeImage(),
                  let destination = CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil)
            else { throw CocoaError(.fileWriteUnknown) }
            CGImageDestinationAddImage(destination, image, nil)
            guard CGImageDestinationFinalize(destination) else { throw CocoaError(.fileWriteUnknown) }
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
}
