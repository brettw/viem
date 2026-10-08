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
    XCTAssertTrue(cache.setVisibleLocations([location], viewportID: 2))
    _ = cache.image(for: location)
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
