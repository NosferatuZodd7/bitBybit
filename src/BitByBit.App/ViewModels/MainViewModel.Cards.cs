using System.Collections.ObjectModel;
using System.Globalization;
using BitByBit.App.Infrastructure;
using BitByBit.Core.Audio;
using BitByBit.Core.QuickDiag;
using CommunityToolkit.Mvvm.Input;

namespace BitByBit.App.ViewModels;

/// <summary>Grouped cards for the list pages, rebuilt from the collected snapshot and the page collections.</summary>
public sealed partial class MainViewModel
{
    /// <summary>Long logs show only the most recent entries as cards; the full list is in the exported report.</summary>
    private const int MaxLogCards = 80;

    public ObservableCollection<CardGroup> StoreGroups { get; } = [];
    public ObservableCollection<CardGroup> BackupGroups { get; } = [];
    public ObservableCollection<CardGroup> UpdateGroups { get; } = [];
    public ObservableCollection<CardGroup> AudioGroups { get; } = [];
    public ObservableCollection<CardGroup> DisplayGroups { get; } = [];
    public ObservableCollection<CardGroup> NetworkGroups { get; } = [];
    public ObservableCollection<CardGroup> HistoryGroups { get; } = [];
    public ObservableCollection<CardGroup> ChangeGroups { get; } = [];
    public ObservableCollection<CardGroup> SystemGroups { get; } = [];

    private void RefreshStoreCards()
    {
        var used = DriverPackages.Select(p => p.Inf).ToHashSet(StringComparer.OrdinalIgnoreCase);
        var unused = (_snapshot?.DriverStorePackages ?? []).Where(p => !used.Contains(p.InfName)).Select(p => new InfoCard
        {
            Glyph = "package",
            Title = p.InfName,
            Subtitle = p.Provider,
            Tag = p.Version,
            Tone = "Neutral",
            Fields = InfoCard.FieldsOf(
                (Text.Get("app.col.driverDate"), p.Date?.ToString("yyyy-MM-dd", CultureInfo.InvariantCulture)),
                (Text.Get("app.col.class"), p.ClassName)),
            ActionText = Text.Get("app.store.remove"),
            ActionGlyph = "trash",
            ActionCommand = new AsyncRelayCommand(() => RemovePackageAsync(p.InfName)),
            ActionIsDanger = true,
        }).ToArray();
        Replace(StoreGroups, UsedStoreGroups().Concat(unused.Length == 0 ? [] :
            [new CardGroup("package", Text.Get("app.store.unused"), unused, Text.Get("app.store.unusedHint"))]));
    }

    private IEnumerable<CardGroup> UsedStoreGroups() => DriverPackages
        .GroupBy(p => DeviceRow.CategoryOf(p.ClassName))
        .OrderBy(g => DeviceRow.CategoryRank(g.Key))
        .Select(g => new CardGroup(DeviceRow.GlyphOf(g.Key), Text.Get("app.devices.category." + g.Key), g.Select(p => new InfoCard
        {
            Glyph = DeviceRow.GlyphOf(g.Key),
            Title = p.DeviceNames.Count switch
            {
                0 => p.Inf,
                1 => p.DeviceNames[0],
                _ => Text.Get("app.cards.store.devices", p.DeviceNames[0], p.DeviceNames.Count - 1),
            },
            Subtitle = p.Provider,
            Tag = p.Inf,
            Tone = p.Signed == Text.Get("common.no") ? "Warn" : "Accent",
            Fields = InfoCard.FieldsOf(
                (Text.Get("app.col.driverVersion"), p.Version),
                (Text.Get("app.col.driverDate"), p.Date),
                (Text.Get("app.col.signed"), p.Signed),
                (Text.Get("app.col.class"), p.ClassName)),
            ToolTip = string.Join(Environment.NewLine, p.DeviceNames),
        }).ToArray()));

    private void RefreshBackupCards() => Replace(BackupGroups, Backups.Count == 0 ? [] :
    [
        new CardGroup("hard-drive", Text.Get("app.page.backups.title"), Backups.Select(b => new InfoCard
        {
            Glyph = "package",
            Title = b.Name,
            Subtitle = b.Created,
            Tag = b.Size,
            Fields = InfoCard.FieldsOf(("INF", b.InfCount.ToString(CultureInfo.CurrentCulture)), (Text.Get("app.col.path"), b.Path)),
            ToolTip = b.Path,
        }).ToArray()),
    ]);

