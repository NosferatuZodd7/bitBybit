using System.Runtime.InteropServices;
using System.Runtime.Versioning;

namespace BitByBit.Platform.Windows.Audio;

// Hand-written COM interop for Core Audio (mmdeviceapi.h / endpointvolume.h) plus the UNDOCUMENTED IPolicyConfig.
// Every method returns an HRESULT ([PreserveSig]) so failures become Result values instead of exceptions.

[StructLayout(LayoutKind.Sequential)]
internal struct PropertyKey
{
    public Guid FormatId;
    public uint PropertyId;

    public PropertyKey(Guid formatId, uint propertyId)
    {
        FormatId = formatId;
        PropertyId = propertyId;
    }
}

/// <summary>PROPVARIANT (24 bytes on x64): vt at 0, value union at 8 (BLOB = cbSize at 8, pBlobData at 16).</summary>
[StructLayout(LayoutKind.Explicit, Size = 24)]
internal struct PropVariant
{
    [FieldOffset(0)] public ushort VarType;
    [FieldOffset(8)] public IntPtr Pointer;
    [FieldOffset(8)] public uint UInt32;
    [FieldOffset(8)] public uint BlobSize;
    [FieldOffset(16)] public IntPtr BlobData;

    public const ushort VtEmpty = 0;
    public const ushort VtI4 = 3;
    public const ushort VtUI4 = 19;
    public const ushort VtLpwstr = 31;
    public const ushort VtBlob = 65;
    public const ushort VtClsid = 72;
}

internal static class PropertyKeys
{
    private static readonly Guid DeviceFmtid = new("a45c254e-df1c-4efd-8020-67d146a850e0");
    private static readonly Guid AudioEndpointFmtid = new("1da5d803-d492-4edd-8c23-e0c0ffee7f0e");

    public static readonly PropertyKey DeviceFriendlyName = new(DeviceFmtid, 14);
    public static readonly PropertyKey DeviceDescription = new(DeviceFmtid, 2);
    public static readonly PropertyKey DeviceInterfaceFriendlyName = new(new Guid("026e516e-b814-414b-83cd-856d6fef4822"), 2);
    public static readonly PropertyKey DeviceContainerId = new(new Guid("8c7ed206-3f8a-4827-b3ab-ae9e1faefc6c"), 2);
    public static readonly PropertyKey EndpointFormFactor = new(AudioEndpointFmtid, 0);
    public static readonly PropertyKey EndpointDisableSysFx = new(AudioEndpointFmtid, 5);
    public static readonly PropertyKey EndpointJackSubType = new(AudioEndpointFmtid, 8);
    public static readonly PropertyKey EngineDeviceFormat = new(new Guid("f19f064d-082c-4e27-bc73-6882a1bb8e4c"), 0);
}

[ComImport, Guid("BCDE0395-E52F-467C-8E3D-C4579291692E")]
internal class MMDeviceEnumeratorCoClass;

/// <summary>UNDOCUMENTED: CPolicyConfigClient. Used by the Windows Sound control panel; no public replacement exists.</summary>
[ComImport, Guid("870af99c-171d-4f9e-af0d-e63df40c2bc9")]
internal class PolicyConfigClientCoClass;

