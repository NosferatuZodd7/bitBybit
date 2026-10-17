namespace BitByBit.Core.Ipc;

/// <summary>
/// Platform-neutral handling of Windows file paths for authorization decisions (runs identically on Linux test hosts).
/// Normalization is deliberately strict: only fully qualified local drive paths are accepted; UNC/device paths,
/// relative segments ("." / ".."), alternate data streams (':' inside a segment), wildcard characters and segments with
/// trailing dots/spaces (which Win32 silently strips) are rejected rather than "fixed". Comparison is case-insensitive.
/// </summary>
public static class WindowsPath
{
    public const int MaxPathLength = 32767;
    private const string ExtendedPrefix = @"\\?\";
    private const string NtPrefix = @"\??\";

    /// <summary>Normalizes a fully qualified local path (e.g. <c>c:/Program Files//bitBybit\bitBybit.exe</c> →
    /// <c>C:\Program Files\bitBybit\bitBybit.exe</c>). Returns false for anything ambiguous.</summary>
    public static bool TryNormalize(string? path, out string normalized)
    {
        normalized = string.Empty;
        if (string.IsNullOrEmpty(path) || path.Length > MaxPathLength)
        {
            return false;
        }

        var p = path.Replace('/', '\\');
        if (p.StartsWith(ExtendedPrefix, StringComparison.Ordinal) || p.StartsWith(NtPrefix, StringComparison.Ordinal))
        {
            p = p[4..];
            if (p.StartsWith(@"UNC\", StringComparison.OrdinalIgnoreCase))
            {
                return false;
            }
        }

        if (p.Length < 3 || !char.IsAsciiLetter(p[0]) || p[1] != ':' || p[2] != '\\')
        {
            return false;
        }

        var segments = p[3..].Split('\\', StringSplitOptions.RemoveEmptyEntries);
        foreach (var segment in segments)
        {
            if (!IsValidSegment(segment))
            {
                return false;
            }
        }

        normalized = char.ToUpperInvariant(p[0]) + @":\" + string.Join('\\', segments);
        return true;
    }

    public static bool PathEquals(string normalizedA, string normalizedB) =>
        string.Equals(normalizedA, normalizedB, StringComparison.OrdinalIgnoreCase);

    /// <summary>True when <paramref name="normalizedPath"/> is strictly inside <paramref name="normalizedDirectory"/>
    /// (segment boundary aware: <c>C:\dev\bin2\x.exe</c> is not under <c>C:\dev\bin</c>).</summary>
    public static bool IsUnderDirectory(string normalizedPath, string normalizedDirectory)
    {
        var directory = normalizedDirectory.EndsWith('\\') ? normalizedDirectory : normalizedDirectory + "\\";
        return normalizedPath.Length > directory.Length && normalizedPath.StartsWith(directory, StringComparison.OrdinalIgnoreCase);
    }

    /// <summary>True for <c>X:\</c> (drive roots are never acceptable trust anchors).</summary>
    public static bool IsDriveRoot(string normalizedPath) => normalizedPath.Length == 3 && normalizedPath.EndsWith(@":\", StringComparison.Ordinal);

    /// <summary>Joins a normalized directory and a plain file name (no separators allowed in the name).</summary>
    public static string Combine(string normalizedDirectory, string fileName)
    {
        if (string.IsNullOrEmpty(fileName) || fileName.Contains('\\') || fileName.Contains('/') || !IsValidSegment(fileName))
        {
            throw new ArgumentException("File name must be a single valid path segment.", nameof(fileName));
        }

        return normalizedDirectory.EndsWith('\\') ? normalizedDirectory + fileName : normalizedDirectory + "\\" + fileName;
    }

    /// <summary>
    /// Extracts the executable from a service command line (<c>QUERY_SERVICE_CONFIG.lpBinaryPathName</c>): quoted
    /// (<c>"C:\Program Files\x\svc.exe" --flag</c>) or unquoted without ambiguity. Unquoted paths with spaces are only
    /// accepted when exactly one ".exe" boundary exists; otherwise the command line is considered ambiguous.
    /// </summary>
    public static bool TryExtractExecutable(string? commandLine, out string executable)
    {
        executable = string.Empty;
        if (string.IsNullOrWhiteSpace(commandLine))
        {
            return false;
        }

        var line = commandLine.Trim();
        string candidate;
        if (line[0] == '"')
        {
            var close = line.IndexOf('"', 1);
            if (close <= 1)
            {
                return false;
            }

            candidate = line[1..close];
        }
        else
        {
            var first = line.IndexOf(".exe", StringComparison.OrdinalIgnoreCase);
            if (first < 0)
            {
                return false;
            }

            var end = first + 4;
            var isBoundary = end == line.Length || line[end] == ' ';
            var second = line.IndexOf(".exe", end, StringComparison.OrdinalIgnoreCase);
            if (!isBoundary || (second >= 0 && line[..end].Contains(' ')))
            {
                return false;
            }

            candidate = line[..end];
        }

        return candidate.EndsWith(".exe", StringComparison.OrdinalIgnoreCase) && TryNormalize(candidate, out executable);
    }

    private static bool IsValidSegment(string segment)
    {
        if (segment is "." or ".." || segment.EndsWith('.') || segment.EndsWith(' '))
        {
            return false;
        }

        foreach (var c in segment)
        {
            if (c < 0x20 || c is '<' or '>' or ':' or '"' or '|' or '?' or '*')
            {
                return false;
            }
        }

        return true;
    }
}
