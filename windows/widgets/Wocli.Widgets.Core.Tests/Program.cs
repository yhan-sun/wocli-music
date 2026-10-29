using System.Collections.Concurrent;
using System.Diagnostics;
using System.Net;
using System.Net.Sockets;
using System.Text;
using System.Text.Json;
using Wocli.Widgets.Core;

var tests = new (string Name, Func<Task> Run)[]
{
    ("authenticated bounded loopback protocol", Protocol),
    ("cancellation and deadline release TCP connections", Cancellation),
    ("config bounds and exact local Windows paths", Configuration),
    ("plain native cards, Unicode limits and guarded controls", Cards),
    ("visible lifecycle, deduplication and idle TCP suspension", BoardLifecycle),
    ("privacy clears, recovery, shell retries and private diagnostics", BoardPrivacy),
    ("guarded actions reject replay and stale library/track controls", BoardActions),
    ("server rejects a raced action and refreshes without reporting offline", BoardStaleActions),
    ("click after config read survives cancellation until the next request", BoardConfigCancellation),
    ("late responses and lifecycle callbacks cannot resurrect content", BoardRaces),
};
var failures = 0;
foreach (var (name, run) in tests)
{
    try { await run(); Console.WriteLine($"PASS {name}"); }
    catch (Exception error) { failures++; Console.Error.WriteLine($"FAIL {name}: {error}"); }
}
return failures == 0 ? 0 : 1;

static void Check(bool condition, string message)
{
    if (!condition) throw new InvalidOperationException(message);
}

static async Task Until(Func<bool> predicate, string message, int milliseconds = 3000)
{
    var started = Stopwatch.StartNew();
    while (!predicate())
    {
        if (started.ElapsedMilliseconds > milliseconds) throw new TimeoutException(message);
        await Task.Delay(5);
    }
}

static async Task Throws<T>(Func<Task> action, string message) where T : Exception
{
    try { await action(); }
    catch (T) { return; }
    throw new InvalidOperationException(message);
}

static string Reply(string line = "歌词正文", string track = "song:1", bool boss = false, bool enabled = true, string state = "playing") =>
    JsonSerializer.Serialize(new { ok = true, data = new { enabled, boss, state, track_id = track, title = "私密歌名", artist = "私密歌手", line } });

static async Task Protocol()
{
    await using var daemon = new FakeDaemon();
    using var directory = new TemporaryDirectory();
    var endpoint = directory.Write("control.endpoint", $"{daemon.Port}\n{daemon.Secret}\n");
    var client = new DaemonClient(TimeSpan.FromMilliseconds(500));
    daemon.Reply = Reply(string.Concat(Enumerable.Repeat("🎵", 240)));
    var result = await client.ReadAsync(endpoint, CancellationToken.None);
    Check(result.Line.EnumerateRunes().Count() == 240, "Unicode scalar boundary rejected");
    Check(daemon.Requests.All(value => value == "{\"op\":\"lyrics_snapshot\"}"), "wrong operation or auth framing");
    const string opaqueTrack = "  song:\u0001\t🎵 \r\n ";
    daemon.GuardActions = true;
    daemon.Reply = Reply(track: opaqueTrack);
    result = await client.ReadAsync(endpoint, CancellationToken.None);
    Check(result.TrackId == opaqueTrack, "opaque track identifier was normalized");
    await client.ActionAsync(endpoint, "next", result.TrackId, result.State, CancellationToken.None);
    Check(daemon.ExecutedActions == 1, "opaque track identifier did not round trip through guarded action");
    daemon.GuardActions = false;
    daemon.Reply = Reply(track: new string('x', 321));
    await Throws<ProtocolException>(() => client.ReadAsync(endpoint, CancellationToken.None), "oversized guard identifier accepted");
    daemon.Reply = Reply(new string('x', 241));
    await Throws<ProtocolException>(() => client.ReadAsync(endpoint, CancellationToken.None), "oversized lyric accepted");
    daemon.Reply = new string('x', DaemonClient.MaxReplyBytes + 1);
    await Throws<ProtocolException>(() => client.ReadAsync(endpoint, CancellationToken.None), "unbounded reply accepted");
    daemon.Reply = "[]";
    await Throws<ProtocolException>(() => client.ReadAsync(endpoint, CancellationToken.None), "array reply accepted");
    daemon.Reply = "{\"ok\":false,\"code\":\"stale_widget_action\",\"error\":\"private server details\"}";
    try
    {
        await client.ActionAsync(endpoint, "next", "song:1", "playing", CancellationToken.None);
        throw new InvalidOperationException("stale action response accepted");
    }
    catch (ProtocolException error) { Check(error.Code == "stale_widget_action" && !error.Message.Contains("private", StringComparison.Ordinal), "server error disclosed private diagnostics"); }
    var requests = daemon.Requests.Count;
    await Throws<ProtocolException>(() => client.ActionAsync(endpoint, "quit", "song:1", "playing", CancellationToken.None), "unlisted operation accepted");
    Check(requests == daemon.Requests.Count, "unlisted operation reached TCP");
    daemon.Reply = Reply();
    directory.Write("control.endpoint", $"{daemon.Port}\nwrong-secret\n");
    await Throws<ProtocolException>(() => client.ReadAsync(endpoint, CancellationToken.None), "authentication failure accepted");
    directory.Write("control.endpoint", $"http://example.com:{daemon.Port}\n{daemon.Secret}\n");
    await Throws<ProtocolException>(() => client.ReadAsync(endpoint, CancellationToken.None), "hostname accepted as port");
}

