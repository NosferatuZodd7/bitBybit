using System.Buffers.Binary;
using System.Collections.Concurrent;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;

namespace BitByBit.Core.Ipc;

/// <summary>
/// Server-side two-phase confirmation. A token is HMAC-SHA256 (per-process random key) over: nonce, expiry, operation,
/// canonical parameters, client user SID, client process id and the digest of the plan shown to the user. Tokens are
/// single-use (nonce replay cache) and expire after <see cref="Lifetime"/>. Any change of parameters, client or plan
/// invalidates the token, so the user always confirms exactly what will run.
/// </summary>
public sealed class ConfirmationTokenService : IDisposable
{
    public static readonly TimeSpan Lifetime = TimeSpan.FromSeconds(120);

    private const int NonceBytes = 16;
    private const int MacBytes = 32;

    private readonly byte[] _key = RandomNumberGenerator.GetBytes(32);
    private readonly TimeProvider _clock;
    private readonly ConcurrentDictionary<string, long> _used = new(StringComparer.Ordinal);

    public ConfirmationTokenService(TimeProvider? clock = null)
    {
        _clock = clock ?? TimeProvider.System;
    }

    public (string Token, DateTimeOffset ExpiresAt) Issue(string operation, ReadOnlySpan<byte> canonicalParameters, ClientIdentity client, OperationPlan plan)
    {
        var nonce = RandomNumberGenerator.GetBytes(NonceBytes);
        var expiresAt = _clock.GetUtcNow() + Lifetime;
        var expiryTicks = expiresAt.UtcTicks;
        var mac = ComputeMac(nonce, expiryTicks, operation, canonicalParameters, client, PlanDigest(plan));
        var token = new byte[NonceBytes + 8 + MacBytes];
        nonce.CopyTo(token, 0);
        BinaryPrimitives.WriteInt64LittleEndian(token.AsSpan(NonceBytes, 8), expiryTicks);
        mac.CopyTo(token, NonceBytes + 8);
        return (Base64Url(token), expiresAt);
    }

    /// <summary>Verifies and consumes the token (a second use fails).</summary>
    public bool TryConsume(string token, string operation, ReadOnlySpan<byte> canonicalParameters, ClientIdentity client, OperationPlan plan)
    {
        if (!TryDecode(token, out var raw) || raw.Length != NonceBytes + 8 + MacBytes)
        {
            return false;
        }

        var nonce = raw.AsSpan(0, NonceBytes);
        var expiryTicks = BinaryPrimitives.ReadInt64LittleEndian(raw.AsSpan(NonceBytes, 8));
        var now = _clock.GetUtcNow().UtcTicks;
        if (expiryTicks < now || expiryTicks > now + Lifetime.Ticks * 2)
        {
            return false;
        }

        var expected = ComputeMac(nonce, expiryTicks, operation, canonicalParameters, client, PlanDigest(plan));
        if (!CryptographicOperations.FixedTimeEquals(expected, raw.AsSpan(NonceBytes + 8, MacBytes)))
        {
            return false;
        }

        PurgeExpired(now);
        return _used.TryAdd(Convert.ToHexString(nonce), expiryTicks);
    }

    public void Dispose() => CryptographicOperations.ZeroMemory(_key);

    internal static byte[] PlanDigest(OperationPlan plan) =>
        SHA256.HashData(JsonSerializer.SerializeToUtf8Bytes(plan with { Operation = plan.Operation }, BrokerJson.Options));

    private byte[] ComputeMac(ReadOnlySpan<byte> nonce, long expiryTicks, string operation, ReadOnlySpan<byte> canonical, ClientIdentity client, byte[] planDigest)
    {
        using var hmac = IncrementalHash.CreateHMAC(HashAlgorithmName.SHA256, _key);
        Span<byte> number = stackalloc byte[8];
        hmac.AppendData(nonce);
        BinaryPrimitives.WriteInt64LittleEndian(number, expiryTicks);
        hmac.AppendData(number);
        AppendField(hmac, Encoding.UTF8.GetBytes(operation));
        AppendField(hmac, canonical);
        AppendField(hmac, Encoding.UTF8.GetBytes(client.UserSid ?? string.Empty));
        BinaryPrimitives.WriteInt64LittleEndian(number, client.ProcessId);
        hmac.AppendData(number);
        AppendField(hmac, planDigest);
        return hmac.GetHashAndReset();
    }

    private static void AppendField(IncrementalHash hmac, ReadOnlySpan<byte> data)
    {
        Span<byte> length = stackalloc byte[4];
        BinaryPrimitives.WriteInt32LittleEndian(length, data.Length);
        hmac.AppendData(length);
        hmac.AppendData(data);
    }

    private void PurgeExpired(long nowTicks)
    {
        if (_used.Count < 256)
        {
            return;
        }

        foreach (var (nonce, expiry) in _used)
        {
            if (expiry < nowTicks)
            {
                _used.TryRemove(nonce, out _);
            }
        }
    }

    private static string Base64Url(byte[] data) =>
        Convert.ToBase64String(data).TrimEnd('=').Replace('+', '-').Replace('/', '_');

    private static bool TryDecode(string token, out byte[] raw)
    {
        raw = [];
        if (string.IsNullOrEmpty(token) || token.Length > BrokerEnvelope.MaxConfirmationTokenLength)
        {
            return false;
        }

        var s = token.Replace('-', '+').Replace('_', '/');
        s = s.PadRight(s.Length + ((4 - (s.Length % 4)) % 4), '=');
        try
        {
            raw = Convert.FromBase64String(s);
            return true;
        }
        catch (FormatException)
        {
            return false;
        }
    }
}
