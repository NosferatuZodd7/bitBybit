using System.Runtime.Versioning;
using BitByBit.Core.QuickDiag;
using Microsoft.Win32;
using Windows.Win32;
using Windows.Win32.Devices.Display;
using Windows.Win32.Foundation;

namespace BitByBit.Platform.Windows.QuickDiag;

/// <summary>
/// Read-only active display paths via the CCD API (QueryDisplayConfig / DisplayConfigGetDeviceInfo): monitor name,
/// output technology (HDMI/DisplayPort/...), resolution and refresh rate; EDID read from the monitor's devnode registry
/// key (readable without elevation) to see whether the monitor announces audio.
/// </summary>
[SupportedOSPlatform("windows6.1")]
internal static class DisplayQuick
{
    public static IReadOnlyList<MonitorSummary> Enumerate(List<CollectionIssue> issues)
    {
        var flags = QUERY_DISPLAY_CONFIG_FLAGS.QDC_ONLY_ACTIVE_PATHS;
        DISPLAYCONFIG_PATH_INFO[] paths;
        DISPLAYCONFIG_MODE_INFO[] modes;
        var attempt = 0;
        while (true)
        {
            var err = PInvoke.GetDisplayConfigBufferSizes(flags, out var pathCount, out var modeCount);
            if (err != WIN32_ERROR.ERROR_SUCCESS)
            {
                issues.Add(new CollectionIssue("display", $"GetDisplayConfigBufferSizes failed: {err}"));
                return [];
            }

            paths = new DISPLAYCONFIG_PATH_INFO[pathCount];
            modes = new DISPLAYCONFIG_MODE_INFO[modeCount];
            err = PInvoke.QueryDisplayConfig(flags, ref pathCount, paths.AsSpan(), ref modeCount, modes.AsSpan());

            if (err == WIN32_ERROR.ERROR_INSUFFICIENT_BUFFER && ++attempt < 5)
            {
                continue;
            }

            if (err != WIN32_ERROR.ERROR_SUCCESS)
            {
                issues.Add(new CollectionIssue("display", $"QueryDisplayConfig failed: {err}"));
                return [];
            }

            Array.Resize(ref paths, (int)pathCount);
            Array.Resize(ref modes, (int)modeCount);
            break;
        }

        var result = new List<MonitorSummary>();
        foreach (var path in paths)
        {
            var target = GetTargetName(path.targetInfo.adapterId, path.targetInfo.id);
            var sourceName = GetSourceName(path.sourceInfo.adapterId, path.sourceInfo.id);
            int? width = null, height = null;
            var modeIdx = path.sourceInfo.Anonymous.modeInfoIdx;
            if (modeIdx != 0xFFFFFFFF && modeIdx < modes.Length &&
                modes[modeIdx].infoType == DISPLAYCONFIG_MODE_INFO_TYPE.DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE)
            {
                width = (int)modes[modeIdx].Anonymous.sourceMode.width;
                height = (int)modes[modeIdx].Anonymous.sourceMode.height;
            }

            double? refresh = path.targetInfo.refreshRate.Denominator != 0
                ? (double)path.targetInfo.refreshRate.Numerator / path.targetInfo.refreshRate.Denominator
                : null;
            var tech = unchecked((int)path.targetInfo.outputTechnology);
            var instanceId = target?.DevicePath is { Length: > 0 } dp ? DevicePathToInstanceId(dp) : null;
            EdidQuickInfo? edid = null;
            if (instanceId is not null)
            {
                edid = ReadEdid(instanceId, issues);
            }

            result.Add(new MonitorSummary
            {
                FriendlyName = string.IsNullOrWhiteSpace(target?.FriendlyName) ? edid?.MonitorName : target!.FriendlyName,
                OutputTechnology = tech,
                OutputTechnologyName = OutputTechnologyName(tech),
                IsActive = true,
                SourceName = sourceName,
                Width = width,
                Height = height,
                RefreshHz = refresh is { } r ? Math.Round(r, 2) : null,
                MonitorInstanceId = instanceId,
                EdidManufacturer = edid?.ManufacturerId,
                EdidProductCode = edid?.ProductCode,
                EdidMonitorName = edid?.MonitorName,
                EdidAnnouncesAudio = edid?.AnnouncesAudio,
                EdidAudioFormats = edid is { AudioFormats.Count: > 0 } ? string.Join(", ", edid.AudioFormats) : (edid?.BasicAudio == true ? "basic audio" : null),
            });
        }

        return result;
    }