static async Task Cancellation()
{
    await using var daemon = new FakeDaemon { Stall = true };
    using var directory = new TemporaryDirectory();
    var endpoint = directory.Write("control.endpoint", $"{daemon.Port}\n{daemon.Secret}\n");
    var client = new DaemonClient(TimeSpan.FromSeconds(5));
    using var cancelled = new CancellationTokenSource();
    var pending = client.ReadAsync(endpoint, cancelled.Token);
    await Until(() => daemon.Active == 1, "stalled server did not receive request");
    cancelled.Cancel();
    await Throws<OperationCanceledException>(() => pending.WaitAsync(TimeSpan.FromMilliseconds(500)), "cancellation did not interrupt read");
    await Until(() => daemon.Active == 0, "cancelled socket remained open");
    client = new DaemonClient(TimeSpan.FromMilliseconds(100));
    await Throws<OperationCanceledException>(() => client.ReadAsync(endpoint, CancellationToken.None), "network deadline not applied");
    await Until(() => daemon.Active == 0, "timed-out socket remained open");
}

static async Task Configuration()
{
    using var directory = new TemporaryDirectory();
    var config = directory.Write("widgets.json", "{}");
    foreach (var path in new[] { @"C:\Users\音乐 用户\control.endpoint", @"\\?\C:\Users\音乐 用户\control.endpoint", directory.File("control.endpoint") })
    {
        directory.Write("widgets.json", JsonSerializer.Serialize(new { schema = 1, mode = "system", enabled = true, endpoint_path = path, future_field = 7 }));
        var parsed = await DaemonClient.ReadConfigAsync(config, CancellationToken.None);
        Check(parsed.EndpointPath == path, "endpoint path was changed");
    }
    foreach (var path in new[] { @"\\server\share\control.endpoint", @"\\?\UNC\server\share\control.endpoint", @"\\.\pipe\control.endpoint", "relative.endpoint", "C:\\local\npath" })
    {
        directory.Write("widgets.json", JsonSerializer.Serialize(new { schema = 1, mode = "system", enabled = true, endpoint_path = path }));
        await Throws<ProtocolException>(() => DaemonClient.ReadConfigAsync(config, CancellationToken.None), "remote/device/relative/control path accepted");
    }
    directory.Write("widgets.json", new string(' ', DaemonClient.MaxConfigBytes + 1));
    await Throws<ProtocolException>(() => DaemonClient.ReadConfigAsync(config, CancellationToken.None), "oversized config accepted");
    directory.Write("widgets.json", JsonSerializer.Serialize(new { enabled = true, endpoint_path = directory.File("control.endpoint") }));
    await Throws<ProtocolException>(() => DaemonClient.ReadConfigAsync(config, CancellationToken.None), "missing schema/mode accepted");
    foreach (var shape in new[] { "[]", "null" })
    {
        directory.Write("widgets.json", shape);
        await Throws<ProtocolException>(() => DaemonClient.ReadConfigAsync(config, CancellationToken.None), "non-object config escaped protocol validation");
    }
}

