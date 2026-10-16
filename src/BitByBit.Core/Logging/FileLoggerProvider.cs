using System.Collections.Concurrent;
using System.Globalization;
using System.Text;
using Microsoft.Extensions.Logging;

namespace BitByBit.Core.Logging;

/// <summary>
/// Minimal rolling file logger (no third-party dependency). One file per component per day, size-capped, old files
/// pruned. Every message passes through <see cref="Redactor"/>. Logs are English, UTC timestamps.
/// </summary>
public sealed class FileLoggerProvider : ILoggerProvider
{
    private readonly string _directory;
    private readonly string _component;
    private readonly long _maxBytesPerFile;
    private readonly int _retainedFiles;
    private readonly LogLevel _minimumLevel;
    private readonly object _writeLock = new();
    private readonly ConcurrentDictionary<string, FileLogger> _loggers = new(StringComparer.Ordinal);
    private StreamWriter? _writer;
    private string? _currentPath;
    private bool _disposed;

    public FileLoggerProvider(string directory, string component, LogLevel minimumLevel = LogLevel.Information, long maxBytesPerFile = 10 * 1024 * 1024, int retainedFiles = 14)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(directory);
        ArgumentException.ThrowIfNullOrWhiteSpace(component);
        if (component.IndexOfAny(Path.GetInvalidFileNameChars()) >= 0)
        {
            throw new ArgumentException("Component name contains invalid file name characters.", nameof(component));
        }

        _directory = directory;
        _component = component;
        _minimumLevel = minimumLevel;
        _maxBytesPerFile = maxBytesPerFile;
        _retainedFiles = Math.Max(1, retainedFiles);
    }

    public ILogger CreateLogger(string categoryName) => _loggers.GetOrAdd(categoryName, name => new FileLogger(this, name));

    public void Dispose()
    {
        lock (_writeLock)
        {
            _disposed = true;
            _writer?.Dispose();
            _writer = null;
        }
    }

    internal bool IsEnabled(LogLevel level) => level != LogLevel.None && level >= _minimumLevel;

    internal void Write(LogLevel level, string category, EventId eventId, string message, Exception? exception)
    {
        var now = DateTimeOffset.UtcNow;
        var sb = new StringBuilder(256);
        sb.Append(now.ToString("yyyy-MM-ddTHH:mm:ss.fffZ", CultureInfo.InvariantCulture))
          .Append(' ').Append(LevelTag(level))
          .Append(' ').Append(category);
        if (eventId.Id != 0)
        {
            sb.Append('[').Append(eventId.Id.ToString(CultureInfo.InvariantCulture)).Append(']');
        }

        sb.Append(": ").Append(Redactor.Redact(message));
        if (exception is not null)
        {
            sb.AppendLine().Append(Redactor.Redact(exception.ToString()));
        }

        var line = sb.ToString();
        lock (_writeLock)
        {
            if (_disposed)
            {
                return;
            }

            try
            {
                EnsureWriter(now);
                _writer!.WriteLine(line);
                _writer.Flush();
            }
            catch (IOException)
            {
                // Logging must never take down the process. Drop the line.
            }
            catch (UnauthorizedAccessException)
            {
            }
        }
    }

    private void EnsureWriter(DateTimeOffset now)
    {
        var day = now.ToString("yyyyMMdd", CultureInfo.InvariantCulture);
        var basePath = Path.Combine(_directory, $"{_component}-{day}");
        var path = basePath + ".log";

        if (_writer is not null && _currentPath is not null && _currentPath.StartsWith(basePath, StringComparison.Ordinal))
        {
            if (_writer.BaseStream.Length < _maxBytesPerFile)
            {
                return;
            }

            _writer.Dispose();
            _writer = null;
            var n = 1;
            while (File.Exists($"{basePath}.{n}.log"))
            {
                n++;
            }

            path = $"{basePath}.{n}.log";
        }
        else
        {
            _writer?.Dispose();
            _writer = null;
        }

        Directory.CreateDirectory(_directory);
        _writer = new StreamWriter(new FileStream(path, FileMode.Append, FileAccess.Write, FileShare.ReadWrite | FileShare.Delete), new UTF8Encoding(false));
        _currentPath = path;
        Prune();
    }

    private void Prune()
    {
        try
        {
            var files = new DirectoryInfo(_directory)
                .GetFiles($"{_component}-*.log")
                .OrderByDescending(f => f.LastWriteTimeUtc)
                .Skip(_retainedFiles);
            foreach (var file in files)
            {
                file.Delete();
            }
        }
        catch (IOException)
        {
        }
        catch (UnauthorizedAccessException)
        {
        }
    }

    private static string LevelTag(LogLevel level) => level switch
    {
        LogLevel.Trace => "TRC",
        LogLevel.Debug => "DBG",
        LogLevel.Information => "INF",
        LogLevel.Warning => "WRN",
        LogLevel.Error => "ERR",
        LogLevel.Critical => "CRT",
        _ => "???",
    };

    private sealed class FileLogger(FileLoggerProvider provider, string category) : ILogger
    {
        public IDisposable? BeginScope<TState>(TState state) where TState : notnull => null;

        public bool IsEnabled(LogLevel logLevel) => provider.IsEnabled(logLevel);

        public void Log<TState>(LogLevel logLevel, EventId eventId, TState state, Exception? exception, Func<TState, Exception?, string> formatter)
        {
            if (!IsEnabled(logLevel))
            {
                return;
            }

            provider.Write(logLevel, category, eventId, formatter(state, exception), exception);
        }
    }
}
