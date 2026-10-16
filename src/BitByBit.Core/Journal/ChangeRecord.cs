using System.Text.Json.Serialization;

namespace BitByBit.Core.Journal;

/// <summary>Area of the system a change belongs to.</summary>
public enum ChangeCategory
{
    Network = 0,
    Firewall = 1,
    PacketFilter = 2,
    Driver = 3,
    DevicePolicy = 4,
    DeviceState = 5,
    Audio = 6,
    Display = 7,
    Service = 8,
    Update = 9,
}

[JsonConverter(typeof(JsonStringEnumConverter<ChangeStatus>))]
public enum ChangeStatus
{
    /// <summary>Change is in effect and owned by bitBybit.</summary>
    Applied = 0,
    /// <summary>bitBybit reverted the change.</summary>
    Reverted = 1,
    /// <summary>Revert was attempted and failed; needs attention.</summary>
    RevertFailed = 2,
    /// <summary>Target was already gone (removed externally or by the OS); nothing to revert.</summary>
    TargetGone = 3,
    /// <summary>Target was modified by someone else after bitBybit applied it; bitBybit will not revert blindly.</summary>
    DriftDetected = 4,
    /// <summary>A later bitBybit change replaced this one (e.g. new NAT prefix).</summary>
    Superseded = 5,
}

/// <summary>
/// A system change made by bitBybit, with enough state to revert it safely.
/// <para><see cref="TargetKind"/> selects the <see cref="IChangeRevertHandler"/>; <see cref="TargetId"/> identifies the
/// object (NAT name, firewall rule name, WFP filter key, registry path, device instance id...).</para>
/// <para><see cref="PreviousState"/> and <see cref="AppliedState"/> are handler-defined JSON documents and MUST NOT contain
/// secrets (passwords, keys). They let the handler detect external drift before reverting.</para>
/// </summary>
public sealed record ChangeRecord
{
    public const string OwnerTag = "bitBybit";

    public required Guid Id { get; init; }
    public required DateTimeOffset CreatedAt { get; init; }
    public required ChangeCategory Category { get; init; }
    public required string TargetKind { get; init; }
    public required string TargetId { get; init; }
    public required string Description { get; init; }
    public string? PreviousState { get; init; }
    public string? AppliedState { get; init; }

    /// <summary>Groups changes made by one user operation (e.g. "start sharing" = IP + NAT + firewall + WFP).</summary>
    public Guid? CorrelationId { get; init; }

    /// <summary>When true the change is reverted automatically at service start if its owning session is gone (e.g. sharing after a crash).</summary>
    public bool RevertOnRecovery { get; init; }

    /// <summary>When true the change is reverted on uninstall.</summary>
    public bool RevertOnUninstall { get; init; } = true;

    public ChangeStatus Status { get; init; } = ChangeStatus.Applied;
    public DateTimeOffset? StatusChangedAt { get; init; }
    public string? StatusDetail { get; init; }

    public bool IsActive => Status is ChangeStatus.Applied or ChangeStatus.RevertFailed;
}