static Task Cards()
{
    var malicious = "[x](https://example.com) \"x\"";
    var lyrics = new LyricsSnapshot(true, false, "playing", "track:1", malicious, malicious, string.Concat(Enumerable.Repeat("🎵", 240)));
    foreach (var size in Enum.GetValues<CardSize>())
    {
        var card = WidgetCards.Create(size, "playing", lyrics, new string('a', 32));
        using var template = JsonDocument.Parse(card.Template);
        using var data = JsonDocument.Parse(card.Data);
        Check(!card.Template.Contains("example.com", StringComparison.Ordinal) && !card.Template.Contains("WebURL", StringComparison.Ordinal), "private input was interpreted as template content");
        Check(!card.Template.Contains("fontFamily", StringComparison.Ordinal) && !card.Template.Contains("color", StringComparison.Ordinal), "card overrides system font or color");
        var body = template.RootElement.GetProperty("body").EnumerateArray().ToArray();
        Check(body.Where(item => item.GetProperty("type").GetString() != "ActionSet").All(item => item.GetProperty("type").GetString() == "RichTextBlock"), "music content can be parsed as Markdown");
        var actions = body.Last().GetProperty("actions").EnumerateArray().ToArray();
        Check(actions.Length == (size == CardSize.Small ? 1 : 3) && actions.All(action => action.GetProperty("type").GetString() == "Action.Execute"), "incorrect touch target budget or unsupported widget action type");
        Check(actions.Select(action => action.GetProperty("verb").GetString()).SequenceEqual(size == CardSize.Small ? new[] { "pause" } : new[] { "prev", "pause", "next" }), "incorrect action whitelist");
        Check(data.RootElement.GetProperty("line").GetString()!.EnumerateRunes().Count() <= (size == CardSize.Small ? 18 : size == CardSize.Medium ? 52 : 112), "lyric layout exceeds Unicode limit");
        Check(body.First().GetProperty("inlines")[0].GetProperty("size").GetString() == "Default", "title consumes excess card height");
        var longText = string.Concat(Enumerable.Repeat("🎵", 240));
        var longCard = WidgetCards.Create(size, "playing", lyrics with { Title = longText, Artist = longText }, new string('a', 32));
        Check(Field(longCard, "title").EnumerateRunes().Count() == (size == CardSize.Small ? 12 : size == CardSize.Medium ? 24 : 32), "title layout budget or Unicode truncation incorrect");
        Check(Field(longCard, "artist").EnumerateRunes().Count() == (size == CardSize.Small ? 0 : size == CardSize.Medium ? 24 : 40), "artist layout budget incorrect");
        Check(Field(longCard, "title").EndsWith("…", StringComparison.Ordinal), "truncated title lacks ellipsis");
        if (size == CardSize.Large) Check(data.RootElement.GetProperty("title").GetString() == malicious, "plain text altered or interpreted");
    }
    foreach (var hidden in new[] { lyrics with { Boss = true }, lyrics with { Enabled = false }, lyrics with { State = "stopped" } })
    {
        var card = WidgetCards.Create(CardSize.Large, "boss", hidden, new string('a', 32));
        Check(!card.Template.Contains("ActionSet", StringComparison.Ordinal) && !card.Data.Contains("example.com", StringComparison.Ordinal), "privacy card retains controls or music");
    }
    return Task.CompletedTask;
}

