#if DEBUG
using System.Text.Json.Serialization;

namespace Viem.Windows.Diagnostics;

// NativeAOT diagnostics exercise the same native code as the shipped build.
// Keep report schemas explicit rather than relying on runtime reflection.
internal sealed record NativeTestSuccess(bool passed, int count, List<string> checks, bool nativeAot);
internal sealed record NativeTestFailure(bool passed, string error);
internal sealed record InputPerformanceStep(int index, string phase, double milliseconds,
    double gcPauseMilliseconds, long allocatedBytes, long shapedCharacters, int drawingBuilds, float top, ulong revision);
internal sealed record InputPerformanceStatistics(int count, double mean, double p50, double p95, double max);
internal sealed record InputPerformanceJobs(int Started, int Installed, int Discarded);
internal sealed record InputPerformanceReport(bool passed, string scenario, int count, List<InputPerformanceStep> steps,
    long shapedCharacters, long fontMetadataReads, int drawingCacheBuilds, float width, float height, float dpi,
    int peakRenderResources, int interval, long glyphBoundsQueries, long glyphBoundsHits, bool backgroundLayout,
    long backgroundShapedCharacters, InputPerformanceJobs backgroundJobs, int rows, int clusters, ulong hardLines,
    Dictionary<string, InputPerformanceStatistics> milliseconds);

[JsonSourceGenerationOptions(WriteIndented = true)]
[JsonSerializable(typeof(NativeTestSuccess))]
[JsonSerializable(typeof(NativeTestFailure))]
[JsonSerializable(typeof(InputPerformanceReport))]
internal partial class NativeTestJsonContext : JsonSerializerContext;
#endif
