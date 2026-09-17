import CViemCore

extension EVCoreViewSession {
    /// Core owns matching, snapshot validation, style resolution, and preview
    /// scrolling. The frontend supplies only opportunities for bounded work.
    @discardableResult
    func pollSearch() throws -> Bool {
        var changed: UInt8 = 0
        try checkedSearch(viem_core_view_poll_search(document.core, viewID, &changed),
                          operation: "Continue search highlighting")
        return changed != 0
    }

    func searchWorkPending() throws -> Bool {
        var pending: UInt8 = 0
        try checkedSearch(viem_core_view_search_work_pending(document.core, viewID, &pending),
                          operation: "Read pending search highlighting")
        return pending != 0
    }
}

private func checkedSearch(_ status: UInt32, operation: String) throws {
    guard status == UInt32(VIEM_STATUS_OK) else {
        throw EVCoreFrontendError.core(operation: operation, status: status)
    }
}
