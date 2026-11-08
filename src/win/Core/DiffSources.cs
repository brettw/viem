using System.Text;
using Viem.Windows.Interop;
using static Viem.Windows.Interop.Abi;
using static Viem.Windows.Interop.Native;

namespace Viem.Windows.Core;

internal sealed record DiffFile(string Path, string BasePath, uint Change)
{
    internal string Title => Change switch {
        VIEM_DIFF_FILE_RENAMED => $"{System.IO.Path.GetFileName(BasePath)} → {System.IO.Path.GetFileName(Path)}",
        VIEM_DIFF_FILE_ADDED => System.IO.Path.GetFileName(Path) + " (added)",
        VIEM_DIFF_FILE_DELETED => System.IO.Path.GetFileName(Path) + " (deleted)",
        VIEM_DIFF_FILE_UNSAVED => System.IO.Path.GetFileName(Path) + " (unsaved)",
        _ => System.IO.Path.GetFileName(Path)
    };
}

internal sealed record DiffBase(uint Kind, string Argument = "")
{
    public override string ToString() => Kind switch {
        VIEM_DIFF_BASE_LAST_SAVED => "Last Saved", VIEM_DIFF_BASE_UNSTAGED => "Unstaged",
        VIEM_DIFF_BASE_HEAD => "HEAD", VIEM_DIFF_BASE_BRANCH => Argument.Length == 0 ? "Branch…" : "Branch: " + Argument,
        _ => "File: " + Path.GetFileName(Argument)
    };
    public bool IsGit => Kind is VIEM_DIFF_BASE_UNSTAGED or VIEM_DIFF_BASE_HEAD or VIEM_DIFF_BASE_BRANCH;
    /// "BASE @ <version>" in the base side's status, in place of its mode.
    public string StatusTitle => Kind switch {
        VIEM_DIFF_BASE_LAST_SAVED => "BASE @ saved", VIEM_DIFF_BASE_UNSTAGED => "BASE @ index",
        VIEM_DIFF_BASE_HEAD => "BASE @ HEAD", VIEM_DIFF_BASE_BRANCH => "BASE @ " + Argument,
        _ => "BASE"
    };
}

// Blocking Git and filesystem requests own their inputs and result for the worker's lifetime.
internal static unsafe class DiffSources
{
    private delegate uint Request(ViemDiffBaseRequestV1* request, ulong* result);

    private static T Read<T>(DiffBase basis, string path, string? scope, string[] unsaved, CancellationToken token,
        Request run, Func<ulong, ViemDiffResultInfoV1, T> read)
    {
        token.ThrowIfCancellationRequested();
        using var arena = new NativeArena();
        var request = New<ViemDiffBaseRequestV1>();
        request.kind = basis.Kind;
        request.file_path = arena.Utf8(basis.Kind == VIEM_DIFF_BASE_FILE ? basis.Argument : path);
        request.scope_path = arena.Utf8(scope ?? "");
        request.branch = arena.Utf8(basis.Kind == VIEM_DIFF_BASE_BRANCH ? basis.Argument : "");
        request.unsaved_paths = arena.Copy<ViemUtf8Slice>(unsaved.Select(arena.Utf8).ToArray());
        request.unsaved_path_count = (ulong)unsaved.Length;
        ulong cancellation = 0;
        Check(viem_diff_cancellation_create(&cancellation), "Prepare diff request");
        request.cancellation = cancellation;
        try {
            ulong cancelHandle = cancellation;
            using var registration = token.Register(() => viem_diff_cancellation_cancel(cancelHandle));
            ulong result = 0;
            Check(run(&request, &result), "Read diff base");
            try {
                token.ThrowIfCancellationRequested();
                ulong resultHandle = result;
                var info = New<ViemDiffResultInfoV1>();
                Check(viem_diff_result_info(result, &info), "Read diff result");
                if (info.result != VIEM_DIFF_RESULT_OK)
                    throw new IOException(Encoding.UTF8.GetString(Copy((p, n, r) => viem_diff_result_copy_message(resultHandle, p, n, r))));
                return read(result, info);
            } finally { viem_diff_result_release(result); }
        } finally { viem_diff_cancellation_release(cancellation); }
    }

    internal static byte[] Load(DiffBase basis, string path, CancellationToken token) =>
        Read(basis, path, null, [], token, viem_diff_load_base,
            (result, _) => Copy((p, n, r) => viem_diff_result_copy_bytes(result, p, n, r)));

    internal static DiffFile[] Files(DiffBase basis, string path, string? scope, string[] unsaved, CancellationToken token) =>
        Read(basis, path, scope, unsaved, token, viem_diff_list_files, Items);

    internal static string[] Branches(string path, CancellationToken token) =>
        Read(new(VIEM_DIFF_BASE_HEAD), path, null, [], token, viem_diff_list_branches, Items).Select(item => item.Path).ToArray();

    private static DiffFile[] Items(ulong result, ViemDiffResultInfoV1 info)
    {
        var files = new DiffFile[checked((int)info.item_count)];
        for (int index = 0; index < files.Length; ++index) {
            var item = New<ViemDiffListItemV1>();
            ulong length = 0;
            uint status = viem_diff_result_copy_item(result, (ulong)index, &item, null, 0, &length);
            if (status != VIEM_STATUS_BUFFER_TOO_SMALL) Check(status, "Read diff file");
            var bytes = new byte[checked((int)length)];
            fixed (byte* p = bytes) Check(viem_diff_result_copy_item(result, (ulong)index, &item, p, length, &length), "Read diff file");
            string path = Text(bytes, 0, item.path_bytes);
            files[index] = new(path, item.base_path_bytes == 0 ? path : Text(bytes, item.path_bytes, item.base_path_bytes), item.change);
        }
        return files;
    }
}
