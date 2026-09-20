import CViemCore

enum EVSelectionModes {
    static func isNative(_ mode: UInt32) -> Bool {
        [UInt32(VIEM_MODE_SELECTION_CHARACTER), UInt32(VIEM_MODE_SELECTION_LINE), UInt32(VIEM_MODE_SELECTION_BLOCK)].contains(mode)
    }

    static func isTextSelection(_ mode: UInt32) -> Bool {
        isNative(mode) || [UInt32(VIEM_MODE_SELECT_CHARACTER), UInt32(VIEM_MODE_SELECT_LINE), UInt32(VIEM_MODE_SELECT_BLOCK)]
            .contains(mode)
    }

    static func hasSelection(_ mode: UInt32) -> Bool {
        isTextSelection(mode) || [UInt32(VIEM_MODE_VISUAL_CHARACTER), UInt32(VIEM_MODE_VISUAL_LINE), UInt32(VIEM_MODE_VISUAL_BLOCK)]
            .contains(mode)
    }

    static func isLinearSelect(_ mode: UInt32) -> Bool {
        [UInt32(VIEM_MODE_SELECT_CHARACTER), UInt32(VIEM_MODE_SELECT_LINE), UInt32(VIEM_MODE_SELECTION_CHARACTER), UInt32(VIEM_MODE_SELECTION_LINE)].contains(mode)
    }

    static func hasInsertionCaret(_ mode: UInt32) -> Bool {
        mode == UInt32(VIEM_MODE_INSERT) || isTextSelection(mode)
    }

    static func canCompose(_ mode: UInt32) -> Bool {
        mode == UInt32(VIEM_MODE_INSERT) || mode == UInt32(VIEM_MODE_REPLACE) || isLinearSelect(mode)
    }
}