static BoardRuntime MakeBoard(RecordingWidgetSink sink, TemporaryDirectory directory, string endpoint)
{
    var config = directory.Write("widgets.json", JsonSerializer.Serialize(new { schema = 1, mode = "system", enabled = true, endpoint_path = endpoint }));
    return new BoardRuntime(sink, config, directory.File("status.json"), new BoardOptions { PollInterval = TimeSpan.FromMilliseconds(20), IoTimeout = TimeSpan.FromMilliseconds(450) });
}

static void Configure(TemporaryDirectory directory, string endpoint, bool enabled = true) =>
    directory.Write("widgets.json", JsonSerializer.Serialize(new { schema = 1, mode = "system", enabled, endpoint_path = endpoint }));

static string Field(WidgetCard card, string name)
{
    using var document = JsonDocument.Parse(card.Data);
    return document.RootElement.GetProperty(name).GetString()!;
}

static bool HasLine(RecordingWidgetSink sink, string line, string id = "widget") => sink.Latest.TryGetValue(id, out var card) && Field(card, "line") == line;
static JsonElement Nonce(WidgetCard card)
{
    using var document = JsonDocument.Parse(JsonSerializer.Serialize(new { nonce = Field(card, "nonce") }));
    return document.RootElement.Clone();
}
static int ActionCount(FakeDaemon daemon) => daemon.Requests.Count(request => request.Contains("widget_action", StringComparison.Ordinal));

static async Task BoardLifecycle()
{
    await using var daemon = new FakeDaemon { Reply = Reply("第一句") };
    using var directory = new TemporaryDirectory();
    var endpoint = directory.Write("control.endpoint", $"{daemon.Port}\n{daemon.Secret}\n");
    var sink = new RecordingWidgetSink();
    await using var runtime = MakeBoard(sink, directory, endpoint);
    runtime.Restore("widget", CardSize.Small);
    runtime.Start(); runtime.Start();
    await Until(() => sink.Latest.ContainsKey("widget"), "restored card not initialized");
    await Task.Delay(100);
    Check(daemon.Requests.IsEmpty, "inactive board polled the daemon");
    runtime.Activate("widget"); runtime.Activate("widget");
    await Until(() => HasLine(sink, "第一句"), "active board did not receive lyrics");
    var updates = sink.Updates.Count;
    await Task.Delay(150);
    Check(sink.Updates.Count == updates, "unchanged card rewritten every poll");
    daemon.Reply = Reply("第二句");
    await Until(() => HasLine(sink, "第二句"), "new lyric did not update");
    runtime.Upsert("widget", CardSize.Large, true);
    await Until(() => sink.Updates.Count > updates + 1, "size change ignored");
    runtime.Deactivate("widget");
    await Until(() => HasLine(sink, "") && daemon.Active == 0, "inactive board retained lyrics or socket");
    var requests = daemon.Requests.Count;
    await Task.Delay(120);
    Check(daemon.Requests.Count == requests, "hidden board continued IPC");
    runtime.Delete("widget");
    Check(runtime.GetSnapshot().WidgetCount == 0, "deleted widget retained state");
    runtime.Restore("second", CardSize.Medium, true);
    await Until(() => HasLine(sink, "第二句", "second"), "active restore failed");
    await runtime.StopAsync();
    Check(sink.PeakConcurrent == 1 && daemon.PeakActive == 1, "duplicate worker or overlapping shell/TCP operation");
    Check(HasLine(sink, "", "second"), "shutdown left private card cached");
}

