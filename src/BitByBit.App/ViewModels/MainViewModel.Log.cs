using System.Text;
using BitByBit.App.Infrastructure;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;

namespace BitByBit.App.ViewModels;

/// <summary>Log page: the tail of today's app log, with Copy and Open folder, so a report needs no explanation.</summary>
public sealed partial class MainViewModel
{
    private const int LogTailLines = 600;

    [ObservableProperty]
    private string _logText = string.Empty;

    [ObservableProperty]
    [NotifyPropertyChangedFor(nameof(PageSubtitle))]
    private string _logFile = "-";

    [RelayCommand]
    private void LoadLog()
    {
        try
        {
            var dir = new DirectoryInfo(AppInfo.Paths.UserLogs);
            var latest = dir.Exists ? dir.EnumerateFiles("app-*.log").OrderByDescending(f => f.LastWriteTimeUtc).FirstOrDefault() : null;
            if (latest is null)
            {
                LogText = Text.Get("app.page.log.empty");
                LogFile = "-";
                return;
            }

            using var stream = new FileStream(latest.FullName, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete);
            using var reader = new StreamReader(stream, Encoding.UTF8);
            var lines = new Queue<string>(LogTailLines);
            while (reader.ReadLine() is { } line)
            {
                if (lines.Count == LogTailLines)
                {
                    lines.Dequeue();
                }

                lines.Enqueue(line);
            }

            LogText = string.Join(Environment.NewLine, lines);
            LogFile = latest.Name;
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
        {
            LogText = ex.Message;
        }
    }

    [RelayCommand]
    private void CopyLog()
    {
        if (LogText.Length > 0)
        {
            System.Windows.Clipboard.SetText(LogText);
            ShowToast(Text.Get("app.page.log.copied"));
        }
    }
}
