import AppKit
import ImageIO
import XCTest
@testable import ViemCoreTextProvider

final class CoreTextImageResourcesTests: XCTestCase {
  func testRemoteAndExecutableLocationsNeverStartLocalLoads() {
    let cache = CoreTextImageResources()
    defer { cache.stop() }
    for value in ["https://example.invalid/image.png", "http://localhost/a.png", "//server/a.png",
                  "file://server/a.png", "data:image/png;base64,AAAA", "javascript:alert(1)", "file:///tmp/a%00.png"] {
      XCTAssertNil(CoreTextImageResources.localURL(value, relativeTo: URL(fileURLWithPath: "/tmp/doc.md")), value)
      XCTAssertNil(cache.image(for: value).image)
    }
    XCTAssertEqual(cache.localLoadCount, 0)
  }

  func testRelativePathsPreserveEscapesAndOnlyUseTheDocumentDirectory() {
    let base = URL(fileURLWithPath: "/tmp/docs/page.md")
    XCTAssertEqual(CoreTextImageResources.localURL("../my%20image.png", relativeTo: base)?.path, "/tmp/my image.png")
    XCTAssertEqual(CoreTextImageResources.localURL("literal%2520.png", relativeTo: base)?.path, "/tmp/docs/literal%20.png")
    XCTAssertNil(CoreTextImageResources.localURL("image.png", relativeTo: nil))
  }