static async Task BoardPrivacy()
{
    await using var daemon = new FakeDaemon { Reply = Reply("隐私歌词") };
    using var directory = new TemporaryDirectory();
    var endpoint = directory.Write("control.endpoint", $"{daemon.Port}\n{daemon.Secret}\n");
    var sink = new RecordingWidgetSink();
    await using var runtime = MakeBoard(sink, directory, endpoint);
    runtime.Upsert("widget", CardSize.Medium, true); runtime.Start();
    await Until(() => HasLine(sink, "隐私歌词"), "initial lyric missing");
    foreach (var condition in new[] { (Reply(boss: true), "boss"), (Reply(enabled: false), "disabled"), (Reply(state: "stopped"), "stopped"), ("[]", "offline") })
    {
        daemon.Reply = condition.Item1;
        await Until(() => runtime.GetSnapshot().State == condition.Item2 && HasLine(sink, ""), "privacy state did not clear card: " + condition.Item2);
        Check(!sink.Latest["widget"].Template.Contains("ActionSet", StringComparison.Ordinal), "hidden card retained playback actions");
        daemon.Reply = Reply("隐私歌词");
        await Until(() => HasLine(sink, "隐私歌词"), "worker did not recover");
    }
    sink.FailUpdate = true;
    daemon.Reply = Reply("重试歌词");
    await Until(() => runtime.GetSnapshot().LastError == "widget_update_failed", "shell error not recorded");
    sink.FailUpdate = false;
    await Until(() => HasLine(sink, "重试歌词"), "shell update failure was not retried");
    Configure(directory, endpoint, false);
    await Until(() => runtime.GetSnapshot().State == "disabled" && HasLine(sink, ""), "config disable retained cached text");
    await Until(() => System.IO.File.Exists(directory.File("status.json")), "missing diagnostics");
    var diagnostics = await System.IO.File.ReadAllTextAsync(directory.File("status.json"));
    using (var document = JsonDocument.Parse(diagnostics))
        Check(document.RootElement.GetProperty("mode").GetString() == "system-widget-card", "diagnostics use stale Feed mode");
    Check(!diagnostics.Contains("隐私", StringComparison.Ordinal) && !diagnostics.Contains("私密", StringComparison.Ordinal) && !diagnostics.Contains(daemon.Secret, StringComparison.Ordinal) && !diagnostics.Contains("song:1", StringComparison.Ordinal), "diagnostics persisted private metadata or auth token");
    Configure(directory, endpoint);
    daemon.Stall = true;
    await Until(() => runtime.GetSnapshot().State == "offline", "timeout did not clear card");
    await runtime.StopAsync();
    await Until(() => daemon.Active == 0, "shutdown retained stalled socket");
}

