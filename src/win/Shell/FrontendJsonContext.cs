using System.Text.Json.Serialization;
using System.Text.Json.Nodes;

namespace Viem.Windows.Shell;

// These startup records have a fixed schema. Generate serialization metadata
// rather than reflecting over the records during the first file open/handoff.
[JsonSerializable(typeof(RecoveryRecord))]
[JsonSerializable(typeof(OpenInvocation))]
[JsonSerializable(typeof(string[]))]
[JsonSerializable(typeof(JsonNode))]
[JsonSerializable(typeof(JsonObject))]
internal partial class FrontendJsonContext : JsonSerializerContext
{
    internal static FrontendJsonContext Indented { get; } = new(new() { WriteIndented = true });
}