  func testPassiveRasterDecodeUsesPixelDimensionsAndRejectsSVG() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let png = directory.appendingPathComponent("local.png")
    let context = try XCTUnwrap(CGContext(data: nil, width: 320, height: 160, bitsPerComponent: 8,
      bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
    context.setFillColor(CGColor(red: 0.2, green: 0.5, blue: 0.8, alpha: 1)); context.fill(CGRect(x: 0, y: 0, width: 320, height: 160))
    let writer = try XCTUnwrap(CGImageDestinationCreateWithURL(png as CFURL, "public.png" as CFString, 1, nil))
    CGImageDestinationAddImage(writer, try XCTUnwrap(context.makeImage()), [kCGImagePropertyDPIWidth: 144, kCGImagePropertyDPIHeight: 144] as CFDictionary)
    XCTAssertTrue(CGImageDestinationFinalize(writer))
    let image = try XCTUnwrap(CoreTextImageResources.decode(png, location: "local.png"))
    XCTAssertEqual(image.size, CGSize(width: 320, height: 160), "Markdown uses intrinsic pixels rather than print DPI")
    XCTAssertNotNil(image.image)
    let svg = directory.appendingPathComponent("external.svg")
    try Data("<svg xmlns='http://www.w3.org/2000/svg'><image href='https://example.invalid/tracker.png'/></svg>".utf8).write(to: svg)
    XCTAssertNil(CoreTextImageResources.decode(svg, location: "external.svg"))
  }

  func testPreviewFileLimitUsesDecimalMegabytesAndIncludesExactBoundary() throws {
    let file = FileManager.default.temporaryDirectory.appendingPathComponent("viem-byte-limit-\(UUID().uuidString).png")
    defer { try? FileManager.default.removeItem(at: file) }
    try writePNG(file, width: 32, height: 16)
    var bytes = try Data(contentsOf: file)
    XCTAssertLessThan(bytes.count, 10_000_000)
    bytes.append(Data(count: 10_000_000 - bytes.count))
    try bytes.write(to: file)
    let accepted = try XCTUnwrap(CoreTextImageResources.decode(file, location: file.path))
    XCTAssertEqual(accepted.state, .ready)
    XCTAssertNotNil(accepted.image)
    XCTAssertEqual(accepted.size, CGSize(width: 32, height: 16))

    bytes.append(0)
    try bytes.write(to: file)
    let rejected = try XCTUnwrap(CoreTextImageResources.decode(file, location: file.path))
    XCTAssertEqual(rejected.state, .limited, "A size limit is a plain location placeholder, not a broken image")
    XCTAssertNil(rejected.image)
    XCTAssertEqual(rejected.location, file.path)
  }

  func testPixelLimitsInclude5000AndReject5001InEitherDimension() throws {
    let file = FileManager.default.temporaryDirectory.appendingPathComponent("viem-pixel-limit-\(UUID().uuidString).png")
    defer { try? FileManager.default.removeItem(at: file) }
    for (width, height, expected) in [(5000, 2, CoreTextInlineImage.State.ready), (2, 5000, .ready),
                                     (5001, 2, .limited), (2, 5001, .limited)] {
      try writePNG(file, width: width, height: height)
      let value = try XCTUnwrap(CoreTextImageResources.decode(file, location: file.path))
      XCTAssertEqual(value.state, expected, "\(width)×\(height)")
      if expected == .ready {
        XCTAssertNotNil(value.image)
        XCTAssertEqual(value.size, CGSize(width: width, height: height))
      } else {
        XCTAssertNil(value.image)
        XCTAssertEqual(value.size, CGSize(width: 300, height: 64), "Rejected dimensions must not become layout metrics")
      }
    }
    try Data("This is not a valid image".utf8).write(to: file)
    XCTAssertNil(CoreTextImageResources.decode(file, location: file.path), "Corruption remains the broken-image result")
  }

  @MainActor
  func testLimitedImagesRemainStableUntilReloadAndRecoverWhenResourceFits() throws {
    let file = FileManager.default.temporaryDirectory.appendingPathComponent("viem-limited-reload-\(UUID().uuidString).png")
    defer { try? FileManager.default.removeItem(at: file) }
    try writePNG(file, width: 5001, height: 2)
    let cache = CoreTextImageResources()
    defer { cache.stop() }
    _ = cache.setVisibleLocations([file.path], viewportID: 1)
    _ = cache.image(for: file.path)
    drainUntil { cache.pendingLoadCount == 0 }
    let limited = cache.image(for: file.path)
    XCTAssertEqual(limited.state, .limited)
    XCTAssertNil(limited.image)
    XCTAssertEqual(cache.retainedRasterBytes, 0)
    XCTAssertEqual(cache.retainedEntryCount, 0, "Terminal placeholders use metadata, not bitmap admission")
    for viewport in 2...5 {
      XCTAssertFalse(cache.setVisibleLocations([file.path], viewportID: UInt64(viewport)))
      XCTAssertEqual(cache.image(for: file.path).identity, limited.identity)
    }
    XCTAssertEqual(cache.localLoadCount, 1)
    try writePNG(file, width: 80, height: 40)
    XCTAssertEqual(cache.image(for: file.path).state, .limited)
    XCTAssertTrue(cache.reload(file.path))
    XCTAssertEqual(cache.image(for: file.path).size, limited.size, "Reload keeps accepted metrics until completion")
    drainUntil { cache.pendingLoadCount == 0 }
    let accepted = cache.image(for: file.path)
    XCTAssertEqual(accepted.state, .ready)
    XCTAssertNotNil(accepted.image)
    XCTAssertEqual(accepted.size, CGSize(width: 80, height: 40))
    XCTAssertEqual(cache.localLoadCount, 2)
    XCTAssertNotEqual(accepted.identity, limited.identity)
  }

  @MainActor
  func testLimitKnowledgeResetsOnDocumentIdentityChangeAndCorruptionStaysBroken() throws {
    let file = FileManager.default.temporaryDirectory.appendingPathComponent("viem-limited-reset-\(UUID().uuidString).png")
    defer { try? FileManager.default.removeItem(at: file) }
    try writePNG(file, width: 2, height: 5001)
    let cache = CoreTextImageResources()
    defer { cache.stop() }
    _ = cache.image(for: file.path)
    drainUntil { cache.pendingLoadCount == 0 }
    XCTAssertEqual(cache.image(for: file.path).state, .limited)
    XCTAssertEqual(cache.localLoadCount, 1)
    try Data("corrupt".utf8).write(to: file)
    XCTAssertTrue(cache.setDocumentURL(file.deletingLastPathComponent().appendingPathComponent("new.md")))
    XCTAssertEqual(cache.image(for: file.path).state, .pending)
    drainUntil { cache.pendingLoadCount == 0 }
    XCTAssertEqual(cache.image(for: file.path).state, .broken)
    XCTAssertEqual(cache.localLoadCount, 2)
    XCTAssertFalse(cache.setVisibleLocations([file.path], viewportID: 1))
    XCTAssertEqual(cache.image(for: file.path).state, .broken)
    XCTAssertEqual(cache.localLoadCount, 2)
  }

  @MainActor
  func testFullQueueBackpressureRetriesVisibleLoadsAfterFailures() {
    let probe = ImageDecodeProbe()
    let cache = CoreTextImageResources(decoder: { url, location in probe.decode(url, location: location) })
    defer { probe.release(64); cache.stop() }
    let locations = (0..<20).map { "/tmp/image-\($0).png" }
    XCTAssertTrue(cache.setVisibleLocations(locations, viewportID: 1))
    cache.setChangeHandler { for location in locations { _ = cache.image(for: location) } }
    for location in locations { _ = cache.image(for: location) }
    drainUntil { probe.started >= 2 }
    XCTAssertEqual(cache.pendingLoadCount, 16)
    XCTAssertEqual(cache.localLoadCount, 16)
    XCTAssertFalse(cache.setVisibleLocations(locations, viewportID: 1), "A full queue waits for completion instead of requesting another immediate layout")
    XCTAssertLessThanOrEqual(probe.maximumActive, 2)
    probe.release(64)
    drainUntil { cache.pendingLoadCount == 0 && cache.localLoadCount == 20 }
    XCTAssertEqual(probe.started, 20)
    XCTAssertLessThanOrEqual(probe.maximumActive, 2)
  }

  @MainActor
  func testImageDrivenMembershipChangesDoNotResetAttemptBudget() {
    let cache = CoreTextImageResources(decoder: { _, _ in nil })
    defer { cache.stop() }
    let location = "/tmp/missing.png"
    XCTAssertTrue(cache.setVisibleLocations([location], viewportID: 1))
    _ = cache.image(for: location)
    drainUntil { cache.pendingLoadCount == 0 }
    XCTAssertFalse(cache.setVisibleLocations([], viewportID: 1))
    XCTAssertFalse(cache.setVisibleLocations([location], viewportID: 1))
    _ = cache.image(for: location)
    XCTAssertEqual(cache.localLoadCount, 1)
    XCTAssertFalse(cache.setVisibleLocations([location], viewportID: 2))
    XCTAssertEqual(cache.image(for: location).state, .broken)
    XCTAssertEqual(cache.localLoadCount, 1, "A failed file stays a stable broken placeholder until Reload")
    XCTAssertTrue(cache.reload(location))
    drainUntil { cache.pendingLoadCount == 0 }
    XCTAssertEqual(cache.localLoadCount, 2)
  }

  @MainActor
  func testBudgetKeepsVisibleImagesAndDeclinesWithoutReloadChurn() throws {
    let raster = try raster(width: 2048, height: 2048)
    let fixture = CoreTextInlineImage(location: "fixture", image: raster,
      size: CGSize(width: 2048, height: 2048), identity: UUID())
    let cache = CoreTextImageResources(decoder: { _, location in
      CoreTextInlineImage(location: location, image: fixture.image, size: fixture.size, identity: UUID())
    })
    defer { cache.stop() }
    let locations = (0..<5).map { "/tmp/large-\($0).png" }
    XCTAssertTrue(cache.setVisibleLocations(locations, viewportID: 1))
    cache.setChangeHandler {
      _ = cache.setVisibleLocations(locations, viewportID: 1)
      for location in locations { _ = cache.image(for: location) }
    }
    for location in locations { _ = cache.image(for: location) }
    drainUntil { cache.pendingLoadCount == 0 }
    let rejected = locations.filter { cache.image(for: $0).image == nil }
    XCTAssertEqual(rejected.count, 1)
    XCTAssertEqual(cache.localLoadCount, 5)
    XCTAssertLessThanOrEqual(cache.retainedRasterBytes, 64 * 1024 * 1024)
    // Geometry reflow can hide/reveal the denied image, without a user scroll.
    for _ in 0..<3 {
      XCTAssertFalse(cache.setVisibleLocations(rejected, viewportID: 1))
      XCTAssertFalse(cache.setVisibleLocations(locations, viewportID: 1))
      for location in locations { _ = cache.image(for: location) }
    }
    XCTAssertEqual(cache.localLoadCount, 5)
    cache.setChangeHandler(nil)
    XCTAssertTrue(cache.setVisibleLocations(rejected, viewportID: 2))
    for location in rejected { _ = cache.image(for: location) }
    drainUntil { cache.pendingLoadCount == 0 }
    XCTAssertEqual(cache.localLoadCount, 6)
    XCTAssertTrue(rejected.allSatisfy { cache.image(for: $0).image != nil })
    XCTAssertLessThanOrEqual(cache.retainedRasterBytes, 64 * 1024 * 1024)
  }

  @MainActor
  func testEntryEvictionKeepsVisibleEntriesAndTracksItsChosenKey() throws {
    let fixture = CoreTextInlineImage(location: "fixture", image: try raster(width: 1, height: 1),
      size: CGSize(width: 1, height: 1), identity: UUID())
    let cache = CoreTextImageResources(decoder: { _, location in
      CoreTextInlineImage(location: location, image: fixture.image, size: fixture.size, identity: UUID())
    })
    defer { cache.stop() }
    let original = (0..<64).map { "/tmp/tiny-\($0).png" }
    _ = cache.setVisibleLocations(original, viewportID: 1)
    cache.setChangeHandler { for location in original { _ = cache.image(for: location) } }
    for location in original { _ = cache.image(for: location) }
    drainUntil { cache.pendingLoadCount == 0 && cache.localLoadCount == 64 }
    cache.setChangeHandler(nil)
    XCTAssertEqual(cache.retainedEntryCount, 64)
    let retained = Array(original.prefix(63))
    for index in 0..<4 {
      let added = "/tmp/new-\(index).png"
      _ = cache.setVisibleLocations(retained + [added], viewportID: UInt64(index + 2))
      _ = cache.image(for: added)
      drainUntil { cache.pendingLoadCount == 0 }
      XCTAssertEqual(cache.retainedEntryCount, 64)
      XCTAssertTrue(retained.allSatisfy { cache.image(for: $0).image != nil })
    }
    XCTAssertEqual(cache.localLoadCount, 68)
  }

  @MainActor
  func testDocumentURLChangeDrainsOldJobsWithoutCancellingNewEpoch() throws {
    let fixture = CoreTextInlineImage(location: "fixture", image: try raster(width: 8, height: 4),
      size: CGSize(width: 8, height: 4), identity: UUID())
    let probe = ImageDecodeProbe(image: fixture)
    let cache = CoreTextImageResources(decoder: { url, location in probe.decode(url, location: location) })
    defer { probe.release(64); cache.stop() }
    _ = cache.setDocumentURL(URL(fileURLWithPath: "/tmp/old/document.md"))
    _ = cache.setVisibleLocations(["image.png"], viewportID: 1)
    _ = cache.image(for: "image.png")
    drainUntil { probe.started == 1 }
    _ = cache.setDocumentURL(URL(fileURLWithPath: "/tmp/new/document.md"))
    _ = cache.setVisibleLocations(["image.png"], viewportID: 2)
    _ = cache.image(for: "image.png")
    drainUntil { probe.started == 2 }
    XCTAssertEqual(cache.pendingLoadCount, 2)
    probe.release(64)
    drainUntil { cache.pendingLoadCount == 0 }
    let image = cache.image(for: "image.png")
    XCTAssertNotNil(image.image)
    XCTAssertEqual(image.size.width, 40, "Old-directory completion must not install over the new resource")
    XCTAssertEqual(cache.localLoadCount, 2)
  }

  @MainActor
  func testKnownIntrinsicSizeSurvivesBitmapEvictionSynchronously() throws {
    let fixture = CoreTextInlineImage(location: "fixture", image: try raster(width: 2, height: 1),
      size: CGSize(width: 1200, height: 600), identity: UUID())
    let cache = CoreTextImageResources(decoder: { _, location in
      CoreTextInlineImage(location: location, image: fixture.image, size: fixture.size, identity: UUID())
    })
    defer { cache.stop() }
    for index in 0..<65 {
      let location = "/tmp/eviction-\(index).png"
      _ = cache.setVisibleLocations([location], viewportID: UInt64(index + 1))
      _ = cache.image(for: location)
      drainUntil { cache.pendingLoadCount == 0 }
    }
    XCTAssertEqual(cache.retainedEntryCount, 64)
    let evicted = cache.image(for: "/tmp/eviction-0.png")
    XCTAssertNil(evicted.image)
    XCTAssertEqual(evicted.state, .ready)
    XCTAssertEqual(evicted.size, fixture.size, "Metrics remain exact with no bitmap or disk access")
    XCTAssertEqual(cache.localLoadCount, 65)
    _ = cache.setVisibleLocations(["/tmp/eviction-0.png"], viewportID: 66)
    XCTAssertEqual(cache.image(for: "/tmp/eviction-0.png").size, fixture.size)
    drainUntil { cache.pendingLoadCount == 0 }
  }

  @MainActor
  func testReloadReadsChangedDiskFileAndCanRecoverBrokenImages() throws {
    let file = FileManager.default.temporaryDirectory.appendingPathComponent("viem-reload-\(UUID().uuidString).png")
    defer { try? FileManager.default.removeItem(at: file) }
    func write(_ width: Int, _ height: Int) throws {
      let destination = try XCTUnwrap(CGImageDestinationCreateWithURL(file as CFURL, "public.png" as CFString, 1, nil))
      CGImageDestinationAddImage(destination, try raster(width: width, height: height), nil)
      XCTAssertTrue(CGImageDestinationFinalize(destination))
    }
    try write(80, 40)
    let cache = CoreTextImageResources(); defer { cache.stop() }
    _ = cache.image(for: file.path)
    drainUntil { cache.pendingLoadCount == 0 }
    let original = cache.image(for: file.path)
    XCTAssertEqual(original.size, CGSize(width: 80, height: 40))
    try write(120, 30)
    XCTAssertTrue(cache.reload(file.path))
    XCTAssertEqual(cache.image(for: file.path).size, original.size, "Keep accepted size during reload")
    drainUntil { cache.pendingLoadCount == 0 }
    let replacement = cache.image(for: file.path)
    XCTAssertEqual(replacement.size, CGSize(width: 120, height: 30))
    XCTAssertNotEqual(replacement.identity, original.identity)
    try FileManager.default.removeItem(at: file)
    XCTAssertTrue(cache.reload(file.path))
    drainUntil { cache.pendingLoadCount == 0 }
    XCTAssertEqual(cache.image(for: file.path).state, .broken)
    XCTAssertNil(cache.image(for: file.path).image)
    try write(32, 16)
    XCTAssertTrue(cache.reload(file.path))
    drainUntil { cache.pendingLoadCount == 0 }
    XCTAssertEqual(cache.image(for: file.path).state, .ready)
    XCTAssertEqual(cache.image(for: file.path).size, CGSize(width: 32, height: 16))
    let count = cache.localLoadCount
    XCTAssertFalse(cache.reload("https://example.invalid/image.png"))
    XCTAssertFalse(cache.canReload("https://example.invalid/image.png"))
    XCTAssertEqual(cache.localLoadCount, count)
  }

  func testMetadataBudgetExhaustionStopsAdmissionRefreshRetries() {
    let cache = CoreTextImageResources(metadataByteBudget: 0)
    defer { cache.stop() }
    let locations = ["/tmp/budget.png"]
    XCTAssertTrue(cache.setVisibleLocations(locations, viewportID: 1))
    XCTAssertNil(cache.image(for: locations[0]).image)
    for _ in 0..<20 { XCTAssertFalse(cache.setVisibleLocations(locations, viewportID: 1)) }
    XCTAssertEqual(cache.localLoadCount, 0)
    XCTAssertEqual(cache.pendingLoadCount, 0)
  }

  @MainActor
  private func drainUntil(_ condition: () -> Bool, file: StaticString = #filePath, line: UInt = #line) {
    let deadline = Date().addingTimeInterval(5)
    while !condition() && Date() < deadline { RunLoop.current.run(until: Date().addingTimeInterval(0.005)) }
    XCTAssertTrue(condition(), "Image loading did not settle", file: file, line: line)
  }

  private func raster(width: Int, height: Int) throws -> CGImage {
    let context = try XCTUnwrap(CGContext(data: nil, width: width, height: height, bitsPerComponent: 8,
      bytesPerRow: width * 4, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
    return try XCTUnwrap(context.makeImage())
  }

  private func writePNG(_ file: URL, width: Int, height: Int) throws {
    let destination = try XCTUnwrap(CGImageDestinationCreateWithURL(file as CFURL, "public.png" as CFString, 1, nil))
    CGImageDestinationAddImage(destination, try raster(width: width, height: height), nil)
    XCTAssertTrue(CGImageDestinationFinalize(destination))
  }

}


private final class ImageDecodeProbe: @unchecked Sendable {
  private let lock = NSLock()
  private let gate = DispatchSemaphore(value: 0)
  private var count = 0
  private var active = 0
  private var maximum = 0
  private let image: CoreTextInlineImage?
  init(image: CoreTextInlineImage? = nil) { self.image = image }
  var started: Int { lock.lock(); defer { lock.unlock() }; return count }
  var maximumActive: Int { lock.lock(); defer { lock.unlock() }; return maximum }
  func release(_ count: Int) { for _ in 0..<count { gate.signal() } }
  func decode(_ url: URL, location: String) -> CoreTextInlineImage? {
    lock.lock(); count += 1; active += 1; maximum = max(maximum, active); lock.unlock()
    _ = gate.wait(timeout: .now() + 5)
    lock.lock(); active -= 1; lock.unlock()
    guard let image else { return nil }
    return CoreTextInlineImage(location: location, image: image.image,
      size: CGSize(width: url.path.contains("/old/") ? 80 : 40, height: 20), identity: UUID())
  }
}