[ComImport, Guid("A95664D2-9614-4F35-A746-DE8DB63617E6"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
internal interface IMMDeviceEnumerator
{
    [PreserveSig] int EnumAudioEndpoints(int dataFlow, int stateMask, out IMMDeviceCollection? devices);
    [PreserveSig] int GetDefaultAudioEndpoint(int dataFlow, int role, out IMMDevice? endpoint);
    [PreserveSig] int GetDevice([MarshalAs(UnmanagedType.LPWStr)] string id, out IMMDevice? device);
    [PreserveSig] int RegisterEndpointNotificationCallback(IMMNotificationClient client);
    [PreserveSig] int UnregisterEndpointNotificationCallback(IMMNotificationClient client);
}

[ComImport, Guid("0BD7A1BE-7A1A-44DB-8397-CC5392387B5E"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
internal interface IMMDeviceCollection
{
    [PreserveSig] int GetCount(out uint count);
    [PreserveSig] int Item(uint index, out IMMDevice? device);
}

[ComImport, Guid("D666063F-1587-4E43-81F1-B948E807363F"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
internal interface IMMDevice
{
    [PreserveSig] int Activate(ref Guid iid, int clsCtx, IntPtr activationParams, [MarshalAs(UnmanagedType.IUnknown)] out object? instance);
    [PreserveSig] int OpenPropertyStore(int access, out IPropertyStore? properties);
    [PreserveSig] int GetId([MarshalAs(UnmanagedType.LPWStr)] out string? id);
    [PreserveSig] int GetState(out int state);
}

/// <summary>Documented IMMEndpoint (QueryInterface from IMMDevice): the endpoint's data flow.</summary>
[ComImport, Guid("1BE09788-6894-4089-8586-9A2A6C265AC5"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
internal interface IMMEndpoint
{
    [PreserveSig] int GetDataFlow(out int dataFlow);
}

[ComImport, Guid("886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
internal interface IPropertyStore
{
    [PreserveSig] int GetCount(out uint count);
    [PreserveSig] int GetAt(uint index, out PropertyKey key);
    [PreserveSig] int GetValue(ref PropertyKey key, out PropVariant value);
    [PreserveSig] int SetValue(ref PropertyKey key, ref PropVariant value);
    [PreserveSig] int Commit();
}

/// <summary>Callbacks arrive on Core Audio worker threads. Implementations must not block or throw.</summary>
[ComImport, Guid("7991EEC9-7E89-4D85-8390-6C703CEC60C0"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
internal interface IMMNotificationClient
{
    [PreserveSig] int OnDeviceStateChanged([MarshalAs(UnmanagedType.LPWStr)] string deviceId, int newState);
    [PreserveSig] int OnDeviceAdded([MarshalAs(UnmanagedType.LPWStr)] string deviceId);
    [PreserveSig] int OnDeviceRemoved([MarshalAs(UnmanagedType.LPWStr)] string deviceId);
    [PreserveSig] int OnDefaultDeviceChanged(int flow, int role, [MarshalAs(UnmanagedType.LPWStr)] string? defaultDeviceId);
    [PreserveSig] int OnPropertyValueChanged([MarshalAs(UnmanagedType.LPWStr)] string deviceId, PropertyKey key);
}

[ComImport, Guid("5CDF2C82-841E-4546-9722-0CF74078229A"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
internal interface IAudioEndpointVolume
{
    [PreserveSig] int RegisterControlChangeNotify(IntPtr notify);
    [PreserveSig] int UnregisterControlChangeNotify(IntPtr notify);
    [PreserveSig] int GetChannelCount(out uint channelCount);
    [PreserveSig] int SetMasterVolumeLevel(float levelDb, ref Guid eventContext);
    [PreserveSig] int SetMasterVolumeLevelScalar(float level, ref Guid eventContext);
    [PreserveSig] int GetMasterVolumeLevel(out float levelDb);
    [PreserveSig] int GetMasterVolumeLevelScalar(out float level);
    [PreserveSig] int SetChannelVolumeLevel(uint channel, float levelDb, ref Guid eventContext);
    [PreserveSig] int SetChannelVolumeLevelScalar(uint channel, float level, ref Guid eventContext);
    [PreserveSig] int GetChannelVolumeLevel(uint channel, out float levelDb);
    [PreserveSig] int GetChannelVolumeLevelScalar(uint channel, out float level);
    [PreserveSig] int SetMute([MarshalAs(UnmanagedType.Bool)] bool mute, ref Guid eventContext);
    [PreserveSig] int GetMute([MarshalAs(UnmanagedType.Bool)] out bool mute);
    [PreserveSig] int GetVolumeStepInfo(out uint step, out uint stepCount);
    [PreserveSig] int VolumeStepUp(ref Guid eventContext);
    [PreserveSig] int VolumeStepDown(ref Guid eventContext);
    [PreserveSig] int QueryHardwareSupport(out uint hardwareSupportMask);
    [PreserveSig] int GetVolumeRange(out float minDb, out float maxDb, out float incrementDb);
}

/// <summary>
/// UNDOCUMENTED IPolicyConfig (Windows 10/11 IID). Method order follows the widely used community definition; only
/// <see cref="SetDefaultEndpoint"/> and <see cref="SetEndpointVisibility"/> are called by bitBybit. The other slots are kept
/// (with opaque pointer parameters) solely so the vtable layout is correct.
/// </summary>
[ComImport, Guid("f8679f50-850a-41cf-9c72-430f290290c8"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
internal interface IPolicyConfig
{
    [PreserveSig] int GetMixFormat([MarshalAs(UnmanagedType.LPWStr)] string deviceId, IntPtr format);
    [PreserveSig] int GetDeviceFormat([MarshalAs(UnmanagedType.LPWStr)] string deviceId, int useDefault, IntPtr format);
    [PreserveSig] int ResetDeviceFormat([MarshalAs(UnmanagedType.LPWStr)] string deviceId);
    [PreserveSig] int SetDeviceFormat([MarshalAs(UnmanagedType.LPWStr)] string deviceId, IntPtr endpointFormat, IntPtr mixFormat);
    [PreserveSig] int GetProcessingPeriod([MarshalAs(UnmanagedType.LPWStr)] string deviceId, int useDefault, IntPtr defaultPeriod, IntPtr minimumPeriod);
    [PreserveSig] int SetProcessingPeriod([MarshalAs(UnmanagedType.LPWStr)] string deviceId, IntPtr period);
    [PreserveSig] int GetShareMode([MarshalAs(UnmanagedType.LPWStr)] string deviceId, IntPtr mode);
    [PreserveSig] int SetShareMode([MarshalAs(UnmanagedType.LPWStr)] string deviceId, IntPtr mode);
    [PreserveSig] int GetPropertyValue([MarshalAs(UnmanagedType.LPWStr)] string deviceId, int fxStore, IntPtr key, IntPtr value);
    [PreserveSig] int SetPropertyValue([MarshalAs(UnmanagedType.LPWStr)] string deviceId, int fxStore, IntPtr key, IntPtr value);
    [PreserveSig] int SetDefaultEndpoint([MarshalAs(UnmanagedType.LPWStr)] string deviceId, int role);
    [PreserveSig] int SetEndpointVisibility([MarshalAs(UnmanagedType.LPWStr)] string deviceId, int visible);
}

[SupportedOSPlatform("windows")]
internal static class CoreAudio
{
    public const int DeviceStateMaskAll = 0xF;
    public const int StgmRead = 0;
    public const int ClsctxAll = 0x17;
    public const int ENotFound = unchecked((int)0x80070490);
    public static Guid IidAudioEndpointVolume = new("5CDF2C82-841E-4546-9722-0CF74078229A");

    [DllImport("ole32.dll")]
    public static extern int PropVariantClear(ref PropVariant value);

    public static IMMDeviceEnumerator CreateEnumerator() => (IMMDeviceEnumerator)new MMDeviceEnumeratorCoClass();

    public static void Release(object? com)
    {
        if (com is not null && Marshal.IsComObject(com))
        {
            Marshal.ReleaseComObject(com);
        }
    }

    public static string? ReadString(IPropertyStore store, PropertyKey key)
    {
        if (store.GetValue(ref key, out var v) != 0)
        {
            return null;
        }

        try
        {
            return v.VarType == PropVariant.VtLpwstr && v.Pointer != IntPtr.Zero ? Marshal.PtrToStringUni(v.Pointer) : null;
        }
        finally
        {
            PropVariantClear(ref v);
        }
    }

    public static uint? ReadUInt(IPropertyStore store, PropertyKey key)
    {
        if (store.GetValue(ref key, out var v) != 0)
        {
            return null;
        }

        try
        {
            return v.VarType is PropVariant.VtUI4 or PropVariant.VtI4 ? v.UInt32 : null;
        }
        finally
        {
            PropVariantClear(ref v);
        }
    }

    public static Guid? ReadGuid(IPropertyStore store, PropertyKey key)
    {
        if (store.GetValue(ref key, out var v) != 0)
        {
            return null;
        }

        try
        {
            if (v.VarType == PropVariant.VtClsid && v.Pointer != IntPtr.Zero)
            {
                return Marshal.PtrToStructure<Guid>(v.Pointer);
            }

            if (v.VarType == PropVariant.VtLpwstr && v.Pointer != IntPtr.Zero && Guid.TryParse(Marshal.PtrToStringUni(v.Pointer), out var g))
            {
                return g;
            }

            return null;
        }
        finally
        {
            PropVariantClear(ref v);
        }
    }

    public static byte[]? ReadBlob(IPropertyStore store, PropertyKey key)
    {
        if (store.GetValue(ref key, out var v) != 0)
        {
            return null;
        }

        try
        {
            if (v.VarType != PropVariant.VtBlob || v.BlobData == IntPtr.Zero || v.BlobSize is 0 or > 4096)
            {
                return null;
            }

            var bytes = new byte[v.BlobSize];
            Marshal.Copy(v.BlobData, bytes, 0, bytes.Length);
            return bytes;
        }
        finally
        {
            PropVariantClear(ref v);
        }
    }
}
