using System.Collections.Concurrent;

namespace BitByBit.Core.Ipc;

/// <summary>One mutating operation per <see cref="ConcurrencyCategory"/> at a time; waiters give up after a timeout (→ Busy).</summary>
public sealed class ConcurrencyGate
{
    private readonly ConcurrentDictionary<string, SemaphoreSlim> _gates = new(StringComparer.Ordinal);

    /// <summary>Returns a lease to dispose, or null when the category stayed busy for <paramref name="wait"/>.</summary>
    public async Task<IDisposable?> TryEnterAsync(ConcurrencyCategory category, TimeSpan wait, CancellationToken cancellationToken)
    {
        if (category.IsNone)
        {
            return NoopLease.Instance;
        }

        var gate = _gates.GetOrAdd(category.Name, _ => new SemaphoreSlim(1, 1));
        return await gate.WaitAsync(wait, cancellationToken).ConfigureAwait(false) ? new Lease(gate) : null;
    }

    public bool IsBusy(ConcurrencyCategory category) =>
        !category.IsNone && _gates.TryGetValue(category.Name, out var gate) && gate.CurrentCount == 0;

    private sealed class Lease(SemaphoreSlim gate) : IDisposable
    {
        private int _released;

        public void Dispose()
        {
            if (Interlocked.Exchange(ref _released, 1) == 0)
            {
                gate.Release();
            }
        }
    }

    private sealed class NoopLease : IDisposable
    {
        public static NoopLease Instance { get; } = new();

        public void Dispose()
        {
        }
    }
}