static async Task BoardActions()
{
    await using var daemon = new FakeDaemon { Reply = Reply("第一句") };
    await using var other = new FakeDaemon { Reply = Reply("另一曲库") };
    using var directory = new TemporaryDirectory();
    var endpoint = directory.Write("control.endpoint", $"{daemon.Port}\n{daemon.Secret}\n");
    var otherEndpoint = directory.Write("other.endpoint", $"{other.Port}\n{other.Secret}\n");
    var sink = new RecordingWidgetSink();
    await using var runtime = MakeBoard(sink, directory, endpoint);
    runtime.Upsert("widget", CardSize.Medium, true); runtime.Start();
    await Until(() => HasLine(sink, "第一句"), "initial controls missing");
    var first = Nonce(sink.Latest["widget"]);
    await runtime.HandleActionAsync("widget", "quit", first);
    await runtime.HandleActionAsync("widget", "resume", first);
    using var forged = JsonDocument.Parse("{\"nonce\":\"00000000000000000000000000000000\"}");
    await runtime.HandleActionAsync("widget", "next", forged.RootElement);
    await Task.Delay(60);
    Check(ActionCount(daemon) == 0, "arbitrary action or forged nonce reached daemon");
    await runtime.HandleActionAsync("widget", "next", first);
    await runtime.HandleActionAsync("widget", "next", first);
    await Until(() => ActionCount(daemon) == 1, "valid action never reached daemon");
    using (var request = JsonDocument.Parse(daemon.Requests.Single(value => value.Contains("widget_action", StringComparison.Ordinal))))
    {
        Check(request.RootElement.GetProperty("track_id").GetString() == "song:1" && request.RootElement.GetProperty("state").GetString() == "playing" && request.RootElement.GetProperty("action").GetString() == "next", "action lost atomic daemon guards");
    }
    await Task.Delay(70);
    Check(ActionCount(daemon) == 1, "replayed action was executed twice");
    daemon.Reply = Reply("新歌曲", track: "song:2");
    await Until(() => HasLine(sink, "新歌曲"), "new track missing");
    await runtime.HandleActionAsync("widget", "next", first);
    await Task.Delay(60);
    Check(ActionCount(daemon) == 1, "old card controlled new track");
    var oldLibrary = Nonce(sink.Latest["widget"]);
    Configure(directory, otherEndpoint);
    await runtime.HandleActionAsync("widget", "next", oldLibrary);
    await Until(() => HasLine(sink, "另一曲库"), "library switch missing");
    Check(ActionCount(daemon) == 1 && ActionCount(other) == 0, "old control operated after library change");
    other.Reply = Reply("暂停歌词", state: "paused");
    await Until(() => HasLine(sink, "暂停歌词"), "pause lost metadata");
    Check(sink.Latest["widget"].Template.Contains("resume", StringComparison.Ordinal), "paused card lacks resume action");
    var paused = Nonce(sink.Latest["widget"]);
    await runtime.HandleActionAsync("widget", "resume", paused);
    await Until(() => ActionCount(other) == 1, "resume not delivered");
    runtime.Deactivate("widget");
    await runtime.HandleActionAsync("widget", "next", paused);
    await Task.Delay(60);
    Check(ActionCount(other) == 1, "inactive card accepted action");
}

static async Task BoardStaleActions()
{
    await using var daemon = new FakeDaemon { Reply = Reply("旧歌词"), GuardActions = true };
    using var directory = new TemporaryDirectory();
    var endpoint = directory.Write("control.endpoint", $"{daemon.Port}\n{daemon.Secret}\n");
    var sink = new RecordingWidgetSink();
    await using var runtime = MakeBoard(sink, directory, endpoint);
    runtime.Upsert("widget", CardSize.Medium, true); runtime.Start();
    await Until(() => HasLine(sink, "旧歌词"), "initial card missing");
    var oldNonce = Nonce(sink.Latest["widget"]);
    var states = new ConcurrentQueue<string>();
    sink.OnUpdate = (_, _) => states.Enqueue(runtime.GetSnapshot().State);
    // The player changes after the click has passed the Core checks, before
    // its guarded action is checked inside the daemon's serialized handler.
    daemon.BeforeAction = () => daemon.Reply = Reply("新歌词", track: "song:2", state: "paused");
    await runtime.HandleActionAsync("widget", "next", oldNonce);
    await Until(() => states.Contains("refreshing") && HasLine(sink, "新歌词"), "stale action did not immediately refresh the card");
    Check(!states.Contains("offline") && daemon.ExecutedActions == 0, "stale control executed or was reported as a connection failure");
    Check(sink.Latest["widget"].Template.Contains("resume", StringComparison.Ordinal), "refreshed paused card has wrong controls");
    var actions = ActionCount(daemon);
    await runtime.HandleActionAsync("widget", "next", oldNonce);
    await Task.Delay(40);
    Check(ActionCount(daemon) == actions, "old card nonce remained valid after refresh");
    daemon.BeforeAction = null;
    await runtime.HandleActionAsync("widget", "resume", Nonce(sink.Latest["widget"]));
    await Until(() => daemon.ExecutedActions == 1, "fresh control did not work after stale-action recovery");
}

