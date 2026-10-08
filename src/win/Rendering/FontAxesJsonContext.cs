using System.Text.Json.Serialization;

namespace Viem.Windows.Rendering;

[JsonSerializable(typeof(Dictionary<string, float>), TypeInfoPropertyName = "Axes")]
internal partial class FontAxesJsonContext : JsonSerializerContext;
