import CViemCore

extension EVCoreViewSession {
    func nativeActionAvailable(_ action: UInt32) -> Bool {
        var mask: UInt64 = 0
        return viem_core_view_native_actions(document.core, viewID, &mask) == VIEM_STATUS_OK && mask & (1 << action) != 0
    }
    @discardableResult
    func nativeAction(_ action: UInt32, origin: UInt32 = UInt32(VIEM_SELECTION_ORIGIN_KEY), returningTo mode: UInt32 = 0) throws -> ViemCoreOutcomeV1 {
        try performHostEffectTurn("Native action") { outcome, effects in
            viem_core_view_native_action(document.core, viewID, action, origin, mode, outcome, effects)
        }
    }
}
