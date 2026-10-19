import AppKit
import ImageIO
import XCTest
@testable import ViemCoreTextProvider

final class CoreTextImageResourcesTests: XCTestCase {
  @MainActor
  func testDocumentViewsReuseOneRasterAndReloadTogetherAfterAViewCloses() throws {
    let file = FileManager.default.temporaryDirectory.appendingPathComponent("viem-shared-image-\(UUID().uuidString).png")
    defer { try? FileManager.default.removeItem(at: file) }
    try writePNG(file, width: 80, height: 40)
    let images = CoreTextDocumentImages()
    let first = CoreTextMeasurementProvider(documentImages: images)
    let cache = images.resources
    _ = cache.setVisibleLocations([file.path], viewportID: 1, consumer: first.imageConsumerID)
    drainUntil { cache.pendingLoadCount == 0 }
    let raster = try XCTUnwrap(cache.image(for: file.path).image)
    let retainedBytes = cache.retainedRasterBytes
    let second = CoreTextMeasurementProvider(documentImages: images)
    _ = cache.setVisibleLocations([file.path], viewportID: 1, consumer: second.imageConsumerID)
    XCTAssertTrue(try XCTUnwrap(cache.image(for: file.path).image) === raster)
    XCTAssertEqual(second.localImageLoadCountForTesting, 1)
    XCTAssertEqual(cache.retainedRasterBytes, retainedBytes, "Another view retains no duplicate decoded pixels")
    XCTAssertEqual(cache.pendingLoadCount, 0)
    // Scrolling the first view away must leave the second view's admission.
    _ = cache.setVisibleLocations([], viewportID: 2, consumer: first.imageConsumerID)
    first.retireResources()
    XCTAssertTrue(try XCTUnwrap(cache.image(for: file.path).image) === raster)
    let third = CoreTextMeasurementProvider(documentImages: images)
    let secondChanged = expectation(description: "second view reload")
    let thirdChanged = expectation(description: "third view reload")
    first.imagesDidChange = { _, _ in XCTFail("closed provider notified") }
    second.imagesDidChange = { _, locations in XCTAssertEqual(locations, [file.path]); secondChanged.fulfill() }
    third.imagesDidChange = { _, locations in XCTAssertEqual(locations, [file.path]); thirdChanged.fulfill() }
    try writePNG(file, width: 120, height: 60)
    XCTAssertTrue(second.reloadImage(file.path))
    wait(for: [secondChanged, thirdChanged], timeout: 5)
    XCTAssertEqual(cache.localLoadCount, 2)
    XCTAssertEqual(cache.image(for: file.path).size, CGSize(width: 120, height: 60))
    XCTAssertNotNil(cache.image(for: file.path).image)
  }

  @MainActor
  func testNewlyInsertedImageInvalidatesPlaceholderGeometryAfterAdmission() throws {
    let file = FileManager.default.temporaryDirectory.appendingPathComponent("viem-insert-image-\(UUID().uuidString).png")
    defer { try? FileManager.default.removeItem(at: file) }
    try writePNG(file, width: 200, height: 100)
    let cache = CoreTextImageResources()
    defer { cache.stop() }
    _ = cache.setVisibleLocations([], viewportID: 1)
    let initial = cache.image(for: file.path)
    XCTAssertEqual(initial.state, .pending)
    XCTAssertEqual(cache.localLoadCount, 0, "Layout precedes visible admission")
    let changed = expectation(description: "Replace served placeholder geometry")
    cache.setResourceChangeHandler { locations in
      XCTAssertEqual(locations, [file.path]); changed.fulfill()
    }
    _ = cache.setVisibleLocations([file.path], viewportID: 2)
    wait(for: [changed], timeout: 5)
    XCTAssertEqual(cache.image(for: file.path).size, CGSize(width: 200, height: 100))
    XCTAssertNotNil(cache.image(for: file.path).image)
  }

