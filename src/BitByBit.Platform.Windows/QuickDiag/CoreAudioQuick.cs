using System.Runtime.InteropServices;
using System.Runtime.Versioning;
using BitByBit.Core.QuickDiag;

namespace BitByBit.Platform.Windows.QuickDiag;

/// <summary>
/// Read-only Core Audio endpoint enumeration (documented MMDevice API). Enumerates endpoints in every state and the
/// default render/capture endpoint per role. Never changes defaults or volume.
/// </summary>
[SupportedOSPlatform("windows")]
internal static class CoreAudioQuick
{
    private const int DeviceStateMaskAll = 0xF;
    private const int StgmRead = 0;
    private const int ENotFound = unchecked((int)0x80070490);

    private static readonly Guid FriendlyNameFmtid = new("a45c254e-df1c-4efd-8020-67d146a850e0");
    private static readonly Guid InterfaceFriendlyNameFmtid = new("026e516e-b814-414b-83cd-856d6fef4822");
    private static readonly Guid FormFactorFmtid = new("1da5d803-d492-4edd-8c23-e0c0ffee7f0e");

    public static IReadOnlyList<AudioEndpointSummary> Enumerate()
    {
        var enumerator = (IMMDeviceEnumerator)new MMDeviceEnumeratorComObject();
        try
        {
            var defaults = new Dictionary<(int Flow, int Role), string>();
            foreach (var flow in new[] { 0, 1 })
            {
                foreach (var role in new[] { 0, 1, 2 })
                {
                    if (enumerator.GetDefaultAudioEndpoint(flow, role, out var device) == 0 && device is not null)
                    {
                        try
                        {
                            if (device.GetId(out var id) == 0 && id is not null)
                            {
                                defaults[(flow, role)] = id;
                            }
                        }
                        finally
                        {
                            Marshal.ReleaseComObject(device);
                        }
                    }
                }
            }

            var result = new List<AudioEndpointSummary>();
            foreach (var flow in new[] { 0, 1 })
            {
                Check(enumerator.EnumAudioEndpoints(flow, DeviceStateMaskAll, out var collection));
                try
                {
                    Check(collection.GetCount(out var count));
                    for (uint i = 0; i < count; i++)
                    {
                        if (collection.Item(i, out var device) != 0 || device is null)
                        {
                            continue;
                        }

                        try
                        {
                            result.Add(Describe(device, flow, defaults));
                        }
                        finally
                        {
                            Marshal.ReleaseComObject(device);
                        }
                    }
                }
                finally
                {
                    Marshal.ReleaseComObject(collection);
                }
            }

            return result;
        }
        finally
        {
            Marshal.ReleaseComObject(enumerator);
        }
    }

    private static AudioEndpointSummary Describe(IMMDevice device, int flow, Dictionary<(int Flow, int Role), string> defaults)
    {
        Check(device.GetId(out var id));
        device.GetState(out var state);
        string? name = null, adapter = null, description = null;
        int? formFactor = null;
        if (device.OpenPropertyStore(StgmRead, out var store) == 0 && store is not null)
        {
            try
            {
                name = ReadString(store, FriendlyNameFmtid, 14);
                description = ReadString(store, FriendlyNameFmtid, 2);
                adapter = ReadString(store, InterfaceFriendlyNameFmtid, 2);
                formFactor = ReadUInt(store, FormFactorFmtid, 0);
            }
            finally
            {
                Marshal.ReleaseComObject(store);
            }
        }

        return new AudioEndpointSummary
        {
            Id = id ?? string.Empty,
            Name = name,
            Description = description,
            AdapterName = adapter,
            Flow = flow == 0 ? EndpointFlow.Render : EndpointFlow.Capture,
            State = (EndpointState)state,
            FormFactor = formFactor,
            IsDefaultConsole = defaults.TryGetValue((flow, 0), out var c) && c == id,
            IsDefaultMultimedia = defaults.TryGetValue((flow, 1), out var m) && m == id,
            IsDefaultCommunications = defaults.TryGetValue((flow, 2), out var k) && k == id,
        };
    }

