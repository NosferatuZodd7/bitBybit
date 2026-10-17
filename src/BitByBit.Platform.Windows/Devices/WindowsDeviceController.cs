using System.Runtime.InteropServices;
using System.Runtime.Versioning;
using BitByBit.Core.Common;
using Windows.Win32;
using Windows.Win32.Devices.DeviceAndDriverInstallation;
using Windows.Win32.Foundation;

namespace BitByBit.Platform.Windows.Devices;

/// <summary>Result of a device operation; <see cref="RebootRequired"/> comes from DI_NEEDREBOOT/DI_NEEDRESTART or newdev.</summary>
public sealed record DeviceOperationResult(string InstanceId, bool RebootRequired, bool PresentAfter, bool StartedAfter, int? ProblemAfter);

/// <summary>
/// Privileged device operations (administrator required): restart (DIF_PROPERTYCHANGE / DICS_PROPCHANGE), enable and
/// disable (DICS_ENABLE / DICS_DISABLE, global scope), uninstall the
/// device instance (DiUninstallDevice — the driver package stays in the Driver Store) and "scan for hardware changes"
/// (CM_Reenumerate_DevNode on the root). Callers must consult the critical-driver guard and journal first.
/// </summary>
[SupportedOSPlatform("windows6.1")]
public static class WindowsDeviceController
{
    public static Result<DeviceOperationResult> Restart(string instanceId) => PropertyChange(instanceId, SETUP_DI_STATE_CHANGE.DICS_PROPCHANGE);

    /// <summary>Enables or disables the device for all hardware profiles (what Device Manager's Enable/Disable does).</summary>
    public static Result<DeviceOperationResult> SetEnabled(string instanceId, bool enabled) =>
        PropertyChange(instanceId, enabled ? SETUP_DI_STATE_CHANGE.DICS_ENABLE : SETUP_DI_STATE_CHANGE.DICS_DISABLE);

    private static Result<DeviceOperationResult> PropertyChange(string instanceId, SETUP_DI_STATE_CHANGE state) =>
        WithDevice(instanceId, (set, data) =>
        {
            var change = new SP_PROPCHANGE_PARAMS
            {
                ClassInstallHeader = new SP_CLASSINSTALL_HEADER
                {
                    cbSize = (uint)Marshal.SizeOf<SP_CLASSINSTALL_HEADER>(),
                    InstallFunction = DI_FUNCTION.DIF_PROPERTYCHANGE,
                },
                StateChange = state,
                Scope = SETUP_DI_PROPERTY_CHANGE_SCOPE.DICS_FLAG_GLOBAL,
                HwProfile = 0,
            };
            var bytes = MemoryMarshal.AsBytes(MemoryMarshal.CreateReadOnlySpan(ref change, 1));
            if (!PInvoke.SetupDiSetClassInstallParams(set, data, bytes))
            {
                return Fail("SetupDiSetClassInstallParams");
            }

            if (!PInvoke.SetupDiCallClassInstaller(DI_FUNCTION.DIF_PROPERTYCHANGE, set, data))
            {
                return Fail("SetupDiCallClassInstaller(DIF_PROPERTYCHANGE)");
            }

            var reboot = NeedsReboot(set, data);
            var (present, started, problem) = Status(instanceId);
            return Result<DeviceOperationResult>.Success(new DeviceOperationResult(instanceId, reboot, present, started, problem));
        });

    /// <summary>Uninstalls the device instance only (no driver package deletion). The device re-appears on the next rescan
    /// unless an installation restriction policy prevents its driver.</summary>
    public static Result<DeviceOperationResult> Uninstall(string instanceId) =>
        WithDevice(instanceId, (set, data) =>
        {
            if (!PInvoke.DiUninstallDevice(HWND.Null, set, data, 0, out var needReboot))
            {
                return Fail("DiUninstallDevice");
            }

            var (present, started, problem) = Status(instanceId);
            return Result<DeviceOperationResult>.Success(new DeviceOperationResult(instanceId, needReboot, present, started, problem));
        });