    private void RefreshUpdateCards() => Replace(UpdateGroups, DriverUpdates
        .GroupBy(u => string.IsNullOrWhiteSpace(u.DriverClass) ? Text.Get("app.devices.category.other") : u.DriverClass!)
        .OrderBy(g => g.Key, StringComparer.CurrentCulture)
        .Select(g => new CardGroup("download", g.Key, g.Select(u => new InfoCard
        {
            Glyph = "download",
            Title = u.Title,
            Subtitle = u.Provider,
            Tag = u.Source,
            Fields = InfoCard.FieldsOf(
                (Text.Get("app.col.driverDate"), u.Date),
                (Text.Get("app.col.size"), u.Size),
                (Text.Get("app.col.hardwareId"), u.HardwareId)),
        }).ToArray())));

    private void RefreshChangeCards()
    {
        InfoCard Card(ChangeRow c) => new()
        {
            Glyph = c.IsUserScope ? "user" : "server",
            Title = c.Title,
            Subtitle = $"{c.When} · {c.Scope}",
            Tag = c.Status,
            Tone = c.CanRevert ? "Accent" : "Neutral",
            Note = string.IsNullOrWhiteSpace(c.Detail) ? c.Description : c.Description + Environment.NewLine + c.Detail,
            ActionText = c.CanRevert ? Text.Get("app.changes.revert") : null,
            ActionGlyph = "rotate-ccw",
            ActionCommand = c.CanRevert ? c.RevertCommand : null,
            ActionIsDanger = true,
        };

        var active = Changes.Where(c => c.CanRevert).Select(Card).ToArray();
        var past = Changes.Where(c => !c.CanRevert).Select(Card).ToArray();
        Replace(ChangeGroups, new[]
        {
            new CardGroup("rotate-ccw", Text.Get("app.cards.changes.active"), active),
            new CardGroup("file", Text.Get("app.cards.changes.past"), past),
        }.Where(g => g.Cards.Count > 0));
    }

    /// <summary>Pages built only from the snapshot (audio, displays, network, history, system).</summary>
    private void RefreshSnapshotCards(QuickSnapshot snapshot)
    {
        Replace(AudioGroups, AudioCards(snapshot));
        Replace(DisplayGroups, DisplayCards(snapshot));
        Replace(NetworkGroups, NetworkCards(snapshot));
        Replace(HistoryGroups, HistoryCards(snapshot));
        Replace(SystemGroups, SystemCards(snapshot));
    }

    private IEnumerable<CardGroup> AudioCards(QuickSnapshot snapshot)
    {
        var states = new[]
        {
            (EndpointState.Active, "check-circle", "app.cards.audio.active", (string?)null),
            (EndpointState.Unplugged, "volume-off", "app.cards.audio.unplugged", "app.cards.audio.unpluggedHint"),
            (EndpointState.Disabled, "volume-off", "app.cards.audio.disabled", null),
            (EndpointState.NotPresent, "volume-off", "app.cards.audio.notPresent", "app.cards.audio.notPresentHint"),
        };
        foreach (var (state, glyph, title, hint) in states)
        {
            var cards = snapshot.AudioEndpoints.Where(e => e.State == state)
                .OrderBy(e => e.Flow).ThenBy(e => e.Name, StringComparer.CurrentCulture)
                .Select(e =>
                {
                    var row = new EndpointRow(e);
                    return new InfoCard
                    {
                        Glyph = e.Flow == EndpointFlow.Capture ? "mic" : e.FormFactor == 9 ? "monitor" : "volume",
                        Title = ShortEndpointName(e),
                        Subtitle = e.AdapterName,
                        Tag = string.IsNullOrEmpty(row.Default) ? row.Flow : row.Default,
                        Tone = state switch
                        {
                            EndpointState.Active => "Ok",
                            EndpointState.Unplugged or EndpointState.Disabled => "Warn",
                            _ => "Neutral",
                        },
                        Fields = InfoCard.FieldsOf((Text.Get("app.col.kind"), row.Kind), (Text.Get("app.col.flow"), row.Flow)),
                        ToolTip = e.Name,
                        IsDimmed = state == EndpointState.NotPresent,
                        // Per-user and journaled (revert in the changes page); no administrator rights needed.
                        ActionText = state switch
                        {
                            EndpointState.Active when e.Flow == EndpointFlow.Render && !e.IsDefaultConsole => Text.Get("app.audio.makeDefault"),
                            EndpointState.Disabled => Text.Get("app.audio.enable"),
                            _ => null,
                        },
                        ActionGlyph = state == EndpointState.Disabled ? "power" : "check",
                        ActionCommand = state switch
                        {
                            EndpointState.Active when e.Flow == EndpointFlow.Render && !e.IsDefaultConsole => new AsyncRelayCommand(() => SetDefaultEndpointAsync(e.Id)),
                            EndpointState.Disabled => new AsyncRelayCommand(() => SetEndpointEnabledAsync(e.Id, true)),
                            _ => null,
                        },
                    };
                }).ToArray();
            if (cards.Length > 0)
            {
                yield return new CardGroup(glyph, Text.Get(title), cards, hint is null ? null : Text.Get(hint));
            }
        }
    }

