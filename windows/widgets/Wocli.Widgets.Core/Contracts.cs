namespace Wocli.Widgets.Core;

public sealed record LyricsSnapshot(
    bool Enabled, bool Boss, string State, string TrackId,
    string Title, string Artist, string Line);

public sealed record WidgetConfig(bool Enabled, string EndpointPath);

public sealed class ProtocolException(string code) : IOException(code)
{
    public string Code { get; } = code;
}