    private static string? ReadString(IPropertyStore store, Guid fmtid, uint pid)
    {
        var key = new PropertyKey { FormatId = fmtid, PropertyId = pid };
        if (store.GetValue(ref key, out var value) != 0)
        {
            return null;
        }

        try
        {
            return value.VarType == 31 /* VT_LPWSTR */ && value.Pointer != IntPtr.Zero ? Marshal.PtrToStringUni(value.Pointer) : null;
        }
        finally
        {
            PropVariantClear(ref value);
        }
    }

    private static int? ReadUInt(IPropertyStore store, Guid fmtid, uint pid)
    {
        var key = new PropertyKey { FormatId = fmtid, PropertyId = pid };
        if (store.GetValue(ref key, out var value) != 0)
        {
            return null;
        }

        try
        {
            return value.VarType is 19 /* VT_UI4 */ or 3 /* VT_I4 */ ? value.Int32 : null;
        }
        finally
        {
            PropVariantClear(ref value);
        }
    }

    private static void Check(int hr)
    {
        if (hr < 0 && hr != ENotFound)
        {
            Marshal.ThrowExceptionForHR(hr);
        }
    }

    [DllImport("ole32.dll")]
    private static extern int PropVariantClear(ref PropVariant pvar);

    [StructLayout(LayoutKind.Sequential)]
    private struct PropertyKey
    {
        public Guid FormatId;
        public uint PropertyId;
    }

    /// <summary>PROPVARIANT (24 bytes on x64): vt at 0, value union at 8.</summary>
    [StructLayout(LayoutKind.Explicit, Size = 24)]
    private struct PropVariant
    {
        [FieldOffset(0)] public ushort VarType;
        [FieldOffset(8)] public IntPtr Pointer;
        [FieldOffset(8)] public int Int32;
    }

    [ComImport, Guid("BCDE0395-E52F-467C-8E3D-C4579291692E")]
    private class MMDeviceEnumeratorComObject;

    [ComImport, Guid("A95664D2-9614-4F35-A746-DE8DB63617E6"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IMMDeviceEnumerator
    {
        [PreserveSig] int EnumAudioEndpoints(int dataFlow, int stateMask, out IMMDeviceCollection devices);
        [PreserveSig] int GetDefaultAudioEndpoint(int dataFlow, int role, out IMMDevice? endpoint);
        [PreserveSig] int GetDevice([MarshalAs(UnmanagedType.LPWStr)] string id, out IMMDevice device);
        [PreserveSig] int RegisterEndpointNotificationCallback(IntPtr client);
        [PreserveSig] int UnregisterEndpointNotificationCallback(IntPtr client);
    }

    [ComImport, Guid("0BD7A1BE-7A1A-44DB-8397-CC5392387B5E"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IMMDeviceCollection
    {
        [PreserveSig] int GetCount(out uint count);
        [PreserveSig] int Item(uint index, out IMMDevice? device);
    }

    [ComImport, Guid("D666063F-1587-4E43-81F1-B948E807363F"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IMMDevice
    {
        [PreserveSig] int Activate(ref Guid iid, int clsCtx, IntPtr activationParams, [MarshalAs(UnmanagedType.IUnknown)] out object instance);
        [PreserveSig] int OpenPropertyStore(int access, out IPropertyStore? properties);
        [PreserveSig] int GetId([MarshalAs(UnmanagedType.LPWStr)] out string? id);
        [PreserveSig] int GetState(out int state);
    }

    [ComImport, Guid("886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IPropertyStore
    {
        [PreserveSig] int GetCount(out uint count);
        [PreserveSig] int GetAt(uint index, out PropertyKey key);
        [PreserveSig] int GetValue(ref PropertyKey key, out PropVariant value);
        [PreserveSig] int SetValue(ref PropertyKey key, ref PropVariant value);
        [PreserveSig] int Commit();
    }
}
