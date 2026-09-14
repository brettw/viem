import Foundation

/// Parsed portable startup policy. Native code resolves filenames against the
/// launch working directory and supplies files to the window's argument list.
public struct EVLaunchArguments: Decodable, Equatable, Sendable {
    public var filenames: [String]
    /// nil opens one pane; zero opens a stacked pane for each filename.
    public var splitCount: Int?
    /// One-based first-file line; UInt64.max requests the last line.
    public var initialLine: UInt64?

    public init(filenames: [String] = [], splitCount: Int? = nil, initialLine: UInt64? = nil) {
        self.filenames = filenames
        self.splitCount = splitCount
        self.initialLine = initialLine
    }

    @MainActor public static var parse: ([String]) throws -> EVLaunchArguments = { arguments in
        guard arguments.isEmpty else {
            throw EVLaunchArgumentError("The command-line parser has not been installed.")
        }
        return EVLaunchArguments()
    }
}

public struct EVLaunchArgumentError: LocalizedError, Equatable {
    public let message: String
    public init(_ message: String) { self.message = message }
    public var errorDescription: String? { message }
}
