using BitByBit.Core.Common;
using BitByBit.Core.Journal;
using Microsoft.Extensions.Logging.Abstractions;

namespace BitByBit.Core.Tests.Foundation;

public sealed class JournalTests : IDisposable
{
    private readonly string _dir = Path.Combine(Path.GetTempPath(), "bitbybit-tests", Guid.NewGuid().ToString("N"));

    public void Dispose()
    {
        if (Directory.Exists(_dir))
        {
            Directory.Delete(_dir, recursive: true);
        }
    }

    private static ChangeRecord NewRecord(string kind = "test", string id = "obj1", DateTimeOffset? at = null, bool recovery = true) => new()
    {
        Id = Guid.NewGuid(),
        CreatedAt = at ?? DateTimeOffset.UtcNow,
        Category = ChangeCategory.Network,
        TargetKind = kind,
        TargetId = id,
        Description = "test change",
        AppliedState = "{\"v\":1}",
        RevertOnRecovery = recovery,
    };

    [Fact]
    public async Task Persists_and_replays_records()
    {
        var path = Path.Combine(_dir, "changes.jsonl");
        var journal = new JsonLinesChangeJournal(path);
        var record = NewRecord();
        await journal.AppendAsync(record, CancellationToken.None);
        await journal.UpdateStatusAsync(record.Id, ChangeStatus.Reverted, null, CancellationToken.None);

        var reopened = new JsonLinesChangeJournal(path);
        var all = await reopened.GetAllAsync(CancellationToken.None);
        var single = Assert.Single(all);
        Assert.Equal(ChangeStatus.Reverted, single.Status);
        Assert.Empty(await reopened.GetActiveAsync(CancellationToken.None));
    }

    [Fact]
    public async Task Ignores_torn_trailing_line()
    {
        var path = Path.Combine(_dir, "changes.jsonl");
        var journal = new JsonLinesChangeJournal(path);
        await journal.AppendAsync(NewRecord(), CancellationToken.None);
        await File.AppendAllTextAsync(path, "{\"id\":\"truncat", TestContext.Current.CancellationToken);

        var reopened = new JsonLinesChangeJournal(path);
        Assert.Single(await reopened.GetAllAsync(CancellationToken.None));
    }

    [Fact]
    public async Task Rejects_duplicate_ids()
    {
        var journal = new JsonLinesChangeJournal(Path.Combine(_dir, "changes.jsonl"));
        var record = NewRecord();
        await journal.AppendAsync(record, CancellationToken.None);
        await Assert.ThrowsAsync<InvalidOperationException>(() => journal.AppendAsync(record, CancellationToken.None));
    }

    private sealed class FakeHandler(LiveStateComparison comparison, bool revertSucceeds = true) : IChangeRevertHandler
    {
        public List<Guid> Reverted { get; } = [];
        public string TargetKind => "test";

        public Task<LiveStateComparison> CompareAsync(ChangeRecord record, CancellationToken cancellationToken) => Task.FromResult(comparison);

        public Task<Result> RevertAsync(ChangeRecord record, CancellationToken cancellationToken)
        {
            if (!revertSucceeds)
            {
                return Task.FromResult(Result.Failure(ErrorCode.PlatformError, "boom"));
            }

            Reverted.Add(record.Id);
            return Task.FromResult(Result.Success());
        }
    }

    [Theory]
    [InlineData(LiveStateComparison.MatchesApplied, ChangeStatus.Reverted, true)]
    [InlineData(LiveStateComparison.Drifted, ChangeStatus.DriftDetected, false)]
    [InlineData(LiveStateComparison.Unknown, ChangeStatus.DriftDetected, false)]
    [InlineData(LiveStateComparison.Gone, ChangeStatus.TargetGone, false)]
    public async Task Recovery_only_reverts_when_live_state_matches(LiveStateComparison comparison, ChangeStatus expected, bool expectRevertCall)
    {
        var journal = new JsonLinesChangeJournal(Path.Combine(_dir, "changes.jsonl"));
        var record = NewRecord();
        await journal.AppendAsync(record, CancellationToken.None);
        var handler = new FakeHandler(comparison);
        var coordinator = new RecoveryCoordinator(journal, [handler], NullLogger<RecoveryCoordinator>.Instance);

        var outcomes = await coordinator.RecoverAfterRestartAsync(new HashSet<Guid>(), CancellationToken.None);

        Assert.Equal(expected, Assert.Single(outcomes).NewStatus);
        Assert.Equal(expectRevertCall, handler.Reverted.Contains(record.Id));
        Assert.Equal(expected, (await journal.GetAllAsync(CancellationToken.None)).Single().Status);
    }

    [Fact]
    public async Task Recovery_skips_changes_of_live_sessions_and_non_recoverable_changes()
    {
        var journal = new JsonLinesChangeJournal(Path.Combine(_dir, "changes.jsonl"));
        var live = Guid.NewGuid();
        var liveRecord = NewRecord() with { CorrelationId = live };
        var persistent = NewRecord(recovery: false);
        await journal.AppendAsync(liveRecord, CancellationToken.None);
        await journal.AppendAsync(persistent, CancellationToken.None);
        var handler = new FakeHandler(LiveStateComparison.MatchesApplied);
        var coordinator = new RecoveryCoordinator(journal, [handler], NullLogger<RecoveryCoordinator>.Instance);

        var outcomes = await coordinator.RecoverAfterRestartAsync(new HashSet<Guid> { live }, CancellationToken.None);

        Assert.Empty(outcomes);
        Assert.Empty(handler.Reverted);
    }

    [Fact]
    public async Task Reverts_newest_first()
    {
        var journal = new JsonLinesChangeJournal(Path.Combine(_dir, "changes.jsonl"));
        var t0 = DateTimeOffset.UtcNow;
        var first = NewRecord(at: t0);
        var second = NewRecord(at: t0.AddSeconds(1));
        await journal.AppendAsync(first, CancellationToken.None);
        await journal.AppendAsync(second, CancellationToken.None);
        var handler = new FakeHandler(LiveStateComparison.MatchesApplied);
        var coordinator = new RecoveryCoordinator(journal, [handler], NullLogger<RecoveryCoordinator>.Instance);

        await coordinator.CleanupForUninstallAsync(CancellationToken.None);

        Assert.Equal([second.Id, first.Id], handler.Reverted);
    }

    [Fact]
    public async Task Missing_handler_marks_revert_failed()
    {
        var journal = new JsonLinesChangeJournal(Path.Combine(_dir, "changes.jsonl"));
        await journal.AppendAsync(NewRecord(kind: "unknown-kind"), CancellationToken.None);
        var coordinator = new RecoveryCoordinator(journal, [], NullLogger<RecoveryCoordinator>.Instance);

        var outcome = Assert.Single(await coordinator.CleanupForUninstallAsync(CancellationToken.None));
        Assert.Equal(ChangeStatus.RevertFailed, outcome.NewStatus);
    }
}
