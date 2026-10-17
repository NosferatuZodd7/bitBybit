using System.Text.Json;
using BitByBit.Core.Common;

namespace BitByBit.Core.Ipc;

/// <summary>A broker-side notification (e.g. "leakGuard.stateChanged", "sharing.clientConnected").</summary>
public sealed record BrokerEvent
{
    public required long Sequence { get; init; }
    public required string Topic { get; init; }
    public required DateTimeOffset At { get; init; }
    public LocalizedText? Message { get; init; }
    public JsonElement? Data { get; init; }
}

/// <summary>Publishing side used by broker services.</summary>
public interface IBrokerEventPublisher
{
    void Publish(string topic, LocalizedText? message = null, object? data = null);
}

/// <summary>
/// Bounded in-memory ring buffer of events with a monotonic sequence, consumed by <c>broker.events.poll</c>. Old events
/// are dropped when the buffer is full; pollers detect the gap through <see cref="EventsPage.Truncated"/>.
/// </summary>
public sealed class BrokerEventHub : IBrokerEventPublisher
{
    private readonly object _lock = new();
    private readonly LinkedList<BrokerEvent> _events = new();
    private readonly int _capacity;
    private readonly TimeProvider _clock;
    private long _sequence;
    private TaskCompletionSource _signal = new(TaskCreationOptions.RunContinuationsAsynchronously);

    public BrokerEventHub(int capacity = 1024, TimeProvider? clock = null)
    {
        ArgumentOutOfRangeException.ThrowIfLessThan(capacity, 1);
        _capacity = capacity;
        _clock = clock ?? TimeProvider.System;
    }

    public long LatestSequence
    {
        get
        {
            lock (_lock)
            {
                return _sequence;
            }
        }
    }

    public void Publish(string topic, LocalizedText? message = null, object? data = null)
    {
        if (!BrokerEnvelope.IsValidOperationName(topic))
        {
            throw new ArgumentException($"Event topic '{topic}' is not valid.", nameof(topic));
        }

        JsonElement? payload = data is null ? null : JsonSerializer.SerializeToElement(data, BrokerJson.Options);
        TaskCompletionSource signal;
        lock (_lock)
        {
            _events.AddLast(new BrokerEvent { Sequence = ++_sequence, Topic = topic, At = _clock.GetUtcNow(), Message = message, Data = payload });
            while (_events.Count > _capacity)
            {
                _events.RemoveFirst();
            }

            signal = _signal;
            _signal = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        }

        signal.TrySetResult();
    }

    /// <summary>Events with a sequence greater than <paramref name="afterSequence"/>, waiting up to <paramref name="maxWait"/> for new ones.</summary>
    public async Task<EventsPage> PollAsync(long afterSequence, int maxEvents, TimeSpan maxWait, CancellationToken cancellationToken)
    {
        maxEvents = Math.Clamp(maxEvents, 1, 500);
        Task wait;
        lock (_lock)
        {
            var page = Collect(afterSequence, maxEvents);
            if (page.Events.Count > 0 || maxWait <= TimeSpan.Zero)
            {
                return page;
            }

            wait = _signal.Task;
        }

        try
        {
            await wait.WaitAsync(maxWait, _clock, cancellationToken).ConfigureAwait(false);
        }
        catch (TimeoutException)
        {
        }

        lock (_lock)
        {
            return Collect(afterSequence, maxEvents);
        }
    }

    private EventsPage Collect(long afterSequence, int maxEvents)
    {
        var list = _events.Where(e => e.Sequence > afterSequence).Take(maxEvents).ToArray();
        var oldest = _events.First?.Value.Sequence ?? _sequence + 1;
        return new EventsPage
        {
            Events = list,
            LatestSequence = _sequence,
            Truncated = afterSequence + 1 < oldest && afterSequence < _sequence,
        };
    }
}

public sealed record EventsPage
{
    public required IReadOnlyList<BrokerEvent> Events { get; init; }
    public required long LatestSequence { get; init; }
    /// <summary>True when events after the requested cursor were already dropped from the buffer.</summary>
    public required bool Truncated { get; init; }
}
