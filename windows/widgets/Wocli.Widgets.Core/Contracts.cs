namespace Wocli.Widgets.Core;

public sealed record Announcement(
    string Id, string PrimaryText, string SecondaryText,
    DateTimeOffset ExpiresAt, TimeSpan Duration);

public interface IAnnouncementSink
{
    void Request(Announcement announcement);
    void Remove(string id);
}

// Text exists only in memory for the local preview. Diagnostics never persist it.
public sealed record RuntimeSnapshot(
    bool Enabled, string State, string TrackId, string Title, string Artist, string Line,
    string? AnnouncementId, DateTimeOffset? LastRequestedAt,
    DateTimeOffset? LastShownAt, DateTimeOffset? LastEngagedAt,
    string? LastError, DateTimeOffset UpdatedAt);

public sealed record LyricsSnapshot(
    bool Enabled, bool Boss, string State, string TrackId,
    string Title, string Artist, string Line);

public sealed record WidgetConfig(bool Enabled, string EndpointPath);

public sealed record RuntimeOptions
{
    public TimeSpan PollInterval { get; init; } = TimeSpan.FromMilliseconds(500);
    public TimeSpan RequestInterval { get; init; } = TimeSpan.FromSeconds(5);
    public TimeSpan IoTimeout { get; init; } = TimeSpan.FromSeconds(2);
    public TimeSpan AnnouncementLifetime { get; init; } = TimeSpan.FromSeconds(30);
    public TimeSpan AnnouncementDuration { get; init; } = TimeSpan.FromSeconds(5);
}

public sealed class ProtocolException(string code) : IOException(code)
{
    public string Code { get; } = code;
}