    public static string OutputTechnologyName(int value) => value switch
    {
        -1 => "Other",
        0 => "VGA (HD15)",
        1 => "S-Video",
        2 => "Composite",
        3 => "Component",
        4 => "DVI",
        5 => "HDMI",
        6 => "LVDS",
        8 => "D-JPN",
        9 => "SDI",
        10 => "DisplayPort",
        11 => "DisplayPort (embedded)",
        12 => "UDI",
        13 => "UDI (embedded)",
        14 => "SDTV dongle",
        15 => "Miracast",
        16 => "Indirect (wired)",
        17 => "Indirect (virtual)",
        18 => "DisplayPort (USB tunnel)",
        unchecked((int)0x80000000) => "Internal",
        _ => $"Unknown ({value})",
    };

    /// <summary>\\?\DISPLAY#DELA0B1#5&amp;2b3f&amp;0&amp;UID4352#{e6f07b5f-...} → DISPLAY\DELA0B1\5&amp;2b3f&amp;0&amp;UID4352</summary>
    public static string? DevicePathToInstanceId(string devicePath)
    {
        var p = devicePath;
        if (p.StartsWith(@"\\?\", StringComparison.Ordinal))
        {
            p = p[4..];
        }

        var guidStart = p.LastIndexOf("#{", StringComparison.Ordinal);
        if (guidStart > 0)
        {
            p = p[..guidStart];
        }

        return p.Length == 0 ? null : p.Replace('#', '\\');
    }

    private static EdidQuickInfo? ReadEdid(string instanceId, List<CollectionIssue> issues)
    {
        try
        {
            using var key = Registry.LocalMachine.OpenSubKey($@"SYSTEM\CurrentControlSet\Enum\{instanceId}\Device Parameters");
            return key?.GetValue("EDID") is byte[] bytes ? EdidQuickParser.TryParse(bytes) : null;
        }
        catch (Exception ex) when (ex is System.Security.SecurityException or UnauthorizedAccessException or IOException)
        {
            issues.Add(new CollectionIssue("display.edid", $"EDID not readable for {instanceId}: {ex.Message}"));
            return null;
        }
    }

    private sealed record TargetName(string FriendlyName, string DevicePath);

    private static unsafe TargetName? GetTargetName(LUID adapter, uint id)
    {
        var request = new DISPLAYCONFIG_TARGET_DEVICE_NAME();
        request.header.type = DISPLAYCONFIG_DEVICE_INFO_TYPE.DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
        request.header.size = (uint)sizeof(DISPLAYCONFIG_TARGET_DEVICE_NAME);
        request.header.adapterId = adapter;
        request.header.id = id;
        if (PInvoke.DisplayConfigGetDeviceInfo(&request.header) != 0)
        {
            return null;
        }

        return new TargetName(request.monitorFriendlyDeviceName.ToString().TrimEnd('\0'), request.monitorDevicePath.ToString().TrimEnd('\0'));
    }

    private static unsafe string? GetSourceName(LUID adapter, uint id)
    {
        var request = new DISPLAYCONFIG_SOURCE_DEVICE_NAME();
        request.header.type = DISPLAYCONFIG_DEVICE_INFO_TYPE.DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
        request.header.size = (uint)sizeof(DISPLAYCONFIG_SOURCE_DEVICE_NAME);
        request.header.adapterId = adapter;
        request.header.id = id;
        return PInvoke.DisplayConfigGetDeviceInfo(&request.header) == 0 ? request.viewGdiDeviceName.ToString().TrimEnd('\0') : null;
    }
}
