import AppKit
import CEvimCore
import XCTest

@testable import EvimEditor

final class EVArabicTypographyTests: XCTestCase {
  @MainActor
  func testLongArabicAndMixedBidiWrapWithinTheViewportAtLegalBoundaries() throws {
    for text in [
      String(repeating: "مرحبا بالعالم هذه فقرة عربية طويلة لاختبار التفاف الكلمات ", count: 8),
      String(repeating: "English العربية 123 ثم English مرحبا بالعالم ", count: 8),
    ] {
      let surface = try makeSurface(text, width: 310)
      let session = try XCTUnwrap(surface.session)
      _ = try session.setWrap(true)
      surface.refreshPresentation()
      let snapshot = try XCTUnwrap(surface.layoutSnapshot)
      XCTAssertGreaterThan(snapshot.rows.count, 3)
      XCTAssertEqual(snapshot.info.document_hard_line_count, 1)
      var boundaries = Set<UInt64>([0])
      var offset: UInt64 = 0
      for character in text {
        offset += UInt64(character.utf8.count)
        boundaries.insert(offset)
      }
      for cluster in snapshot.clusters {
        XCTAssertTrue(boundaries.contains(cluster.text_start))
        XCTAssertTrue(boundaries.contains(cluster.text_end))
        XCTAssertGreaterThanOrEqual(cluster.advance, 0)
        XCTAssertLessThan(cluster.advance, 40)
      }
      for row in snapshot.rows {
        let clusters = snapshot.clusters.filter { $0.row_index == row.row_index }.sorted {
          $0.x < $1.x
        }
        for pair in zip(clusters, clusters.dropFirst()) {
          XCTAssertLessThanOrEqual(pair.0.x + pair.0.advance, pair.1.x + 0.02)
        }
        XCTAssertLessThanOrEqual(row.width, 310)
      }
    }
  }

  @MainActor
  func testLargeArabicDocumentRetainsUnchangedRenderResourcesAndInvalidatesFontGeneration() throws {
    let text = (0..<10_000).map { "فقرة عربية مرحبا بالعالم \($0)" }.joined(separator: "\n")
    let surface = try makeSurface(text, width: 360)
    let session = try XCTUnwrap(surface.session)
    let before = try XCTUnwrap(surface.layoutSnapshot)
    XCTAssertEqual(before.info.document_hard_line_count, 10_000)
    XCTAssertLessThan(before.rows.count, 100)
    let surviving = Set(
      before.clusters.filter { $0.row_index == 2 }.map { $0.render_run.identifier })
    surface.performInput { _ = try session.sendText("iX") }
    let after = try XCTUnwrap(surface.layoutSnapshot)
    XCTAssertLessThan(after.rows.count, 100)
    let reused = Set(after.clusters.filter { $0.row_index == 2 }.map { $0.render_run.identifier })
    XCTAssertEqual(reused, surviving)
    let generation = session.provider.metricsGeneration
    session.provider.invalidateMetrics()
    surface.refreshPresentation()
    let refreshed = try XCTUnwrap(surface.layoutSnapshot)
    XCTAssertGreaterThan(session.provider.metricsGeneration, generation)
    XCTAssertTrue(
      refreshed.clusters.allSatisfy {
        $0.render_run.metrics_generation == session.provider.metricsGeneration
      })
    XCTAssertLessThan(refreshed.rows.count, 100)
  }

  @MainActor
  private func makeSurface(_ text: String, width: CGFloat) throws -> EVEditorSurfaceController {
    let backend = EVCoreDocumentBackend()
    try backend.read(source: Data(text.utf8), typeName: "public.plain-text")
    let surface = try XCTUnwrap(backend.makeEditorSurface() as? EVEditorSurfaceController)
    surface.loadViewIfNeeded()
    surface.view.frame = NSRect(x: 0, y: 0, width: width, height: 650)
    surface.viewDidLayout()
    return surface
  }
}
