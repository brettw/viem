# Native dependency licenses

This inventory covers all 19 distinct packages in `src/win/packages.lock.json`,
the Windows SDK .NET reference package selected by `WindowsSdkPackageVersion`,
and .NET 10.0.13 runtime/host packs corresponding to the locked NativeAOT toolchain.
[Machine-readable provenance and SHA-256 hashes](inventory.json) record each file's
origin. Paths in that inventory are relative to this directory. Package license
and notice files retain their original bytes; explicitly labeled text extractions
are accompanied by the original HTML or RTF.

`Package.swift` declares no external Swift package dependencies. The macOS
frontend uses Apple frameworks and the operating system's Swift runtime;
the build scripts do not bundle a separate copy of those platform components.

## Windows packages

The NuGet links identify the exact package versions. The license column describes
the delivered package, including its notices, rather than applying the source
repository's license to every binary.

| Package | Version | License and notices | Scope |
| --- | --- | --- | --- |
| [Microsoft.DotNet.ILCompiler](https://www.nuget.org/packages/Microsoft.DotNet.ILCompiler/10.0.13) | 10.0.13 | MIT: [LICENSE.TXT](Microsoft.DotNet.ILCompiler-10.0.13/LICENSE.TXT); [THIRD-PARTY-NOTICES.TXT](Microsoft.DotNet.ILCompiler-10.0.13/THIRD-PARTY-NOTICES.TXT) | locked dependency |
| [Microsoft.Graphics.Win2D](https://www.nuget.org/packages/Microsoft.Graphics.Win2D/1.3.2) | 1.3.2 | Microsoft Win2D binary terms; MIT source: [LICENSE-binary.html](Microsoft.Graphics.Win2D-1.3.2/LICENSE-binary.html); [LICENSE-binary.txt](Microsoft.Graphics.Win2D-1.3.2/LICENSE-binary.txt); [LICENSE.txt](Microsoft.Graphics.Win2D-1.3.2/LICENSE.txt) | locked dependency |
| [Microsoft.NET.ILLink.Tasks](https://www.nuget.org/packages/Microsoft.NET.ILLink.Tasks/10.0.13) | 10.0.13 | MIT: [LICENSE.TXT](Microsoft.NET.ILLink.Tasks-10.0.13/LICENSE.TXT); [THIRD-PARTY-NOTICES.TXT](Microsoft.NET.ILLink.Tasks-10.0.13/THIRD-PARTY-NOTICES.TXT) | locked dependency |
| [Microsoft.NETCore.App.Host.win-x64](https://www.nuget.org/packages/Microsoft.NETCore.App.Host.win-x64/10.0.13) | 10.0.13 | MIT: [LICENSE.TXT](Microsoft.NETCore.App.Host.win-x64-10.0.13/LICENSE.TXT); [THIRD-PARTY-NOTICES.TXT](Microsoft.NETCore.App.Host.win-x64-10.0.13/THIRD-PARTY-NOTICES.TXT) | SDK-provided runtime/host pack (10.0.13 reference) |
| [Microsoft.NETCore.App.Runtime.NativeAOT.win-x64](https://www.nuget.org/packages/Microsoft.NETCore.App.Runtime.NativeAOT.win-x64/10.0.13) | 10.0.13 | MIT: [LICENSE.TXT](Microsoft.NETCore.App.Runtime.NativeAOT.win-x64-10.0.13/LICENSE.TXT); [THIRD-PARTY-NOTICES.TXT](Microsoft.NETCore.App.Runtime.NativeAOT.win-x64-10.0.13/THIRD-PARTY-NOTICES.TXT) | SDK-provided runtime/host pack (10.0.13 reference) |
| [Microsoft.NETCore.App.Runtime.win-x64](https://www.nuget.org/packages/Microsoft.NETCore.App.Runtime.win-x64/10.0.13) | 10.0.13 | MIT plus Windows .NET distribution terms: [LICENSE-Windows.html](Microsoft.NETCore.App.Runtime.win-x64-10.0.13/LICENSE-Windows.html); [LICENSE-Windows.txt](Microsoft.NETCore.App.Runtime.win-x64-10.0.13/LICENSE-Windows.txt); [LICENSE.TXT](Microsoft.NETCore.App.Runtime.win-x64-10.0.13/LICENSE.TXT); [THIRD-PARTY-NOTICES.TXT](Microsoft.NETCore.App.Runtime.win-x64-10.0.13/THIRD-PARTY-NOTICES.TXT) | SDK-provided runtime/host pack (10.0.13 reference) |
| [Microsoft.Web.WebView2](https://www.nuget.org/packages/Microsoft.Web.WebView2/1.0.3719.77) | 1.0.3719.77 | BSD-3-Clause (SDK/loader): [LICENSE.txt](Microsoft.Web.WebView2-1.0.3719.77/LICENSE.txt); [NOTICE.txt](Microsoft.Web.WebView2-1.0.3719.77/NOTICE.txt) | locked dependency |
| [Microsoft.Windows.AI.MachineLearning](https://www.nuget.org/packages/Microsoft.Windows.AI.MachineLearning/2.1.1) | 2.1.1 | Microsoft Software License Terms: [ThirdPartyNotices.txt](Microsoft.Windows.AI.MachineLearning-2.1.1/ThirdPartyNotices.txt); [license.txt](Microsoft.Windows.AI.MachineLearning-2.1.1/license.txt) | locked dependency |
| [Microsoft.Windows.SDK.BuildTools](https://www.nuget.org/packages/Microsoft.Windows.SDK.BuildTools/10.0.26100.4654) | 10.0.26100.4654 | Microsoft Software License Terms: [LICENSE.rtf](Microsoft.Windows.SDK.BuildTools-10.0.26100.4654/LICENSE.rtf); [LICENSE.txt](Microsoft.Windows.SDK.BuildTools-10.0.26100.4654/LICENSE.txt) | locked dependency |
| [Microsoft.Windows.SDK.BuildTools.MSIX](https://www.nuget.org/packages/Microsoft.Windows.SDK.BuildTools.MSIX/1.7.251221100) | 1.7.251221100 | Microsoft Software License Terms: [NOTICE.txt](Microsoft.Windows.SDK.BuildTools.MSIX-1.7.251221100/NOTICE.txt); [sdk_license.txt](Microsoft.Windows.SDK.BuildTools.MSIX-1.7.251221100/sdk_license.txt) | locked dependency |
| [Microsoft.Windows.SDK.NET.Ref](https://www.nuget.org/packages/Microsoft.Windows.SDK.NET.Ref/10.0.26100.87) | 10.0.26100.87 | Microsoft Software License Terms: [LICENSE.rtf](Microsoft.Windows.SDK.NET.Ref-10.0.26100.87/LICENSE.rtf); [LICENSE.txt](Microsoft.Windows.SDK.NET.Ref-10.0.26100.87/LICENSE.txt) | SDK reference package |
| [Microsoft.WindowsAppSDK](https://www.nuget.org/packages/Microsoft.WindowsAppSDK/2.1.3) | 2.1.3 | Microsoft Software License Terms: [NOTICE.txt](Microsoft.WindowsAppSDK-2.1.3/NOTICE.txt); [license.txt](Microsoft.WindowsAppSDK-2.1.3/license.txt) | locked dependency |
| [Microsoft.WindowsAppSDK.AI](https://www.nuget.org/packages/Microsoft.WindowsAppSDK.AI/2.1.10) | 2.1.10 | Microsoft Software License Terms: [license.txt](Microsoft.WindowsAppSDK.AI-2.1.10/license.txt) | locked dependency |
| [Microsoft.WindowsAppSDK.Base](https://www.nuget.org/packages/Microsoft.WindowsAppSDK.Base/2.0.4) | 2.0.4 | Microsoft Software License Terms: [NOTICE.txt](Microsoft.WindowsAppSDK.Base-2.0.4/NOTICE.txt); [license.txt](Microsoft.WindowsAppSDK.Base-2.0.4/license.txt) | locked dependency |
| [Microsoft.WindowsAppSDK.DWrite](https://www.nuget.org/packages/Microsoft.WindowsAppSDK.DWrite/2.1.0) | 2.1.0 | Microsoft Software License Terms: [license.txt](Microsoft.WindowsAppSDK.DWrite-2.1.0/license.txt) | locked dependency |
| [Microsoft.WindowsAppSDK.Foundation](https://www.nuget.org/packages/Microsoft.WindowsAppSDK.Foundation/2.0.21) | 2.0.21 | Microsoft Software License Terms: [license.txt](Microsoft.WindowsAppSDK.Foundation-2.0.21/license.txt) | locked dependency |
| [Microsoft.WindowsAppSDK.InteractiveExperiences](https://www.nuget.org/packages/Microsoft.WindowsAppSDK.InteractiveExperiences/2.0.13) | 2.0.13 | Microsoft Software License Terms: [license.txt](Microsoft.WindowsAppSDK.InteractiveExperiences-2.0.13/license.txt) | locked dependency |
| [Microsoft.WindowsAppSDK.ML](https://www.nuget.org/packages/Microsoft.WindowsAppSDK.ML/2.1.1) | 2.1.1 | Microsoft Software License Terms: [ThirdPartyNotices.txt](Microsoft.WindowsAppSDK.ML-2.1.1/ThirdPartyNotices.txt); [license.txt](Microsoft.WindowsAppSDK.ML-2.1.1/license.txt) | locked dependency |
| [Microsoft.WindowsAppSDK.Runtime](https://www.nuget.org/packages/Microsoft.WindowsAppSDK.Runtime/2.1.3) | 2.1.3 | Microsoft Software License Terms: [NOTICE.txt](Microsoft.WindowsAppSDK.Runtime-2.1.3/NOTICE.txt); [license.txt](Microsoft.WindowsAppSDK.Runtime-2.1.3/license.txt) | locked dependency |
| [Microsoft.WindowsAppSDK.Widgets](https://www.nuget.org/packages/Microsoft.WindowsAppSDK.Widgets/2.0.5) | 2.0.5 | Microsoft Software License Terms: [license.txt](Microsoft.WindowsAppSDK.Widgets-2.0.5/license.txt) | locked dependency |
| [Microsoft.WindowsAppSDK.WinUI](https://www.nuget.org/packages/Microsoft.WindowsAppSDK.WinUI/2.1.0) | 2.1.0 | Microsoft Windows App SDK Engineering Preview terms: [NOTICE.txt](Microsoft.WindowsAppSDK.WinUI-2.1.0/NOTICE.txt); [license.txt](Microsoft.WindowsAppSDK.WinUI-2.1.0/license.txt); [tools/NOTICE.txt](Microsoft.WindowsAppSDK.WinUI-2.1.0/tools/NOTICE.txt) | locked dependency |
| [runtime.win-x64.Microsoft.DotNet.ILCompiler](https://www.nuget.org/packages/runtime.win-x64.Microsoft.DotNet.ILCompiler/10.0.13) | 10.0.13 | MIT: [LICENSE.TXT](runtime.win-x64.Microsoft.DotNet.ILCompiler-10.0.13/LICENSE.TXT); [THIRD-PARTY-NOTICES.TXT](runtime.win-x64.Microsoft.DotNet.ILCompiler-10.0.13/THIRD-PARTY-NOTICES.TXT) | locked dependency |
| [System.Numerics.Tensors](https://www.nuget.org/packages/System.Numerics.Tensors/9.0.0) | 9.0.0 | MIT: [LICENSE.TXT](System.Numerics.Tensors-9.0.0/LICENSE.TXT); [THIRD-PARTY-NOTICES.TXT](System.Numerics.Tensors-9.0.0/THIRD-PARTY-NOTICES.TXT) | locked dependency |

## Source and distribution distinctions

- **Win2D:** the package's declared binary license URL now redirects to unrelated
  web-development documentation. `LICENSE-binary.html` preserves the original
  Microsoft page from the [January 13, 2025 archive snapshot](https://web.archive.org/web/20250113180923id_/https://www.microsoft.com/web/webpi/eula/eula_win2d_10012014.htm),
  with a readable text extraction beside it. `LICENSE.txt` is the separate MIT
  source license from public commit
  [`65e90b2`](https://github.com/microsoft/Win2D/commit/65e90b29055de64b02e7f2a3d3f042b7fa36326c),
  which records Win2D-WinUI 1.3.2 in its changelog. The package's
  `Win2d.githash.txt` names `feefabe2d8ef80025030c1dd7c9d3d71e2462cd3`,
  which was not available from the public repository. The source license does
  not replace the package's declared binary terms.
- **Windows SDK BuildTools and SDK.NET.Ref:** these packages contain a license
  URL instead of a license file. Their original RTF copies come from the
  [Microsoft download linked by that URL](https://download.microsoft.com/download/0/F/F/0FF2B061-47DD-4F55-89B6-FD1D8C44F14D/sdk_license.rtf).
  Readable text extractions accompany them. Build tools are development inputs;
  their inclusion here does not mean the entire SDK is redistributed with Viem.
- **Windows App SDK:** preserve each component's terms separately. In particular,
  `Microsoft.WindowsAppSDK.WinUI` 2.1.0 actually includes **Engineering Preview**
  terms, with a live-operating-environment restriction and time-sensitive terms.
  The ordinary terms in the umbrella package do not erase that discrepancy;
  resolve it with the upstream package publisher before relying on production
  redistribution of that component.
- **.NET and NativeAOT:** the ILCompiler, linker, host, and NativeAOT runtime
  packages declare MIT and include full .NET third-party notices. The three
  compiler/linker packages omit their MIT text, so it is copied from the exact
  `dotnet/dotnet` commit recorded in their NuGet metadata. NativeAOT links runtime
  code into both the editor and blocking helper; these notices cover that input
  as well as the compiler. The runtime and host packs listed here use 10.0.13 to
  match the locked NativeAOT version. SDK-selected framework packs and the helper
  are not independently locked in this repository, so a build using a different
  .NET servicing version must refresh their license and notice copies from its
  actual restore graph. The regular Windows runtime pack and Microsoft's
  additional Windows distribution terms are included for managed/self-contained
  variants. Microsoft's [distribution guidance](https://github.com/dotnet/core/blob/main/license-information.md)
  and [Windows component distinctions](https://github.com/dotnet/core/blob/main/license-information-windows.md)
  explain the difference between MIT package/source licenses and the Windows
  runtime product terms.
- **WebView2:** these are the SDK/loader package's BSD terms and bundled notice.
  A separately installed or redistributed Edge WebView2 Runtime is a distinct
  Microsoft product; its terms are not replaced by the SDK's license. The
  presence of an SDK dependency alone does not say a browser runtime is bundled.

## Updating

When the Windows lockfile or SDK version changes, refresh the affected package
from its exact NuGet archive, retain every license and third-party notice, and
update this table and `inventory.json`. Compare the final published file inventory
with its actual restore graph when changing the SDK, architecture, or deployment
mode. Preserve component-specific notices and original license bytes.