  @MainActor
  func testPopupDimensionsUseHeaderOnlyAndRemoteLocationsStayPassive() throws {
    let file = FileManager.default.temporaryDirectory.appendingPathComponent("viem-popup-size-\(UUID().uuidString).png")
    defer { try? FileManager.default.removeItem(at: file) }
    try writePNG(file, width: 321, height: 123)
    let cache = CoreTextImageResources()
    defer { cache.stop() }
    let local = expectation(description: "Local header")
    _ = cache.requestDimensions(file.path) { size in
      XCTAssertEqual(size, CGSize(width: 321, height: 123)); local.fulfill()
    }
    wait(for: [local], timeout: 5)
    let remote = expectation(description: "Passive remote")
    _ = cache.requestDimensions("https://example.invalid/image.png") { size in
      XCTAssertNil(size); remote.fulfill()
    }
    wait(for: [remote], timeout: 5)
    XCTAssertEqual(cache.localLoadCount, 0)
    XCTAssertEqual(cache.dimensionProbeCount, 1)
    XCTAssertEqual(cache.retainedRasterBytes, 0)
  }

  @MainActor
  func testPopupDimensionRequestsCoalesceAndCancelledDraftsCannotPublish() {
    let entered = DispatchSemaphore(value: 0)
    let release = DispatchSemaphore(value: 0)
    let cache = CoreTextImageResources(prober: { _, location in
      if location == "/tmp/first.png" { entered.signal(); release.wait() }
      return CoreTextInlineImage(location: location, image: nil, size: CGSize(width: 80, height: 40),
        identity: UUID(), state: .measured)
    })
    defer { release.signal(); cache.stop() }
    let first = cache.requestDimensions("/tmp/first.png") { _ in XCTFail("Cancelled draft published") }
    XCTAssertEqual(entered.wait(timeout: .now() + 2), .success)
    first.cancel()
    for index in 0..<100 {
      _ = cache.requestDimensions("/tmp/superseded-\(index).png") { _ in XCTFail("Superseded draft published") }
    }
    let finished = expectation(description: "Newest draft")
    _ = cache.requestDimensions("/tmp/current.png") { size in
      XCTAssertEqual(size, CGSize(width: 80, height: 40)); finished.fulfill()
    }
    release.signal()
    wait(for: [finished], timeout: 5)
    XCTAssertEqual(cache.dimensionProbeCount, 2)
    XCTAssertEqual(cache.localLoadCount, 0)
  }

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

  /// Counts change notifications, which always arrive on the main thread.
  private final class Notifications: @unchecked Sendable {
    var geometry: [[String]] = []
    var rasters = 0
  }

  private func observed(_ cache: CoreTextImageResources) -> Notifications {
    let notifications = Notifications()
    cache.setResourceChangeHandler { locations in
      MainActor.assumeIsolated { notifications.geometry.append(locations) }
    }
    cache.setRasterChangeHandler { MainActor.assumeIsolated { notifications.rasters += 1 } }
    return notifications
  }

  @MainActor
  func testWorkerProbeMakesDimensionsExactAndLaterDecodeOnlyRepaints() throws {
    let file = FileManager.default.temporaryDirectory.appendingPathComponent("viem-probe-\(UUID().uuidString).png")
    defer { try? FileManager.default.removeItem(at: file) }
    try writePNG(file, width: 80, height: 40)
    let cache = CoreTextImageResources()
    defer { cache.stop() }
    let notifications = observed(cache)
    let probed = cache.image(for: file.path, probe: true)
    XCTAssertEqual(probed.state, .measured)
    XCTAssertEqual(probed.size, CGSize(width: 80, height: 40))
    XCTAssertNil(probed.image, "A probe reads the header without decoding pixels")
    XCTAssertEqual(cache.dimensionProbeCount, 1)
    XCTAssertEqual(cache.localLoadCount, 0)
    XCTAssertEqual(cache.image(for: file.path, probe: true).identity, probed.identity, "Known dimensions are not probed again")
    XCTAssertEqual(cache.dimensionProbeCount, 1)
    XCTAssertTrue(cache.setVisibleLocations([file.path], viewportID: 1), "Admission starts the bitmap load itself")
    drainUntil { cache.pendingLoadCount == 0 && notifications.rasters > 0 }
    let decoded = cache.image(for: file.path)
    XCTAssertEqual(decoded.state, .ready)
    XCTAssertNotNil(decoded.image)
    XCTAssertEqual(decoded.layoutKey, probed.layoutKey)
    XCTAssertEqual(decoded.identity, probed.identity, "Unchanged geometry keeps its render identity")
    XCTAssertTrue(notifications.geometry.isEmpty, "A bitmap with the probed size never reshapes layout")
    XCTAssertEqual(cache.presentation(matching: probed.withoutRaster())?.image != nil, true, "Drawing finds the current bitmap")
  }

