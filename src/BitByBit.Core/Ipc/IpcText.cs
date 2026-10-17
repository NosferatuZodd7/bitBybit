using BitByBit.Core.Common;

namespace BitByBit.Core.Ipc;

/// <summary>User-facing texts of the broker protocol (pt-PT in Localization/Catalogs/pt-PT/ipc.json).</summary>
internal static class IpcText
{
    public static LocalizedText UnsupportedProtocol(int version) =>
        LocalizedText.Of("ipc.error.unsupportedProtocol", "The bitBybit service uses a different protocol version ({0}). Reinstall or update bitBybit.", version);

    public static LocalizedText MalformedRequest() =>
        LocalizedText.Of("ipc.error.malformedRequest", "The request sent to the bitBybit service is not valid.");

    public static LocalizedText ConfirmationInvalid() =>
        LocalizedText.Of("ipc.error.confirmationInvalid", "The confirmation is no longer valid. Review the operation and confirm again.");

    public static LocalizedText UnknownOperation(string name) =>
        LocalizedText.Of("ipc.error.unknownOperation", "The bitBybit service does not allow the operation '{0}'.", name);

    public static LocalizedText ClientNotAuthorized() =>
        LocalizedText.Of("ipc.error.clientNotAuthorized", "This program is not an installed bitBybit component and cannot use the bitBybit service.");

    public static LocalizedText ElevationRequired() =>
        LocalizedText.Of("ipc.error.elevationRequired", "This operation requires administrator rights. bitBybit will ask Windows for permission (UAC).");

    public static LocalizedText Busy(string category) =>
        LocalizedText.Of("ipc.error.busy", "Another '{0}' operation is in progress. Try again in a moment.", category);

    public static LocalizedText Timeout() =>
        LocalizedText.Of("ipc.error.timeout", "The operation took too long and was stopped.");

    public static LocalizedText InternalError() =>
        LocalizedText.Of("ipc.error.internal", "The bitBybit service hit an unexpected error. Details were written to the service log.");
}