    /// <summary>"Altifalantes (Realtek High Definition Audio)" → "Altifalantes" when the adapter is shown below.</summary>
    private static string ShortEndpointName(AudioEndpointSummary e)
    {
        var name = e.Name ?? e.Id;
        var suffix = e.AdapterName is null ? null : $" ({e.AdapterName})";
        return suffix is not null && name.EndsWith(suffix, StringComparison.Ordinal) && name.Length > suffix.Length ? name[..^suffix.Length] : name;
    }

    private static IEnumerable<CardGroup> DisplayCards(QuickSnapshot snapshot)
    {
        var monitors = snapshot.Monitors.Select(m =>
        {
            var row = new MonitorRow(m);
            return new InfoCard
            {
                Glyph = "monitor",
                Title = row.Name ?? "-",
                Subtitle = row.Mode,
                Tag = row.Connection,
                Tone = m.IsDvi ? "Warn" : m.IsActive ? "Ok" : "Neutral",
                Fields = InfoCard.FieldsOf(
                    (Text.Get("app.col.audio"), row.Audio),
                    (Text.Get("app.cards.display.native"), m.EdidNativeWidth is null ? null
                        : string.Create(CultureInfo.CurrentCulture, $"{m.EdidNativeWidth}×{m.EdidNativeHeight} ({m.EdidNativePixelClockMHz:0.#} MHz)")),
                    ("EDID", m.EdidLength is null ? row.Edid : $"{row.Edid} · {m.EdidLength} bytes"),
                    (Text.Get("app.col.source"), row.Source)),
                Note = m.IsDvi ? Text.Get("app.cards.display.dviNote") : null,
            };
        }).ToArray();
        var gpus = snapshot.Gpus.Select(g => new InfoCard
        {
            Glyph = "cpu",
            Title = g.Name ?? "-",
            Subtitle = g.CurrentMode,
            Tag = g.Status,
            Tone = string.Equals(g.Status, "OK", StringComparison.OrdinalIgnoreCase) ? "Ok" : "Warn",
            Fields = InfoCard.FieldsOf(
                (Text.Get("app.col.driverVersion"), g.DriverVersion),
                (Text.Get("app.col.driverDate"), g.DriverDate?.ToString("yyyy-MM-dd", CultureInfo.InvariantCulture))),
        }).ToArray();
        return new[]
        {
            new CardGroup("monitor", Text.Get("app.display.monitors"), monitors),
            new CardGroup("cpu", Text.Get("app.display.gpus"), gpus),
        }.Where(g => g.Cards.Count > 0);
    }

    private static IEnumerable<CardGroup> NetworkCards(QuickSnapshot snapshot)
    {
        var groups = new[] { ("Up", "wifi", "app.cards.network.up"), ("Down", "power", "app.cards.network.down"), ("NotPresent", "power", "app.cards.network.notPresent") };
        string Bucket(string? status) => status switch { "Up" => "Up", "NotPresent" or "Not Present" => "NotPresent", _ => "Down" };
        foreach (var (key, glyph, title) in groups)
        {
            var cards = snapshot.NetworkAdapters.Where(n => Bucket(n.Status) == key).OrderBy(n => n.Name, StringComparer.CurrentCulture).Select(n =>
            {
                var row = new NetworkRow(n);
                return new InfoCard
                {
                    Glyph = NetworkGlyph(n.KindGuess),
                    Title = n.Name ?? n.Description ?? "-",
                    Subtitle = n.Description,
                    Tag = n.KindGuess,
                    Tone = key == "Up" ? "Ok" : "Neutral",
                    Fields = InfoCard.FieldsOf(
                        ("IPv4", row.IPv4),
                        ("Gateway", row.Gateway),
                        ("DNS", row.Dns),
                        (Text.Get("app.col.speed"), row.Speed),
                        (Text.Get("app.col.traffic"), row.Traffic)),
                    ToolTip = n.KindEvidence,
                    IsDimmed = key != "Up",
                };
            }).ToArray();
            if (cards.Length > 0)
            {
                yield return new CardGroup(glyph, Text.Get(title), cards);
            }
        }
    }

    private static string NetworkGlyph(string? kind) => kind switch
    {
        { } k when k.Contains("Wi-Fi", StringComparison.OrdinalIgnoreCase) => "wifi",
        { } k when k.Contains("Ethernet", StringComparison.OrdinalIgnoreCase) => "link",
        { } k when k.Contains("VPN", StringComparison.OrdinalIgnoreCase) || k.Contains("Tunnel", StringComparison.OrdinalIgnoreCase) => "globe",
        _ => "server",
    };

