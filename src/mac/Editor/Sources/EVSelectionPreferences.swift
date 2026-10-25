import CViemCore
import Foundation
import ViemAppShell

/// Native and Vim selection options are application-wide. Core documents retain their
/// own interpreters, so the host distributes validated values without sending
/// synthetic commands through another document's current mode or composition.
@MainActor
enum EVSelectionPreferences {
    private final class WeakDocument {
        weak var value: EVCoreDocumentBackend?
        init(_ value: EVCoreDocumentBackend) { self.value = value }
    }
    private static var documents: [WeakDocument] = []
    private static var values: [URL: [UInt32: String]] = [:]
    /// The Settings default last distributed per profile. Unrelated
    /// configuration writes leave `:set autoselect` overrides in place.
    private static var configuredAutoselect: [URL: Bool] = [:]
    private static var configurationObserver: NSObjectProtocol?
    private static let options = [UInt32(VIEM_EX_OPTION_KEYMODEL), UInt32(VIEM_EX_OPTION_SELECTMODE), UInt32(VIEM_EX_OPTION_AUTOSELECT)]

    /// Applies the Settings default before startup.viem can override it.
    static func seed(_ document: EVCoreDocumentBackend) throws {
        observeConfiguration()
        let key = document.configuration.directory.standardizedFileURL
        let enabled = configuredAutoselect[key] ?? document.configuration.autoselect
        configuredAutoselect[key] = enabled
        try set(enabled ? "1" : "0", option: UInt32(VIEM_EX_OPTION_AUTOSELECT), on: document)
    }

    /// A changed Settings default replaces the live value in every document of
    /// its profile, including one chosen with `:set` or startup.viem.
    static func configure(autoselect enabled: Bool, directory: URL) {
        let key = directory.standardizedFileURL
        // Profiles without a seeded document read the configuration when one opens.
        guard let previous = configuredAutoselect[key], previous != enabled else { return }
        configuredAutoselect[key] = enabled
        let option = UInt32(VIEM_EX_OPTION_AUTOSELECT)
        let value = enabled ? "1" : "0"
        guard values[key] != nil, values[key]?[option] != value else { return }
        values[key]?[option] = value
        documents.removeAll { $0.value == nil }
        for target in documents.compactMap(\.value) where target.configuration.directory.standardizedFileURL == key {
            // A closing core must not keep the remaining documents stale.
            try? set(value, option: option, on: target)
        }
    }

    private static func observeConfiguration() {
        guard configurationObserver == nil else { return }
        configurationObserver = NotificationCenter.default.addObserver(
            forName: .viemConfigurationDidChange, object: nil, queue: .main
        ) { notification in
            MainActor.assumeIsolated {
                guard let configuration = notification.object as? EVConfigurationStore else { return }
                configure(autoselect: configuration.autoselect, directory: configuration.directory)
            }
        }
    }

    static func attach(_ document: EVCoreDocumentBackend) throws {
        documents.removeAll { $0.value == nil }
        let key = document.configuration.directory.standardizedFileURL
        var settings = values[key] ?? [:]
        for option in options {
            if let value = settings[option] { try set(value, option: option, on: document) }
            else { settings[option] = try read(option, from: document) }
        }
        values[key] = settings
        if !documents.contains(where: { $0.value === document }) { documents.append(WeakDocument(document)) }
    }

    static func apply(_ effects: [EVExOptionEffect], from document: EVCoreDocumentBackend) throws {
        let key = document.configuration.directory.standardizedFileURL
        for effect in effects where options.contains(effect.name) {
            let value: String
            switch effect.value {
            case .string(let text): value = text
            case .boolean(let enabled) where effect.name == UInt32(VIEM_EX_OPTION_AUTOSELECT): value = enabled ? "1" : "0"
            default: continue
            }
            guard values[key]?[effect.name] != value else { continue }
            // The issuing core has already validated this option transaction.
            values[key, default: [:]][effect.name] = value
            documents.removeAll { $0.value == nil }
            for target in documents.compactMap(\.value)
                where target !== document && target.configuration.directory.standardizedFileURL == key {
                try set(value, option: effect.name, on: target)
            }
        }
    }

    static func setAutoselect(_ enabled: Bool, on document: EVCoreDocumentBackend) throws {
        let option = UInt32(VIEM_EX_OPTION_AUTOSELECT)
        try set(enabled ? "1" : "0", option: option, on: document)
        try apply([EVExOptionEffect(name: option, value: .boolean(enabled))], from: document)
    }

    static func read(_ option: UInt32, from document: EVCoreDocumentBackend) throws -> String {
        var required: UInt64 = 0
        let status = viem_core_copy_selection_option(document.core, option, nil, 0, &required)
        if status != UInt32(VIEM_STATUS_BUFFER_TOO_SMALL) { try check(status) }
        var bytes = [UInt8](repeating: 0, count: Int(required))
        try bytes.withUnsafeMutableBufferPointer {
            try check(viem_core_copy_selection_option(document.core, option, $0.baseAddress, UInt64($0.count), &required))
        }
        return String(decoding: bytes, as: UTF8.self)
    }

    private static func set(_ value: String, option: UInt32, on document: EVCoreDocumentBackend) throws {
        try Array(value.utf8).withUnsafeBufferPointer {
            try check(viem_core_set_selection_option(document.core, option, $0.baseAddress, UInt64($0.count)))
        }
    }

    private static func check(_ status: UInt32) throws {
        guard status == UInt32(VIEM_STATUS_OK) else {
            throw EVCoreFrontendError.core(operation: "Synchronize selection options", status: status)
        }
    }
}
