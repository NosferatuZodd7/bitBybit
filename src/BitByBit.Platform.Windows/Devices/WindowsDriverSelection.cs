using System.Runtime.InteropServices;
using System.Runtime.Versioning;
using BitByBit.Core.Common;
using BitByBit.Core.Drivers;
using Windows.Win32;
using Windows.Win32.Devices.DeviceAndDriverInstallation;
using Windows.Win32.Devices.Properties;
using Windows.Win32.Foundation;

namespace BitByBit.Platform.Windows.Devices;

/// <summary>
/// Driver choice for one device, like Device Manager's "Browse my computer › Let me pick": lists the compatible drivers
/// already on this computer (read-only, no administrator needed) and installs a chosen one on that device only
/// (administrator). Nothing is downloaded and no package is added to or removed from the Driver Store.
/// </summary>
[SupportedOSPlatform("windows6.1")]
public static class WindowsDriverSelection
{
    private const int DetailBufferSize = 16 * 1024;

    /// <summary>The driver Plug and Play reports for the device (DEVPKEY_Device_Driver*), or null when it has none.</summary>
    public static InstalledDriverInfo? ReadInstalled(string instanceId)
    {
        if (!TryLocate(instanceId, out var devInst))
        {
            return null;
        }

        var inf = ReadString(devInst, PInvoke.DEVPKEY_Device_DriverInfPath);
        var version = ReadString(devInst, PInvoke.DEVPKEY_Device_DriverVersion);
        if (inf is null && version is null)
        {
            return null;
        }

        return new InstalledDriverInfo(inf, version, ReadFileTime(devInst, PInvoke.DEVPKEY_Device_DriverDate),
            ReadString(devInst, PInvoke.DEVPKEY_Device_DriverProvider), ReadString(devInst, PInvoke.DEVPKEY_Device_DriverDesc));
    }

    /// <summary>Compatible drivers on this computer for the device, best match first. Read-only.</summary>
    public static Result<IReadOnlyList<DriverCandidate>> ListCompatible(string instanceId)
    {
        var current = ReadInstalled(instanceId);
        return WithDriverList(instanceId, (set, data, drivers) =>
        {
            var list = drivers
                .Select(d => new DriverCandidate(d.Description, d.Provider, d.Manufacturer, d.Version, d.Date, d.InfName, d.Rank,
                    current is not null && string.Equals(current.InfName, d.InfName, StringComparison.OrdinalIgnoreCase) &&
                    string.Equals(current.Version, d.Version, StringComparison.OrdinalIgnoreCase)))
                .OrderByDescending(c => c.IsCurrent).ThenBy(c => c.Rank).ThenByDescending(c => c.Date)
                .ToArray();
            return Result<IReadOnlyList<DriverCandidate>>.Success(list);
        });
    }

    /// <summary>
    /// Installs the compatible driver from <paramref name="infName"/> (a bare INF name such as "hdaudio.inf") on this
    /// device only. When the INF offers several matching entries, the best ranked one is used.
    /// </summary>
    public static Result<DeviceOperationResult> Install(string instanceId, string infName) =>
        WithDriverList(instanceId, (set, data, drivers) =>
        {
            var chosen = drivers.Where(d => string.Equals(d.InfName, infName, StringComparison.OrdinalIgnoreCase)).OrderBy(d => d.Rank).FirstOrDefault();
            if (chosen is null)
            {
                return Result<DeviceOperationResult>.Failure(ErrorCode.NotFound, $"No compatible driver from {infName} was found for this device.");
            }

            var info = chosen.Raw;
            if (!PInvoke.SetupDiSetSelectedDriver(set, ref data, ref info))
            {
                return Fail<DeviceOperationResult>("SetupDiSetSelectedDriver");
            }

            if (!PInvoke.DiInstallDevice(HWND.Null, set, in data, info, DIINSTALLDEVICE_FLAGS.DIIDFLAG_NOFINISHINSTALLUI, out var needReboot))
            {
                return Fail<DeviceOperationResult>("DiInstallDevice");
            }

            var (present, started, problem) = WindowsDeviceController.Status(instanceId);
            return Result<DeviceOperationResult>.Success(new DeviceOperationResult(instanceId, needReboot, present, started, problem));
        });