    private static IEnumerable<CardGroup> HistoryCards(QuickSnapshot snapshot)
    {
        static string Tone(string? result) => result switch
        {
            null => "Neutral",
            { } r when r.Contains("FAIL", StringComparison.OrdinalIgnoreCase) => "Error",
            { } r when r.Contains("SUCC", StringComparison.OrdinalIgnoreCase) => "Ok",
            _ => "Neutral",
        };

        static string When(DateTimeOffset? t) => t?.ToLocalTime().ToString("yyyy-MM-dd HH:mm", CultureInfo.InvariantCulture) ?? "?";

        var wu = snapshot.WindowsUpdateHistory.OrderByDescending(w => w.Date).Select(w => new InfoCard
        {
            Glyph = "download",
            Title = w.Title,
            Subtitle = When(w.Date) + (w.Operation is null ? string.Empty : " · " + w.Operation),
            Tag = w.Result,
            Tone = Tone(w.Result),
        }).ToArray();
        var setup = snapshot.SetupApiSections.OrderByDescending(s => s.Start).Take(MaxLogCards).Select(s => new InfoCard
        {
            Glyph = "terminal",
            Title = s.Header,
            Subtitle = When(s.Start),
            Tag = s.ExitStatus,
            Tone = Tone(s.ExitStatus),
            Fields = InfoCard.FieldsOf((Text.Get("app.col.command"), s.Command)),
        }).ToArray();
        var events = snapshot.PnpEvents.OrderByDescending(e => e.Time).Take(MaxLogCards).Select(e => new InfoCard
        {
            Glyph = "file",
            Title = FirstLine(e.Message),
            Subtitle = When(e.Time) + " · " + (e.Provider ?? "?"),
            Tag = e.EventId.ToString(CultureInfo.InvariantCulture),
            Tone = "Neutral",
            ToolTip = e.Message,
        }).ToArray();

        string? Capped(int shown, int total) => total > shown ? Text.Get("app.cards.history.capped", shown, total) : null;
        return new[]
        {
            new CardGroup("download", Text.Get("app.history.wu"), wu, IsList: true),
            new CardGroup("terminal", Text.Get("app.history.setupapi"), setup,
                setup.Length == 0 ? Text.Get("app.history.setupapiHint") : Capped(setup.Length, snapshot.SetupApiSections.Count), IsList: true),
            new CardGroup("file", Text.Get("app.history.events"), events, Capped(events.Length, snapshot.PnpEvents.Count), IsList: true),
        }.Where(g => g.Cards.Count > 0 || g.Hint is not null);
    }

    private static string FirstLine(string text)
    {
        var line = text.Split(['\r', '\n'], StringSplitOptions.RemoveEmptyEntries).FirstOrDefault() ?? text;
        return line.Length <= 160 ? line : line[..160] + "…";
    }

    private IEnumerable<CardGroup> SystemCards(QuickSnapshot snapshot)
    {
        var s = snapshot.System;
        yield return new CardGroup("server", Text.Get("app.cards.system.computer"),
        [
            new InfoCard
            {
                Glyph = "server",
                Title = $"{s.Manufacturer} {s.Model}".Trim(),
                Subtitle = WindowsVersion,
                Tag = s.IsElevated ? Text.Get("app.cards.system.admin") : null,
                Fields = SystemInfo.Skip(1).Where(k => !string.IsNullOrWhiteSpace(k.Value)).Select(k => new CardField(k.Key, k.Value!)).ToArray(),
            },
            new InfoCard
            {
                Glyph = "rotate-ccw",
                Title = Text.Get("app.cards.system.history"),
                Subtitle = Text.Get("app.cards.system.historyHint", Changes.Count, ActiveChangeCount),
                Tone = ActiveChangeCount > 0 ? "Warn" : "Neutral",
                ActionText = Text.Get("app.cards.system.historyOpen"),
                ActionGlyph = "chevron-right",
                ActionCommand = new RelayCommand(() => Navigate("changes")),
            },
        ], IsList: true);
        yield return new CardGroup("settings", Text.Get("app.system.services"), snapshot.Services.Select(v => new InfoCard
        {
            Glyph = "settings",
            Title = v.DisplayName ?? v.Name,
            Subtitle = v.Name,
            Tag = v.Status,
            Tone = v.Status == "Running" ? "Ok" : v.StartType == "Disabled" ? "Error" : "Neutral",
            Fields = InfoCard.FieldsOf((Text.Get("app.col.startType"), v.StartType)),
        }).ToArray());
        if (snapshot.Issues.Count > 0)
        {
            yield return new CardGroup("warning", Text.Get("app.system.issues"), snapshot.Issues.Select(i => new InfoCard
            {
                Glyph = "warning",
                Title = i.Area,
                Tone = "Warn",
                Note = i.Message,
            }).ToArray());
        }
    }
}
