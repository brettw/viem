import AppKit

/// Native window placement is separate from document/view layout. The first
/// window restores the last frame; later windows use AppKit's cascade spacing.
@MainActor
public final class EVDocumentWindowPlacement {
  public static let shared = EVDocumentWindowPlacement()

  private let loadFrame: @MainActor () -> NSRect?
  private let saveFrame: @MainActor (NSRect) throws -> Void
  private let visibleScreens: @MainActor () -> [NSRect]
  private let nextCascadePoint: @MainActor (NSWindow) -> NSPoint
  private var lastFrame: NSRect?
  private var hasPlacedWindow = false

  public convenience init(configuration: EVConfigurationStore? = nil) {
    let configuration = configuration ?? .shared
    self.init(
      loadFrame: { configuration.documentWindowFrame },
      saveFrame: { try configuration.setDocumentWindowFrame($0) },
      visibleScreens: {
        let screens = NSScreen.screens
        guard let main = NSScreen.main else { return screens.map(\.visibleFrame) }
        return [main.visibleFrame] + screens.filter { $0 !== main }.map(\.visibleFrame)
      },
      nextCascadePoint: { $0.cascadeTopLeft(from: .zero) }
    )
  }

  /// Inject storage and desktop geometry so tests do not depend on connected
  /// displays, the user's saved frame, or the Window Server's cascade spacing.
  init(
    loadFrame: @escaping @MainActor () -> NSRect?,
    saveFrame: @escaping @MainActor (NSRect) throws -> Void = { _ in },
    visibleScreens: @escaping @MainActor () -> [NSRect] = { [] },
    nextCascadePoint: @escaping @MainActor (NSWindow) -> NSPoint = { window in
      let height = window.frame.height - window.contentRect(forFrameRect: window.frame).height
      return NSPoint(x: window.frame.minX + height, y: window.frame.maxY - height)
    }
  ) {
    self.loadFrame = loadFrame
    self.saveFrame = saveFrame
    self.visibleScreens = visibleScreens
    self.nextCascadePoint = nextCascadePoint
  }

  func initialFrame(for window: NSWindow, defaultContentSize: NSSize) -> NSRect {
    let screens = visibleScreens().filter(Self.isValid)
    let stored = lastFrame ?? loadFrame()
    var frame: NSRect
    if let stored, Self.isValid(stored) {
      frame = stored
    } else {
      frame = window.frameRect(forContentRect: NSRect(origin: .zero, size: defaultContentSize))
      if let screen = screens.first {
        frame.origin = NSPoint(x: screen.midX - frame.width / 2, y: screen.midY - frame.height / 2)
      }
    }
    let minimum = window.frameRect(forContentRect: NSRect(origin: .zero, size: window.contentMinSize)).size
    frame.size.width = max(frame.width, minimum.width)
    frame.size.height = max(frame.height, minimum.height)
    frame = Self.fitting(frame, to: screens)

    if hasPlacedWindow {
      // .zero asks AppKit for the next title-bar position without relocating
      // the candidate except for its normal visible-screen constraints. Apply
      // our complete-frame fit afterwards so moving precedes any shrinking.
      window.setFrame(frame, display: false)
      let next = nextCascadePoint(window)
      if next.x.isFinite && next.y.isFinite {
        frame.origin = NSPoint(x: next.x, y: next.y - frame.height)
      }
      frame = Self.fitting(frame, to: screens)
    }
    hasPlacedWindow = true
    lastFrame = frame
    return frame
  }

  func record(window: NSWindow) {
    guard !window.isMiniaturized, !window.styleMask.contains(.fullScreen) else { return }
    record(frame: window.frame)
  }

  /// Recheck after first ordering if AppKit only then acquired its screens.
  /// This deliberately does not advance the cascade sequence.
  func fittedFrame(_ frame: NSRect) -> NSRect {
    Self.fitting(frame, to: visibleScreens())
  }

  func record(frame: NSRect) {
    guard Self.isValid(frame) else { return }
    lastFrame = frame
    guard frame != loadFrame() else { return }
    // Configuration retains the diagnostic and refuses to overwrite invalid
    // user data. Failure to save placement must not interrupt moving a window.
    try? saveFrame(frame)
  }

  static func isValid(_ frame: NSRect) -> Bool {
    frame.origin.x.isFinite && frame.origin.y.isFinite
      && frame.width.isFinite && frame.height.isFinite
      && frame.width > 0 && frame.height > 0
      && frame.maxX.isFinite && frame.maxY.isFinite
  }

  /// Preserve both dimensions whenever translation alone can fit the frame.
  /// An unavailable display falls back to the nearest remaining visible frame.
  static func fitting(_ frame: NSRect, to screens: [NSRect]) -> NSRect {
    let screens = screens.filter(isValid)
    guard isValid(frame), let screen = screens.max(by: { lhs, rhs in
      let left = intersectionArea(frame, lhs), right = intersectionArea(frame, rhs)
      if left != right { return left < right }
      return distanceSquared(frame, lhs) > distanceSquared(frame, rhs)
    }) else { return frame }
    let size = NSSize(width: min(frame.width, screen.width), height: min(frame.height, screen.height))
    return NSRect(
      x: min(max(frame.minX, screen.minX), screen.maxX - size.width),
      y: min(max(frame.minY, screen.minY), screen.maxY - size.height),
      width: size.width, height: size.height
    )
  }

  private static func intersectionArea(_ lhs: NSRect, _ rhs: NSRect) -> CGFloat {
    let intersection = lhs.intersection(rhs)
    return intersection.isNull ? 0 : intersection.width * intersection.height
  }

  private static func distanceSquared(_ frame: NSRect, _ screen: NSRect) -> CGFloat {
    let x = frame.midX - min(max(frame.midX, screen.minX), screen.maxX)
    let y = frame.midY - min(max(frame.midY, screen.minY), screen.maxY)
    return x * x + y * y
  }
}