    /// <summary>"Roll back driver": returns to the driver the device used before its last update, without UI.</summary>
    public static Result<DeviceOperationResult> Rollback(string instanceId) =>
        WithDevice(instanceId, (set, data) =>
        {
            if (!PInvoke.DiRollbackDriver(set, in data, HWND.Null, DIROLLBACKDRIVER_FLAGS.ROLLBACK_FLAG_NO_UI, out var needReboot))
            {
                return Fail<DeviceOperationResult>("DiRollbackDriver");
            }

            var (present, started, problem) = WindowsDeviceController.Status(instanceId);
            return Result<DeviceOperationResult>.Success(new DeviceOperationResult(instanceId, needReboot, present, started, problem));
        });

    private sealed record DriverNode(SP_DRVINFO_DATA_V2_W Raw, string Description, string? Provider, string? Manufacturer, string? Version, DateTimeOffset? Date, string InfName, uint Rank);

    private static Result<T> WithDriverList<T>(string instanceId, Func<SafeHandle, SP_DEVINFO_DATA, IReadOnlyList<DriverNode>, Result<T>> action) =>
        WithDevice(instanceId, (set, data) =>
        {
            if (!PInvoke.SetupDiBuildDriverInfoList(set, ref data, SETUP_DI_DRIVER_TYPE.SPDIT_COMPATDRIVER))
            {
                return Fail<T>("SetupDiBuildDriverInfoList");
            }

            try
            {
                return action(set, data, EnumerateDrivers(set, data));
            }
            finally
            {
                PInvoke.SetupDiDestroyDriverInfoList(set, data, SETUP_DI_DRIVER_TYPE.SPDIT_COMPATDRIVER);
            }
        });

    private static unsafe List<DriverNode> EnumerateDrivers(SafeHandle set, SP_DEVINFO_DATA data)
    {
        var nodes = new List<DriverNode>();
        var buffer = new byte[DetailBufferSize];
        for (uint i = 0; i < 512; i++)
        {
            var info = new SP_DRVINFO_DATA_V2_W { cbSize = (uint)sizeof(SP_DRVINFO_DATA_V2_W) };
            if (!PInvoke.SetupDiEnumDriverInfo(set, data, SETUP_DI_DRIVER_TYPE.SPDIT_COMPATDRIVER, i, ref info))
            {
                break; // ERROR_NO_MORE_ITEMS
            }

            string? infPath = null;
            fixed (byte* p = buffer)
            {
                var detail = (SP_DRVINFO_DETAIL_DATA_W*)p;
                detail->cbSize = (uint)sizeof(SP_DRVINFO_DETAIL_DATA_W);
                // A too-small buffer only truncates the hardware id list; the fixed part (INF file name) is still filled.
                if (PInvoke.SetupDiGetDriverInfoDetail(set, data, in info, buffer.AsSpan(), out _) ||
                    Marshal.GetLastPInvokeError() == (int)WIN32_ERROR.ERROR_INSUFFICIENT_BUFFER)
                {
                    infPath = detail->InfFileName.ToString();
                }
            }

            if (string.IsNullOrEmpty(infPath))
            {
                continue;
            }

            var install = new SP_DRVINSTALL_PARAMS { cbSize = (uint)sizeof(SP_DRVINSTALL_PARAMS) };
            var rank = PInvoke.SetupDiGetDriverInstallParams(set, data, in info, ref install) ? install.Rank : uint.MaxValue;
            nodes.Add(new DriverNode(info, info.Description.ToString(), NullIfEmpty(info.ProviderName.ToString()), NullIfEmpty(info.MfgName.ToString()),
                FormatVersion(info.DriverVersion), ToDate(info.DriverDate), Path.GetFileName(infPath), rank));
        }

        return nodes;
    }

