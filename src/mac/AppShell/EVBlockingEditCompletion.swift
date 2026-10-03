import ViemBlockingTransport

/// Kept in the shell's public vocabulary while the command-line helper shares
/// only the Foundation/POSIX transport, without linking AppKit or the Rust core.
public typealias EVBlockingEditCompletion = ViemBlockingTransport.EVBlockingEditCompletion