  @MainActor
  func testPlaceholderUsedByLayoutReportsOneGeometryChange() throws {
    let file = FileManager.default.temporaryDirectory.appendingPathComponent("viem-placeholder-\(UUID().uuidString).png")
    defer { try? FileManager.default.removeItem(at: file) }
    try writePNG(file, width: 120, height: 30)
    let cache = CoreTextImageResources()
    defer { cache.stop() }
    let notifications = observed(cache)
    _ = cache.setVisibleLocations([file.path], viewportID: 1)
    let placeholder = cache.image(for: file.path)
    XCTAssertEqual(placeholder.state, .pending)
    drainUntil { cache.pendingLoadCount == 0 && !notifications.geometry.isEmpty }
    XCTAssertEqual(notifications.geometry, [[file.path]])
    XCTAssertEqual(notifications.rasters, 0)
    XCTAssertNil(cache.presentation(matching: placeholder), "Stale placeholder geometry never draws the new bitmap")
    XCTAssertEqual(cache.image(for: file.path).size, CGSize(width: 120, height: 30))
  }

  @MainActor
  func testDisplayRastersAreResampledOffTheMainThreadOncePerBitmapAndSize() throws {
    let file = FileManager.default.temporaryDirectory.appendingPathComponent("viem-display-\(UUID().uuidString).png")
    defer { try? FileManager.default.removeItem(at: file) }
    try writePNG(file, width: 1000, height: 500)
    let cache = CoreTextImageResources()
    defer { cache.stop() }
    let notifications = observed(cache)
    _ = cache.setVisibleLocations([file.path], viewportID: 1)
    drainUntil { cache.pendingLoadCount == 0 && cache.image(for: file.path).image != nil }
    let image = cache.image(for: file.path)
    let repaints = notifications.rasters
    func resampled(_ size: CGSize) throws -> CGImage {
      XCTAssertTrue(cache.displayRaster(for: image, pixels: size) === image.image, "The original draws until resampling finishes")
      drainUntil { cache.displayRaster(for: image, pixels: size) !== image.image }
      return try XCTUnwrap(cache.displayRaster(for: image, pixels: size))
    }
    let small = try resampled(CGSize(width: 200, height: 100))
    XCTAssertEqual(small.width, 200)
    XCTAssertEqual(small.height, 100)
    XCTAssertEqual(notifications.rasters, repaints + 1, "A finished resample repaints once")
    XCTAssertTrue(cache.displayRaster(for: image, pixels: CGSize(width: 200, height: 100)) === small)
    XCTAssertEqual(cache.scaledRenderCount, 1)
    XCTAssertEqual(try resampled(CGSize(width: 2000, height: 1000)).width, 2000, "Upscaled drawing is resampled too")
    XCTAssertEqual(cache.scaledRenderCount, 2)
    XCTAssertTrue(cache.displayRaster(for: image, pixels: CGSize(width: 200, height: 100)) === small,
                  "Views at different zoom levels retain their own scaled variant")
    XCTAssertEqual(cache.scaledRenderCount, 2)
    XCTAssertTrue(cache.displayRaster(for: image, pixels: CGSize(width: 1050, height: 520)) === image.image,
                  "Near-native sizes draw the original bitmap")
    XCTAssertTrue(cache.displayRaster(for: image, pixels: CGSize(width: 4000, height: 2000)) === image.image,
                  "Very large drawings are not cached")
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