    private static Result<T> WithDevice<T>(string instanceId, Func<SafeHandle, SP_DEVINFO_DATA, Result<T>> action)
    {
        if (string.IsNullOrWhiteSpace(instanceId) || instanceId.Length > 400 || instanceId.Any(char.IsControl))
        {
            return Result<T>.Failure(ErrorCode.InvalidArgument, "Invalid device instance id.");
        }

        using var set = PInvoke.SetupDiCreateDeviceInfoList(null, HWND.Null);
        if (set.IsInvalid)
        {
            return Fail<T>("SetupDiCreateDeviceInfoList");
        }

        var data = new SP_DEVINFO_DATA { cbSize = (uint)Marshal.SizeOf<SP_DEVINFO_DATA>() };
        if (!PInvoke.SetupDiOpenDeviceInfo(set, instanceId, HWND.Null, 0, ref data))
        {
            var err = Marshal.GetLastPInvokeError();
            return Result<T>.Failure(ErrorCode.NotFound, $"Device '{instanceId}' not found (error {err}).", err);
        }

        return action(set, data);
    }

    private static unsafe bool TryLocate(string instanceId, out uint devInst)
    {
        devInst = 0;
        if (string.IsNullOrWhiteSpace(instanceId) || instanceId.Length > 400)
        {
            return false;
        }

        fixed (char* id = instanceId)
        {
            return PInvoke.CM_Locate_DevNode(out devInst, new PWSTR(id), CM_LOCATE_DEVNODE_FLAGS.CM_LOCATE_DEVNODE_PHANTOM) == CONFIGRET.CR_SUCCESS;
        }
    }

    private static string? ReadString(uint devInst, DEVPROPKEY key)
    {
        var bytes = ReadProperty(devInst, key, DEVPROPTYPE.DEVPROP_TYPE_STRING);
        return bytes is null ? null : NullIfEmpty(MemoryMarshal.Cast<byte, char>(bytes).ToString().TrimEnd('\0'));
    }

    private static DateTimeOffset? ReadFileTime(uint devInst, DEVPROPKEY key)
    {
        var bytes = ReadProperty(devInst, key, DEVPROPTYPE.DEVPROP_TYPE_FILETIME);
        return bytes is { Length: 8 } && BitConverter.ToInt64(bytes) > 0 ? new DateTimeOffset(DateTime.FromFileTimeUtc(BitConverter.ToInt64(bytes))) : null;
    }

    private static byte[]? ReadProperty(uint devInst, DEVPROPKEY key, DEVPROPTYPE expected)
    {
        uint size = 0;
        var cr = PInvoke.CM_Get_DevNode_Property(devInst, in key, out _, default, ref size, 0);
        if (cr != CONFIGRET.CR_BUFFER_SMALL || size is 0 or > 64 * 1024)
        {
            return null;
        }

        var buffer = new byte[size];
        cr = PInvoke.CM_Get_DevNode_Property(devInst, in key, out var type, buffer, ref size, 0);
        return cr == CONFIGRET.CR_SUCCESS && type == expected ? buffer.AsSpan(0, (int)size).ToArray() : null;
    }

    private static string? FormatVersion(ulong v) =>
        v == 0 ? null : $"{(v >> 48) & 0xFFFF}.{(v >> 32) & 0xFFFF}.{(v >> 16) & 0xFFFF}.{v & 0xFFFF}";

    private static DateTimeOffset? ToDate(System.Runtime.InteropServices.ComTypes.FILETIME ft)
    {
        var ticks = ((long)(uint)ft.dwHighDateTime << 32) | (uint)ft.dwLowDateTime;
        return ticks <= 0 ? null : new DateTimeOffset(DateTime.FromFileTimeUtc(ticks));
    }

    private static string? NullIfEmpty(string? s) => string.IsNullOrWhiteSpace(s) ? null : s;

    private static Result<T> Fail<T>(string api)
    {
        var err = Marshal.GetLastPInvokeError();
        var code = err == 5 ? ErrorCode.AccessDenied : ErrorCode.PlatformError;
        return Result<T>.Failure(code, $"{api} failed (Win32 error {err}).", err);
    }
}
