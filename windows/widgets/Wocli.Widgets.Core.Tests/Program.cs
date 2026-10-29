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
    ("latest-only pacing, privacy and stale shown callbacks", Runtime),
    ("offline recovery and serialized removal retry", Recovery),
    ("endpoint changes, repeated verses and disable during request", Switching),
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
    daemon.Reply = Reply(new string('x', 241));
    await Throws<ProtocolException>(() => client.ReadAsync(endpoint, CancellationToken.None), "oversized lyric accepted");
    daemon.Reply = new string('x', DaemonClient.MaxReplyBytes + 1);
    await Throws<ProtocolException>(() => client.ReadAsync(endpoint, CancellationToken.None), "unbounded reply accepted");
    daemon.Reply = "[]";
    await Throws<ProtocolException>(() => client.ReadAsync(endpoint, CancellationToken.None), "array reply accepted");
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

static async Task Runtime()
{
    await using var daemon = new FakeDaemon { Reply = Reply("A") };
    using var directory = new TemporaryDirectory();
    var endpoint = directory.Write("control.endpoint", $"{daemon.Port}\n{daemon.Secret}\n");
    var config = directory.Write("widgets.json", JsonSerializer.Serialize(new { schema = 1, mode = "system", enabled = true, endpoint_path = endpoint }));
    var status = directory.File("widgets-status.json");
    var sink = new RecordingSink();
    await using var loop = new RuntimeLoop(sink, config, status, new RuntimeOptions
    {
        PollInterval = TimeSpan.FromMilliseconds(15), RequestInterval = TimeSpan.FromMilliseconds(400), IoTimeout = TimeSpan.FromMilliseconds(300)
    });
    loop.SetEnabled(true);
    loop.Start();
    loop.Start();
    await Until(() => sink.Requested.Count == 1, "first lyric never requested");
    var first = sink.Requested.Single();
    Check(loop.GetSnapshot().State == "requested", "request was misreported as shown");
    loop.MarkShown(first.Id);
    loop.MarkShown(first.Id);
    foreach (var line in new[] { "B", "C", "D" })
    {
        daemon.Reply = Reply(line);
        await Until(() => loop.GetSnapshot().Line == line, "latest lyric not sampled");
    }
    await Until(() => sink.Requested.Count == 2, "latest lyric never requested after pacing delay");
    Check(sink.Requested.Select(value => value.PrimaryText).SequenceEqual(new[] { "A", "D" }), "intermediate lyrics accumulated in a queue");
    Check(sink.Removed.Contains(first.Id), "superseded line not removed");
    await Task.Delay(450);
    Check(sink.Requested.Count == 2, "unchanged line requested repeatedly");
    var second = sink.Requested.Last();
    daemon.Reply = Reply("D", boss: true);
    await Until(() => loop.GetSnapshot().State == "boss", "boss mode ignored");
    Check(loop.GetSnapshot().Line == "" && loop.GetSnapshot().Title == "", "boss mode retained private text");
    var removals = sink.Removed.Count;
    loop.MarkShown(second.Id);
    await Until(() => sink.Removed.Count > removals, "late shell display was not retracted");
    await Until(() => System.IO.File.Exists(status), "diagnostics not written");
    using (var document = JsonDocument.Parse(await System.IO.File.ReadAllTextAsync(status)))
    {
        var root = document.RootElement;
        Check(root.GetProperty("shown_count").GetInt64() == 1, "duplicate/stale Shown callback incremented displayed count");
        Check(!root.TryGetProperty("title", out _) && !root.TryGetProperty("line", out _) && !root.TryGetProperty("track_id", out _), "diagnostics persisted private music text");
        Check(!document.RootElement.GetRawText().Contains(daemon.Secret, StringComparison.Ordinal), "diagnostics leaked authentication token");
    }
    daemon.Reply = Reply("D", track: "song:2");
    await Until(() => sink.Requested.Count == 3, "same song restart was incorrectly deduplicated");
    Check(sink.Requested.Last().Id != second.Id, "generation did not affect announcement identity");
    daemon.Stall = true;
    await Until(() => daemon.Active > 0, "read did not stall");
    loop.SetEnabled(false);
    await Until(() => loop.GetSnapshot().State == "disabled", "disable waited for socket deadline", 500);
    Check(loop.GetSnapshot().Line == "", "disable kept lyric text");
    await loop.StopAsync();
    await Until(() => daemon.Active == 0, "stop left open sockets");
    Check(sink.PeakConcurrent == 1 && daemon.PeakActive == 1, "runtime overlapped requests or shell operations");
}

