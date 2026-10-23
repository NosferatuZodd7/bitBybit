using System.Runtime.InteropServices;
using System.Runtime.Versioning;
using BitByBit.Core.Common;

namespace BitByBit.Platform.Windows.Drivers;

/// <summary>A driver update chosen by the user, as Windows Update describes it.</summary>
public sealed record SelectedDriverUpdate(string UpdateId, string Title, string? HardwareId);

/// <summary>Outcome of one update: WUA result code (2 succeeded, 3 succeeded with errors, 4 failed, 5 aborted).</summary>
public sealed record DriverUpdateOutcome(SelectedDriverUpdate Update, int ResultCode, int HResult)
{
    public bool Succeeded => ResultCode is 2 or 3;
}

/// <summary>
/// Installs driver updates offered by Windows Update, chosen by their update id, in two steps so the caller can journal
/// the affected devices in between: <see cref="Prepare"/> searches and selects, <see cref="Install"/> downloads and
/// installs. Administrator required. Uses only the Windows Update Agent (no other source).
/// </summary>
[SupportedOSPlatform("windows6.1")]
public sealed class WindowsUpdateDriverInstaller : IDisposable
{
    private readonly dynamic _session;
    private readonly dynamic _collection;
    private readonly List<object> _com = [];

    private WindowsUpdateDriverInstaller(dynamic session, dynamic collection, IReadOnlyList<SelectedDriverUpdate> selected)
    {
        _session = session;
        _collection = collection;
        Selected = selected;
    }

    public IReadOnlyList<SelectedDriverUpdate> Selected { get; }

    public bool RebootRequired { get; private set; }

    public static Result<WindowsUpdateDriverInstaller> Prepare(IReadOnlyCollection<string> updateIds)
    {
        var sessionType = Type.GetTypeFromProgID("Microsoft.Update.Session");
        var collectionType = Type.GetTypeFromProgID("Microsoft.Update.UpdateColl");
        if (sessionType is null || collectionType is null)
        {
            return Result<WindowsUpdateDriverInstaller>.Failure(ErrorCode.NotSupported, "The Windows Update Agent is not available.");
        }

        try
        {
            dynamic session = Activator.CreateInstance(sessionType)!;
            session.ClientApplicationID = "bitBybit";
            dynamic searcher = session.CreateUpdateSearcher();
            dynamic result = searcher.Search("IsInstalled=0 and Type='Driver'");
            dynamic collection = Activator.CreateInstance(collectionType)!;
            var selected = new List<SelectedDriverUpdate>();
            int count = result.Updates.Count;
            for (var i = 0; i < count; i++)
            {
                dynamic update = result.Updates.Item(i);
                string id = update.Identity.UpdateID;
                if (!updateIds.Contains(id, StringComparer.OrdinalIgnoreCase))
                {
                    continue;
                }

                if (!(bool)update.EulaAccepted)
                {
                    update.AcceptEula(); // the user confirmed this update in bitBybit
                }

                collection.Add(update);
                string? hwid = null;
                try
                {
                    hwid = update.DriverHardwareID;
                }
                catch (Exception ex) when (ex is COMException or Microsoft.CSharp.RuntimeBinder.RuntimeBinderException)
                {
                }

                selected.Add(new SelectedDriverUpdate(id, (string)update.Title, hwid));
            }

            var installer = new WindowsUpdateDriverInstaller(session, collection, selected);
            installer._com.AddRange(new object[] { result, searcher });
            return selected.Count == 0
                ? Result<WindowsUpdateDriverInstaller>.Failure(ErrorCode.NotFound, "Windows Update no longer offers the selected drivers (already installed or replaced).")
                : Result<WindowsUpdateDriverInstaller>.Success(installer);
        }
        catch (COMException ex)
        {
            return Result<WindowsUpdateDriverInstaller>.Failure(ErrorCode.PlatformError, "Windows Update search failed: " + ex.Message, ex.HResult);
        }
    }

    public Result<IReadOnlyList<DriverUpdateOutcome>> Install()
    {
        try
        {
            dynamic downloader = _session.CreateUpdateDownloader();
            _com.Add(downloader);
            downloader.Updates = _collection;
            downloader.Download();
            dynamic installer = _session.CreateUpdateInstaller();
            _com.Add(installer);
            installer.Updates = _collection;
            dynamic result = installer.Install();
            RebootRequired = (bool)result.RebootRequired;
            var outcomes = new List<DriverUpdateOutcome>();
            for (var i = 0; i < Selected.Count; i++)
            {
                dynamic one = result.GetUpdateResult(i);
                outcomes.Add(new DriverUpdateOutcome(Selected[i], (int)one.ResultCode, (int)one.HResult));
            }

            return Result<IReadOnlyList<DriverUpdateOutcome>>.Success(outcomes);
        }
        catch (COMException ex)
        {
            return Result<IReadOnlyList<DriverUpdateOutcome>>.Failure(ErrorCode.PlatformError, "Windows Update installation failed: " + ex.Message, ex.HResult);
        }
    }

    public void Dispose()
    {
        foreach (var o in _com.Append((object)_collection).Append((object)_session))
        {
            if (Marshal.IsComObject(o))
            {
                Marshal.FinalReleaseComObject(o);
            }
        }
    }
}