static async Task BoardConfigCancellation()
{
    await using var daemon = new FakeDaemon { Reply = Reply("点击竞态"), GuardActions = true };
    using var directory = new TemporaryDirectory();
    var endpoint = directory.Write("control.endpoint", $"{daemon.Port}\n{daemon.Secret}\n");
    Configure(directory, endpoint);
    var sink = new RecordingWidgetSink();
    BoardRuntime? runtime = null;
    JsonElement nonce = default;
    var armed = 0;
    async Task<WidgetConfig> ReadConfig(string path, CancellationToken token)
    {
        var config = await DaemonClient.ReadConfigAsync(path, token);
        // Deterministically deliver the click after file I/O succeeded but
        // before the worker consumes pending actions under its state lock.
        if (Interlocked.Exchange(ref armed, 0) == 1)
            await runtime!.HandleActionAsync("widget", "next", nonce);
        return config;
    }
    await using var owned = new BoardRuntime(sink, directory.File("widgets.json"), directory.File("status.json"),
        new BoardOptions { PollInterval = TimeSpan.FromMilliseconds(20), IoTimeout = TimeSpan.FromMilliseconds(450) }, ReadConfig);
    runtime = owned;
    runtime.Upsert("widget", CardSize.Medium, true); runtime.Start();
    await Until(() => HasLine(sink, "点击竞态"), "initial controls missing");
    nonce = Nonce(sink.Latest["widget"]);
    var states = new ConcurrentQueue<string>();
    sink.OnUpdate = (_, _) => states.Enqueue(runtime.GetSnapshot().State);
    Volatile.Write(ref armed, 1);
    await Until(() => daemon.ExecutedActions == 1, "cancelled config iteration consumed and lost the queued click");
    Check(ActionCount(daemon) == 1 && !states.Contains("offline"), "click was duplicated or cancellation was reported offline");
}

static async Task BoardRaces()
{
    await using var first = new FakeDaemon { Reply = Reply("迟到旧歌词"), Delay = 200 };
    await using var second = new FakeDaemon { Reply = Reply("正确新歌词") };
    using var directory = new TemporaryDirectory();
    var oldEndpoint = directory.Write("old.endpoint", $"{first.Port}\n{first.Secret}\n");
    var newEndpoint = directory.Write("new.endpoint", $"{second.Port}\n{second.Secret}\n");
    var sink = new RecordingWidgetSink();
    await using var runtime = MakeBoard(sink, directory, oldEndpoint);
    runtime.Upsert("widget", CardSize.Medium, true); runtime.Start();
    await Until(() => first.Active == 1, "old daemon never requested");
    Configure(directory, newEndpoint);
    await Until(() => HasLine(sink, "正确新歌词"), "new library never sampled");
    Check(sink.Updates.All(update => Field(update.Card, "line") != "迟到旧歌词"), "late result from old library was published");
    sink.OnUpdate = (_, card) => { if (Field(card, "line") == "取消竞态") runtime.Deactivate("widget"); };
    second.Reply = Reply("取消竞态");
    await Until(() => runtime.GetSnapshot().ActiveCount == 0 && HasLine(sink, ""), "deactivate inside shell callback resurrected lyrics");
    sink.OnUpdate = null;
    second.Stall = true;
    runtime.Activate("widget");
    await Until(() => second.Active == 1, "new read did not stall");
    runtime.Deactivate("widget");
    await Until(() => second.Active == 0 && HasLine(sink, ""), "deactivate did not cancel socket immediately", 250);
    Check(sink.PeakConcurrent == 1, "callback reentered a shell update");
}

sealed class TemporaryDirectory : IDisposable
{
    private readonly string _path = Path.Combine(Path.GetTempPath(), "wocli-widget-tests-" + Guid.NewGuid().ToString("N"));
    public TemporaryDirectory() => Directory.CreateDirectory(_path);
    public string File(string name) => Path.Combine(_path, name);
    public string Write(string name, string value)
    {
        var path = File(name);
        System.IO.File.WriteAllText(path + ".tmp", value);
        System.IO.File.Move(path + ".tmp", path, true);
        return path;
    }
    public void Dispose() => Directory.Delete(_path, true);
}