static async Task Recovery()
{
    await using var daemon = new FakeDaemon { Reply = Reply("恢复测试") };
    using var directory = new TemporaryDirectory();
    var endpoint = directory.Write("control.endpoint", $"{daemon.Port}\n{daemon.Secret}\n");
    var closedListener = new TcpListener(IPAddress.Loopback, 0);
    closedListener.Start();
    var closedPort = ((IPEndPoint)closedListener.LocalEndpoint).Port;
    closedListener.Stop();
    directory.Write("control.endpoint", $"{closedPort}\n{daemon.Secret}\n");
    var config = directory.Write("widgets.json", JsonSerializer.Serialize(new { schema = 1, mode = "system", enabled = true, endpoint_path = endpoint }));
    var sink = new RecordingSink();
    await using var loop = new RuntimeLoop(sink, config, directory.File("status.json"), new RuntimeOptions
    {
        PollInterval = TimeSpan.FromMilliseconds(15), RequestInterval = TimeSpan.FromMilliseconds(50), IoTimeout = TimeSpan.FromMilliseconds(150)
    });
    loop.SetEnabled(true);
    loop.Start();
    await Until(() => loop.GetSnapshot().State == "offline", "refused TCP connection terminated or bypassed the runtime");
    directory.Write("control.endpoint", $"{daemon.Port}\n{daemon.Secret}\n");
    await Until(() => sink.Requested.Count == 1, "initial request missing");
    sink.FailRemove = true;
    daemon.Reply = Reply(state: "stopped");
    await Until(() => loop.GetSnapshot().State == "stopped", "stop state missing");
    Check(loop.GetSnapshot().Line == "", "failed removal retained private text");
    sink.FailRemove = false;
    await Until(() => sink.Removed.Count > 0, "failed removal was never retried");
    daemon.Reply = "[]";
    await Until(() => loop.GetSnapshot().State == "offline", "malformed reply not treated as unavailable");
    daemon.Reply = Reply("新的歌词", track: "song:3");
    await Until(() => sink.Requested.Count == 2, "worker stopped permanently after protocol failure");
    directory.Write("widgets.json", "null");
    await Until(() => loop.GetSnapshot().State == "offline", "non-object config did not hide the announcement");
    directory.Write("widgets.json", JsonSerializer.Serialize(new { schema = 1, mode = "system", enabled = true, endpoint_path = endpoint }));
    await Until(() => sink.Requested.Count == 3, "worker stopped permanently after malformed config");
    directory.Write("widgets.json", JsonSerializer.Serialize(new { schema = 1, mode = "system", enabled = false, endpoint_path = endpoint }));
    await Until(() => loop.GetSnapshot().State == "disabled", "config disable ignored");
    await loop.StopAsync();
    Check(sink.PeakConcurrent == 1, "removal retry raced another shell operation");
}

static async Task Switching()
{
    await using var firstDaemon = new FakeDaemon { Reply = Reply("") };
    await using var secondDaemon = new FakeDaemon { Reply = Reply("") };
    using var directory = new TemporaryDirectory();
    var firstEndpoint = directory.Write("first.endpoint", $"{firstDaemon.Port}\n{firstDaemon.Secret}\n");
    var secondEndpoint = directory.Write("second.endpoint", $"{secondDaemon.Port}\n{secondDaemon.Secret}\n");
    var config = directory.Write("widgets.json", JsonSerializer.Serialize(new { schema = 1, mode = "system", enabled = true, endpoint_path = firstEndpoint }));
    var sink = new RecordingSink();
    await using var loop = new RuntimeLoop(sink, config, directory.File("status.json"), new RuntimeOptions
    {
        PollInterval = TimeSpan.FromMilliseconds(15), RequestInterval = TimeSpan.FromMilliseconds(100), IoTimeout = TimeSpan.FromMilliseconds(300)
    });
    loop.SetEnabled(true);
    loop.Start();
    await Until(() => sink.Requested.Count == 1, "title fallback was not requested");
    var first = sink.Requested.Single();
    Check(first.PrimaryText == "私密歌名" && loop.GetSnapshot().Title == "私密歌名", "no-lyric playback lost its title");
    directory.Write("widgets.json", JsonSerializer.Serialize(new { schema = 1, mode = "system", enabled = true, endpoint_path = secondEndpoint }));
    await Until(() => sink.Requested.Count == 2, "identical track from a different daemon was wrongly reused");
    Check(sink.Removed.Contains(first.Id), "endpoint switch did not remove old announcement");
    var oldVerse = sink.Requested.Last();
    secondDaemon.Reply = Reply("下一句");
    await Until(() => sink.Requested.Count == 3, "next verse missing");
    secondDaemon.Reply = Reply("");
    await Until(() => sink.Requested.Count == 4, "repeated verse missing");
    var repeatedVerse = sink.Requested.Last();
    Check(repeatedVerse.Id != oldVerse.Id, "repeated verse reused announcement id");
    var removals = sink.Removed.Count;
    loop.MarkShown(oldVerse.Id);
    await Until(() => sink.Removed.Count > removals, "old verse Shown callback not retracted");
    Check(loop.GetSnapshot().AnnouncementId == repeatedVerse.Id, "old verse callback replaced or removed current verse");
    sink.OnRequest = _ => loop.SetEnabled(false);
    secondDaemon.Reply = Reply("禁用竞态");
    await Until(() => sink.Requested.Count == 5 && loop.GetSnapshot().State == "disabled", "disable inside request did not win");
    Check(sink.Removed.Contains(sink.Requested.Last().Id), "request completed after disable without immediate removal");
    Check(loop.GetSnapshot().Line == "", "disable race retained text");
    await loop.StopAsync();
    Check(sink.PeakConcurrent == 1, "shell callbacks reentered sink operations");
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

sealed class RecordingSink : IAnnouncementSink
{
    public ConcurrentQueue<Announcement> Requested { get; } = new();
    public ConcurrentQueue<string> Removed { get; } = new();
    public volatile bool FailRemove;
    public Action<Announcement>? OnRequest;
    private int _active;
    public int PeakConcurrent;
    public void Request(Announcement announcement)
    {
        var active = Interlocked.Increment(ref _active);
        PeakConcurrent = Math.Max(PeakConcurrent, active);
        try { Requested.Enqueue(announcement); OnRequest?.Invoke(announcement); }
        finally { Interlocked.Decrement(ref _active); }
    }
    public void Remove(string id)
    {
        var active = Interlocked.Increment(ref _active);
        PeakConcurrent = Math.Max(PeakConcurrent, active);
        try
        {
            if (FailRemove) throw new IOException("test shell failure");
            Removed.Enqueue(id);
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
