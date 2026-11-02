using System.Text.Json.Nodes;
using System.Text.Json.Serialization;

namespace Viem.Windows.Core;

[JsonSourceGenerationOptions(PropertyNameCaseInsensitive = true)]
[JsonSerializable(typeof(ReferenceContext))]
[JsonSerializable(typeof(ReferenceChoice[]))]
[JsonSerializable(typeof(JsonObject))]
internal partial class ReferenceJsonContext : JsonSerializerContext;
