import Foundation
import ViemBlockingTransport
import XCTest
@testable import ViemAppShell

@MainActor
final class EVCommandLineLaunchTests: XCTestCase {
    func testLaunchEnvelopePreservesPortableArgumentsAndConnectsOnlyAnAcceptanceReceipt() throws {
        let listener = try EVBlockingEditListener()
        let arguments = ["-O2", "+12", "--", "--日本語 file.md", "second.txt"]
        let expected = EVLaunchArguments(filenames: Array(arguments.suffix(2)), splitCount: 2,
            initialLine: 12, verticalSplits: true)
        let originalParser = EVLaunchArguments.parse
        defer { EVLaunchArguments.parse = originalParser }
        EVLaunchArguments.parse = { received in
            XCTAssertEqual(received, arguments)
            return expected
        }

        let parsed = try EVApplication.parse(EVInstanceLaunchRequest(
            arguments: ["--cli-launch", listener.endpoint] + arguments, workingDirectory: "/tmp"))

        XCTAssertEqual(parsed.arguments, expected)
        XCTAssertNil(parsed.completion)
        let receipt = try XCTUnwrap(parsed.launchCompletion)
        receipt.finish(exitCode: 0)
        XCTAssertEqual(try listener.waitForCompletion(startupTimeout: 1, completionTimeout: 1), 0)
    }

    func testInvalidArgumentsAndMalformedEnvelopeAreRejectedBeforeAcceptance() throws {
        let originalParser = EVLaunchArguments.parse
        defer { EVLaunchArguments.parse = originalParser }
        EVLaunchArguments.parse = { _ in throw EVLaunchArgumentError("Unsupported option") }
        let listener = try EVBlockingEditListener()
        XCTAssertThrowsError(try EVApplication.parse(EVInstanceLaunchRequest(
            arguments: ["--cli-launch", listener.endpoint, "--invalid"], workingDirectory: "/tmp"))) { error in
            XCTAssertTrue(error.localizedDescription.contains("Unsupported option"))
        }
        XCTAssertThrowsError(try EVApplication.parse(EVInstanceLaunchRequest(
            arguments: ["--cli-launch"], workingDirectory: "/tmp")))
    }

    func testOrdinaryLaunchHasNoAcceptanceOrDocumentCompletion() throws {
        let originalParser = EVLaunchArguments.parse
        defer { EVLaunchArguments.parse = originalParser }
        EVLaunchArguments.parse = { _ in EVLaunchArguments() }
        let parsed = try EVApplication.parse(EVInstanceLaunchRequest(arguments: [], workingDirectory: "/tmp"))
        XCTAssertNil(parsed.completion)
        XCTAssertNil(parsed.launchCompletion)
    }
}
