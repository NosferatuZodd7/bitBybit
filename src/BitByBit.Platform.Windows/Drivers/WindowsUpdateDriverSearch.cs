using System.Runtime.InteropServices;
using System.Runtime.Versioning;
using BitByBit.Core.Common;
using BitByBit.Core.Drivers;

namespace BitByBit.Platform.Windows.Drivers;

/// <summary>
/// Asks the Windows Update Agent which drivers are offered for this computer and not installed
/// (<c>IsInstalled=0 and Type='Driver'</c>). Read-only: nothing is downloaded or installed. Needs the Windows Update
/// service and, usually, an Internet connection; it can take a minute.
/// </summary>
[SupportedOSPlatform("windows6.1")]
public static class WindowsUpdateDriverSearch
{
    public static Task<Result<IReadOnlyList<DriverUpdateCandidate>>> SearchAsync(CancellationToken cancellationToken) =>
        Task.Run(() => Search(cancellationToken), cancellationToken);

    private static Result<IReadOnlyList<DriverUpdateCandidate>> Search(CancellationToken cancellationToken)
    {
        dynamic? session = null;
        dynamic? searcher = null;
        dynamic? result = null;
        try
        {
            var type = Type.GetTypeFromProgID("Microsoft.Update.Session");
            if (type is null)
            {
                return Result<IReadOnlyList<DriverUpdateCandidate>>.Failure(ErrorCode.NotSupported, "The Windows Update Agent is not available.");
            }

            session = Activator.CreateInstance(type)!;
            session.ClientApplicationID = "bitBybit";
            searcher = session.CreateUpdateSearcher();
            cancellationToken.ThrowIfCancellationRequested();
            result = searcher.Search("IsInstalled=0 and Type='Driver'");
            var list = new List<DriverUpdateCandidate>();
            int count = result.Updates.Count;
            for (var i = 0; i < count; i++)
            {
                dynamic update = result.Updates.Item(i);
                try
                {
                    list.Add(new DriverUpdateCandidate
                    {
                        Title = (string)update.Title,
                        UpdateId = Try(() => (string?)update.Identity.UpdateID),
                        DriverClass = Try(() => (string?)update.DriverClass),
                        Provider = Try(() => (string?)update.DriverProvider),
                        Manufacturer = Try(() => (string?)update.DriverManufacturer),
                        Model = Try(() => (string?)update.DriverModel),
                        HardwareId = Try(() => (string?)update.DriverHardwareID),
                        DriverDate = Try<DateTimeOffset?>(() => new DateTimeOffset(DateTime.SpecifyKind((DateTime)update.DriverVerDate, DateTimeKind.Utc))),
                        MaxDownloadBytes = Try<long?>(() => Convert.ToInt64(update.MaxDownloadSize, System.Globalization.CultureInfo.InvariantCulture)),
                    });
                }
                finally
                {
                    Marshal.FinalReleaseComObject(update);
                }
            }

            return Result<IReadOnlyList<DriverUpdateCandidate>>.Success(list
                .OrderBy(c => c.DriverClass, StringComparer.CurrentCultureIgnoreCase)
                .ThenBy(c => c.Title, StringComparer.CurrentCultureIgnoreCase)
                .ToArray());
        }
        catch (COMException ex)
        {
            return Result<IReadOnlyList<DriverUpdateCandidate>>.Failure(ErrorCode.PlatformError, ex.Message, ex.HResult);
        }
        finally
        {
            foreach (var o in new object?[] { result, searcher, session })
            {
                if (o is not null && Marshal.IsComObject(o))
                {
                    Marshal.FinalReleaseComObject(o);
                }
            }
        }
    }

    private static T? Try<T>(Func<T?> read)
    {
        try
        {
            return read();
        }
        catch (Exception ex) when (ex is COMException or Microsoft.CSharp.RuntimeBinder.RuntimeBinderException or InvalidCastException)
        {
            return default;
        }
    }
}
