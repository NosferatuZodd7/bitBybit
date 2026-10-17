using System.Text;
using System.Text.Json;
using BitByBit.Core.Logging;

namespace BitByBit.Core.Ipc;

/// <summary>
/// Turns request parameters into a short, secret-free string for the audit log: properties whose name looks secret
/// are masked regardless of value, string values pass through <see cref="Redactor"/>, and the result is truncated.
/// </summary>
public static class AuditRedaction
{
    private static readonly string[] SecretNameFragments =
        ["password", "passphrase", "passwd", "pwd", "psk", "secret", "token", "key", "credential"];

    public static bool IsSecretPropertyName(string name)
    {
        var lower = name.ToLowerInvariant();
        foreach (var fragment in SecretNameFragments)
        {
            if (lower.Contains(fragment, StringComparison.Ordinal))
            {
                return true;
            }
        }

        return false;
    }

    public static string Describe(JsonElement element, int maxChars)
    {
        var sb = new StringBuilder();
        Write(element, sb, depth: 0);
        var text = Redactor.Redact(sb.ToString());
        return maxChars > 0 && text.Length > maxChars ? text[..maxChars] + "…" : text;
    }

    private static void Write(JsonElement element, StringBuilder sb, int depth)
    {
        if (depth > BrokerJson.MaxDepth)
        {
            sb.Append("…");
            return;
        }

        switch (element.ValueKind)
        {
            case JsonValueKind.Object:
                sb.Append('{');
                var first = true;
                foreach (var p in element.EnumerateObject())
                {
                    if (!first)
                    {
                        sb.Append(',');
                    }

                    first = false;
                    sb.Append(p.Name).Append(':');
                    if (IsSecretPropertyName(p.Name) && p.Value.ValueKind is not (JsonValueKind.Null or JsonValueKind.Undefined))
                    {
                        sb.Append(Redactor.Mask);
                    }
                    else
                    {
                        Write(p.Value, sb, depth + 1);
                    }
                }

                sb.Append('}');
                break;
            case JsonValueKind.Array:
                sb.Append('[');
                var i = 0;
                foreach (var item in element.EnumerateArray())
                {
                    if (i++ > 0)
                    {
                        sb.Append(',');
                    }

                    if (i > 32)
                    {
                        sb.Append('…');
                        break;
                    }

                    Write(item, sb, depth + 1);
                }

                sb.Append(']');
                break;
            case JsonValueKind.String:
                sb.Append('"').Append(element.GetString()).Append('"');
                break;
            default:
                sb.Append(element.GetRawText());
                break;
        }
    }
}
