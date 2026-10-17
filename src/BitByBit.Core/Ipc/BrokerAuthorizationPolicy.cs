namespace BitByBit.Core.Ipc;

/// <summary>Outcome of an authorization check, with the reason for the audit log.</summary>
public readonly record struct AuthorizationDecision(bool Allowed, string Reason, bool ElevationWouldHelp = false)
{
    public static AuthorizationDecision Allow(string reason) => new(true, reason);

    public static AuthorizationDecision Deny(string reason, bool elevationWouldHelp = false) => new(false, reason, elevationWouldHelp);
}

/// <summary>
/// Who may call which tier (docs/SECURITY.md): only resolved, local clients of at least Medium integrity whose process
/// image is one of the installed bitBybit binaries; Elevated-tier operations additionally need an elevated token.
/// Developer mode (off by default, enabled only through an admin-writable machine config file) allows images under one
/// explicit directory, e.g. a build output folder.
/// </summary>
public sealed class BrokerAuthorizationPolicy
{
    private readonly string[] _allowedImages;
    private readonly string? _developerDirectory;

    /// <param name="allowedImagePaths">Full paths of the installed client binaries (bitBybit.exe, bitbybit-cli.exe).</param>
    /// <param name="developerModeDirectory">Null unless developer mode was explicitly enabled.</param>
    public BrokerAuthorizationPolicy(IEnumerable<string> allowedImagePaths, string? developerModeDirectory = null)
    {
        ArgumentNullException.ThrowIfNull(allowedImagePaths);
        _allowedImages = allowedImagePaths
            .Select(p => WindowsPath.TryNormalize(p, out var n) ? n : throw new ArgumentException($"Invalid allowed image path '{p}'.", nameof(allowedImagePaths)))
            .ToArray();
        if (developerModeDirectory is not null)
        {
            if (!WindowsPath.TryNormalize(developerModeDirectory, out var dir) || WindowsPath.IsDriveRoot(dir))
            {
                throw new ArgumentException("Developer mode directory must be a normalized, non-root directory.", nameof(developerModeDirectory));
            }

            _developerDirectory = dir;
        }
    }

    public bool DeveloperModeEnabled => _developerDirectory is not null;

    public AuthorizationDecision Authorize(ClientIdentity client, RiskTier tier)
    {
        ArgumentNullException.ThrowIfNull(client);
        if (!client.IsResolved)
        {
            return AuthorizationDecision.Deny("client identity unresolved: " + (client.ResolutionError ?? "incomplete"));
        }

        if (!client.IsLocal)
        {
            return AuthorizationDecision.Deny("remote clients are not allowed");
        }

        if (client.IntegrityLevel < IntegrityLevel.Medium)
        {
            return AuthorizationDecision.Deny($"integrity level {client.IntegrityLevel} is below Medium");
        }

        if (!IsAllowedImage(client.ImagePath, out var how))
        {
            return AuthorizationDecision.Deny("client image is not an installed bitBybit binary");
        }

        if (tier == RiskTier.Elevated && !client.IsElevated)
        {
            return AuthorizationDecision.Deny("elevated tier requires an elevated client token", elevationWouldHelp: true);
        }

        return AuthorizationDecision.Allow($"{tier} allowed ({how})");
    }

    private bool IsAllowedImage(string? imagePath, out string how)
    {
        how = string.Empty;
        if (!WindowsPath.TryNormalize(imagePath, out var normalized))
        {
            return false;
        }

        foreach (var allowed in _allowedImages)
        {
            if (WindowsPath.PathEquals(normalized, allowed))
            {
                how = "installed image";
                return true;
            }
        }

        if (_developerDirectory is not null && WindowsPath.IsUnderDirectory(normalized, _developerDirectory))
        {
            how = "developer mode";
            return true;
        }

        return false;
    }
}
