using System.Text.Json;

namespace Wocli.Widgets.Core;

public enum CardSize { Small, Medium, Large }
public sealed record WidgetCard(string Template, string Data);
public interface IWidgetSink { void Update(string widgetId, WidgetCard card); }
public sealed record BoardRuntimeSnapshot(bool Enabled, string State, int WidgetCount, int ActiveCount, string? LastError);
public sealed record BoardOptions
{
    public TimeSpan PollInterval { get; init; } = TimeSpan.FromMilliseconds(500);
    public TimeSpan IoTimeout { get; init; } = TimeSpan.FromSeconds(2);
}

public static class WidgetCards
{
    public static WidgetCard Create(CardSize size, string state, LyricsSnapshot? lyrics = null, string nonce = "")
    {
        var content = lyrics is { Enabled: true, Boss: false, State: "playing" or "paused" } && lyrics.TrackId.Length > 0;
        var controls = content && nonce.Length > 0;
        var body = new List<object>();
        // TextRun is plain text. Music metadata never enters template JSON or a
        // Markdown-capable TextBlock, so lyrics cannot introduce links or HTML.
        if (content)
        {
            body.Add(Rich("${title}", "Bolder", size == CardSize.Small ? "Default" : "Medium"));
            if (size != CardSize.Small) body.Add(Rich("${artist}", "Default", "Small", subtle: true));
            body.Add(Rich("${line}", "Default", size == CardSize.Large ? "Large" : "Default"));
        }
        else
        {
            body.Add(new { type = "TextBlock", text = "wocli-music", weight = "Bolder", wrap = true });
            body.Add(new { type = "TextBlock", text = StatusText(state), wrap = true, maxLines = size == CardSize.Small ? 2 : 4, isSubtle = true });
        }
        if (controls)
        {
            body.Add(new
            {
                type = "ActionSet",
                actions = new[]
                {
                    Action("上一首", "prev"),
                    Action(lyrics!.State == "paused" ? "播放" : "暂停", lyrics.State == "paused" ? "resume" : "pause"),
                    Action("下一首", "next")
                }
            });
        }
        var template = JsonSerializer.Serialize(new { type = "AdaptiveCard", version = "1.5", body });
        var data = JsonSerializer.Serialize(new
        {
            title = content ? Limit(lyrics!.Title, size == CardSize.Small ? 28 : 80) : "",
            artist = content ? Limit(lyrics!.Artist, 60) : "",
            line = content ? Limit(lyrics!.Line.Length > 0 ? lyrics.Line : "暂无同步歌词", size switch { CardSize.Small => 44, CardSize.Medium => 100, _ => 200 }) : "",
            nonce = controls ? nonce : ""
        });
        return new WidgetCard(template, data);
    }

    private static object Rich(string text, string weight, string size, bool subtle = false) => new
    {
        type = "RichTextBlock", spacing = "Small",
        inlines = new[] { new { type = "TextRun", text, weight, size, isSubtle = subtle } }
    };

    private static object Action(string title, string verb) => new
    {
        type = "Action.Execute", title, verb, data = new { nonce = "${nonce}" }
    };

    private static string Limit(string text, int maxRunes)
    {
        var runes = text.EnumerateRunes().ToArray();
        return runes.Length <= maxRunes ? text : string.Concat(runes.Take(maxRunes - 1).Select(rune => rune.ToString())) + "…";
    }

    private static string StatusText(string state) => state switch
    {
        "boss" => "歌词已隐藏",
        "disabled" => "请在播放器中开启系统小组件歌词",
        "offline" => "播放器未连接",
        "stopped" => "播放音乐后在这里查看歌词",
        "inactive" => "展开小组件面板后刷新歌词",
        "switching_source" => "正在切换播放器",
        "refreshing" => "正在刷新播放状态",
        _ => "正在连接播放器"
    };
}
