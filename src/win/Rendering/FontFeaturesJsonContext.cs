using System.Text.Json.Serialization;

namespace Viem.Windows.Rendering;

[JsonSerializable(typeof(Dictionary<string, uint>), TypeInfoPropertyName = "Features")]
internal partial class FontFeaturesJsonContext : JsonSerializerContext;
