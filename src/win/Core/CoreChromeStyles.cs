using Viem.Windows.Interop;
using static Viem.Windows.Interop.Native;
using static Viem.Windows.Interop.Abi;

namespace Viem.Windows.Core;

internal sealed unsafe partial class CoreDocument
{
    // Chrome can resolve theme defaults before a renderer or editor view exists.
    // This query does not shape text or enumerate installed fonts.
    internal StyleSheet ChromeStyleSheet()
    {
        var info = New<ViemStyleSheetInfoV1>(); Check(viem_core_style_sheet_info(Handle, &info), "Read chrome styles");
        var definitions = new ViemStyleDefinitionV1[checked((int)info.definition_count)];
        var properties = new ViemStylePropertyV1[checked((int)info.property_count)];
        var items = new ViemStyleValueItemV1[checked((int)info.value_item_count)];
        var dependencies = new ViemStyleDependencyV1[checked((int)info.dependency_count)];
        var strings = new byte[checked((int)info.string_bytes)]; var identity = info.identity;
        fixed (ViemStyleDefinitionV1* d = definitions) fixed (ViemStylePropertyV1* p = properties)
        fixed (ViemStyleValueItemV1* v = items) fixed (ViemStyleDependencyV1* r = dependencies) fixed (byte* s = strings)
            Check(viem_core_copy_style_sheet(Handle, &identity, d, (ulong)definitions.Length, p, (ulong)properties.Length,
                v, (ulong)items.Length, r, (ulong)dependencies.Length, s, (ulong)strings.Length, &info), "Copy chrome styles");
        return new(identity, definitions.Select(d => new StyleDefinition(d, Text(strings, d.stable_id), Text(strings, d.display_name),
            Text(strings, d.parent_id), Text(strings, d.next_style_id), properties.Skip(checked((int)d.first_property))
                .Take(checked((int)d.property_count)).ToDictionary(p => p.property))).ToArray(), strings, items);
    }
}
