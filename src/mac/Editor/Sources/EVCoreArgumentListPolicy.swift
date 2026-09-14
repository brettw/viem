import CViemCore
import ViemAppShell

/// AppShell supplies file identity; Rust applies argument count and boundary
/// policy so every frontend selects the same destination.
@MainActor
enum EVCoreArgumentListPolicy {
    static func install() {
        EVArgumentListPolicy.resolve = { length, current, remembered, navigation in
            guard length >= 0, (current ?? 0) >= 0, (remembered ?? 0) >= 0
            else { return .failure(EVArgumentListError.invalidIndex) }
            let result = viem_argument_list_resolve(
                UInt64(length), current.map(UInt64.init) ?? UInt64.max,
                remembered.map(UInt64.init) ?? UInt64.max,
                navigation.target.rawValue, navigation.count
            )
            switch result.status {
            case UInt32(VIEM_ARGUMENT_RESOLVE_OK):
                guard let index = Int(exactly: result.index) else {
                    return .failure(EVArgumentListError.invalidIndex)
                }
                return .success(index)
            case UInt32(VIEM_ARGUMENT_RESOLVE_EMPTY): return .failure(EVArgumentListError.empty)
            case UInt32(VIEM_ARGUMENT_RESOLVE_BEFORE_FIRST): return .failure(EVArgumentListError.beforeFirst)
            case UInt32(VIEM_ARGUMENT_RESOLVE_AFTER_LAST): return .failure(EVArgumentListError.afterLast)
            default: return .failure(EVArgumentListError.invalidIndex)
            }
        }
    }
}