sealed class RecordingWidgetSink : IWidgetSink
{
    public ConcurrentQueue<(string Id, WidgetCard Card)> Updates { get; } = new();
    public ConcurrentDictionary<string, WidgetCard> Latest { get; } = new();
    public volatile bool FailUpdate;
    public Action<string, WidgetCard>? OnUpdate;
    private int _active;
    public int PeakConcurrent;
    public void Update(string id, WidgetCard card)
    {
        var active = Interlocked.Increment(ref _active);
        PeakConcurrent = Math.Max(PeakConcurrent, active);
        try
        {
            if (FailUpdate) throw new IOException("test shell failure");
            Updates.Enqueue((id, card)); Latest[id] = card; OnUpdate?.Invoke(id, card);
        }
        finally { Interlocked.Decrement(ref _active); }
    }
}

sealed class FakeDaemon : IAsyncDisposable
{
    private readonly TcpListener _listener = new(IPAddress.Loopback, 0);
    private readonly CancellationTokenSource _closed = new();
    private readonly ConcurrentBag<Task> _clients = new();
    private readonly Task _accept;
    public string Secret { get; } = new('a', 64);
    public volatile string Reply = "{}";
    public volatile bool Stall;
    public volatile int Delay;
    public volatile bool GuardActions;
    public Action? BeforeAction;
    public int ExecutedActions;
    public ConcurrentQueue<string> Requests { get; } = new();
    public int Active;
    public int PeakActive;
    public int Port => ((IPEndPoint)_listener.LocalEndpoint).Port;
    public FakeDaemon()
    {
        _listener.Start();
        _accept = Accept();
    }
    private async Task Accept()
    {
        try
        {
            while (!_closed.IsCancellationRequested)
            {
                var client = await _listener.AcceptTcpClientAsync(_closed.Token);
                _clients.Add(Handle(client));
            }
        }
        catch (OperationCanceledException) { }
    }
    private async Task Handle(TcpClient client)
    {
        using (client)
        {
            var active = Interlocked.Increment(ref Active);
            PeakActive = Math.Max(PeakActive, active);
            try
            {
                await using var stream = client.GetStream();
                using var reader = new StreamReader(stream, Encoding.UTF8, leaveOpen: true);
                var token = await reader.ReadLineAsync(_closed.Token);
                var request = await reader.ReadLineAsync(_closed.Token);
                Requests.Enqueue(request ?? "");
                if (Stall) { await stream.ReadAsync(new byte[1], _closed.Token); return; }
                var reply = token == Secret ? Reply : "{\"ok\":false,\"error\":\"auth rejected\"}";
                if (token == Secret && GuardActions && request is not null)
                {
                    using var command = JsonDocument.Parse(request);
                    if (command.RootElement.GetProperty("op").GetString() == "widget_action")
                    {
                        BeforeAction?.Invoke();
                        reply = Reply;
                        using var current = JsonDocument.Parse(reply);
                        var data = current.RootElement.GetProperty("data");
                        if (command.RootElement.GetProperty("track_id").GetString() != data.GetProperty("track_id").GetString() ||
                            command.RootElement.GetProperty("state").GetString() != data.GetProperty("state").GetString())
                            reply = "{\"ok\":false,\"code\":\"stale_widget_action\"}";
                        else Interlocked.Increment(ref ExecutedActions);
                    }
                }
                if (Delay > 0) await Task.Delay(Delay, _closed.Token);
                await stream.WriteAsync(Encoding.UTF8.GetBytes(reply + "\n"), _closed.Token);
            }
            catch (Exception error) when (error is IOException or OperationCanceledException or SocketException) { }
            finally { Interlocked.Decrement(ref Active); }
        }
    }
    public async ValueTask DisposeAsync()
    {
        _closed.Cancel();
        _listener.Stop();
        await _accept;
        await Task.WhenAll(_clients);
        _closed.Dispose();
    }
}