    /// <summary>"Scan for hardware changes": re-enumerates the device tree synchronously.</summary>
    public static Result Rescan()
    {
        var cr = PInvoke.CM_Locate_DevNode(out var root, default, CM_LOCATE_DEVNODE_FLAGS.CM_LOCATE_DEVNODE_NORMAL);
        if (cr != CONFIGRET.CR_SUCCESS)
        {
            return Result.Failure(ErrorCode.PlatformError, $"CM_Locate_DevNode failed: {cr}", (int)cr);
        }

        cr = PInvoke.CM_Reenumerate_DevNode(root, CM_REENUMERATE_FLAGS.CM_REENUMERATE_SYNCHRONOUS);
        return cr == CONFIGRET.CR_SUCCESS ? Result.Success() : Result.Failure(ErrorCode.PlatformError, $"CM_Reenumerate_DevNode failed: {cr}", (int)cr);
    }

    /// <summary>Present / started / problem code of a device instance (read-only, no admin needed).</summary>
    public static unsafe (bool Present, bool Started, int? Problem) Status(string instanceId)
    {
        fixed (char* id = instanceId)
        {
            var cr = PInvoke.CM_Locate_DevNode(out var devInst, new PWSTR(id), CM_LOCATE_DEVNODE_FLAGS.CM_LOCATE_DEVNODE_NORMAL);
            if (cr != CONFIGRET.CR_SUCCESS)
            {
                return (false, false, null);
            }

            cr = PInvoke.CM_Get_DevNode_Status(out var status, out var problem, devInst, 0);
            if (cr != CONFIGRET.CR_SUCCESS)
            {
                return (cr != CONFIGRET.CR_NO_SUCH_DEVNODE, false, null);
            }

            var started = (status & CM_DEVNODE_STATUS_FLAGS.DN_STARTED) != 0;
            var hasProblem = (status & CM_DEVNODE_STATUS_FLAGS.DN_HAS_PROBLEM) != 0;
            return (true, started, hasProblem ? (int)problem : 0);
        }
    }

    private static Result<DeviceOperationResult> WithDevice(string instanceId, Func<SafeHandle, SP_DEVINFO_DATA, Result<DeviceOperationResult>> action)
    {
        if (string.IsNullOrWhiteSpace(instanceId) || instanceId.Length > 400 || instanceId.Any(char.IsControl))
        {
            return Result<DeviceOperationResult>.Failure(ErrorCode.InvalidArgument, "Invalid device instance id.");
        }

        using var set = PInvoke.SetupDiCreateDeviceInfoList(null, HWND.Null);
        if (set.IsInvalid)
        {
            return Fail("SetupDiCreateDeviceInfoList");
        }

        var data = new SP_DEVINFO_DATA { cbSize = (uint)Marshal.SizeOf<SP_DEVINFO_DATA>() };
        if (!PInvoke.SetupDiOpenDeviceInfo(set, instanceId, HWND.Null, 0, ref data))
        {
            var err = Marshal.GetLastPInvokeError();
            return Result<DeviceOperationResult>.Failure(ErrorCode.NotFound, $"Device '{instanceId}' not found (error {err}).", err);
        }

        return action(set, data);
    }

    private static bool NeedsReboot(SafeHandle set, SP_DEVINFO_DATA data)
    {
        var p = new SP_DEVINSTALL_PARAMS_W { cbSize = (uint)Marshal.SizeOf<SP_DEVINSTALL_PARAMS_W>() };
        return PInvoke.SetupDiGetDeviceInstallParams(set, data, ref p) &&
               (p.Flags & (SETUP_DI_DEVICE_INSTALL_FLAGS.DI_NEEDREBOOT | SETUP_DI_DEVICE_INSTALL_FLAGS.DI_NEEDRESTART)) != 0;
    }

    private static Result<DeviceOperationResult> Fail(string api)
    {
        var err = Marshal.GetLastPInvokeError();
        var code = err == 5 ? ErrorCode.AccessDenied : ErrorCode.PlatformError;
        return Result<DeviceOperationResult>.Failure(code, $"{api} failed (Win32 error {err}).", err);
    }
}
