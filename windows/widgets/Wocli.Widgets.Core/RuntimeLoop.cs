using System.Diagnostics;
using System.Net.Sockets;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;

namespace Wocli.Widgets.Core;

/// <summary>
/// One polling task owns all announcement operations. Changes replace the latest
/// sample; there is no lyric queue and a successful request does not mean shown.
/// </summary>
public sealed class RuntimeLoop : IAsyncDisposable
{
    private readonly object _gate = new();
    private readonly IAnnouncementSink _sink;
    private readonly string _configPath;
    private readonly string _statusPath;
    private readonly RuntimeOptions _options;
    private readonly DaemonClient _client;
    private readonly SemaphoreSlim _wake = new(0, 1);
    private CancellationTokenSource? _lifetime;
    private CancellationTokenSource? _request;
    private Task? _worker;
    private bool _feedEnabled;
    private bool _disposed;
    private bool _wakeCancelled;
    private string _endpointPath = "";
    private string? _activeId;
    private string? _lastFingerprint;
    private string? _requestedId;
    private string? _shownId;
    private string? _staleShownId;
    private string? _lastErrorCode;
    private long _requestedCount;
    private long _shownCount;
    private long? _lastRequestTick;
    private long? _lastStatusTick;
    private string? _lastStatusContent;
    private DateTimeOffset _expiresAt;
    private RuntimeSnapshot _snapshot = new(false, "starting", "", "", "", "", null, null, null, null, null, DateTimeOffset.UtcNow);

    public RuntimeLoop(IAnnouncementSink sink, string? configPath = null, string? statusPath = null, RuntimeOptions? options = null)
    {
        _sink = sink ?? throw new ArgumentNullException(nameof(sink));
        var directory = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "wocli-music");
        _configPath = configPath ?? Path.Combine(directory, "widgets.json");
        _statusPath = statusPath ?? Path.Combine(directory, "widgets-status.json");
        _options = options ?? new RuntimeOptions();
        if (_options.PollInterval <= TimeSpan.Zero || _options.RequestInterval <= TimeSpan.Zero ||
            _options.AnnouncementLifetime <= TimeSpan.Zero || _options.AnnouncementDuration <= TimeSpan.Zero)
            throw new ArgumentOutOfRangeException(nameof(options));
        _client = new DaemonClient(_options.IoTimeout);
    }

    public void Start()
    {
        lock (_gate)
        {
            ObjectDisposedException.ThrowIf(_disposed, this);
            if (_worker is { IsCompleted: false }) return;
            _lifetime?.Dispose();
            _lifetime = new CancellationTokenSource();
            _worker = Task.Run(() => RunAsync(_lifetime.Token));
        }
    }

    public void SetEnabled(bool enabled)
    {
        lock (_gate)
        {
            if (_disposed) return;
            if (_feedEnabled == enabled) return;
            _feedEnabled = enabled;
            _wakeCancelled = true;
            _request?.Cancel();
        }
        Wake();
    }

    public async Task StopAsync()
    {
        Task? worker;
        lock (_gate)
        {
            _lifetime?.Cancel();
            _request?.Cancel();
            worker = _worker;
        }
        Wake();
        if (worker is not null) await worker.ConfigureAwait(false);
    }

    public RuntimeSnapshot GetSnapshot()
    {
        lock (_gate) return _snapshot;
    }

    public void MarkShown(string id)
    {
        lock (_gate)
        {
            if (!_feedEnabled || id != _snapshot.AnnouncementId)
            {
                // The shell can report an old request after boss/offline/disable.
                // Keep only its latest visible id and retract on the worker;
                // calling COM synchronously from its callback can deadlock.
                _staleShownId = id;
                _wakeCancelled = true;
                _request?.Cancel();
            }
            else
            {
                if (id == _shownId) return;
                _shownId = id;
                _shownCount++;
                _snapshot = _snapshot with { State = "shown", LastShownAt = DateTimeOffset.UtcNow };
            }
        }
        Wake();
    }

    public void MarkEngaged(string id)
    {
        lock (_gate)
        {
            if (id != _snapshot.AnnouncementId) return;
            _snapshot = _snapshot with { LastEngagedAt = DateTimeOffset.UtcNow };
        }
        Wake();
    }

    private void Wake()
    {
        try { _wake.Release(); }
        catch (SemaphoreFullException) { }
        catch (ObjectDisposedException) { }
    }

    private async Task RunAsync(CancellationToken cancellationToken)
    {
        try
        {
            while (!cancellationToken.IsCancellationRequested)
            {
                RemoveStaleShown();
                using var request = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
                request.CancelAfter(_options.IoTimeout);
                lock (_gate)
                {
                    _wakeCancelled = false;
                    _request = request;
                }
                try
                {
                    bool enabled;
                    lock (_gate) enabled = _feedEnabled;
                    var config = await DaemonClient.ReadConfigAsync(_configPath, request.Token).ConfigureAwait(false);
                    if (!string.Equals(_endpointPath, config.EndpointPath, StringComparison.Ordinal)) Clear("switching_source", false);
                    lock (_gate) _endpointPath = config.EndpointPath;
                    if (!enabled || !config.Enabled)
                    {
                        Clear("disabled", false);
                    }
                    else
                    {
                        var lyrics = await WithHeartbeatAsync(_client.ReadAsync(config.EndpointPath, request.Token), request.Token).ConfigureAwait(false);
                        request.Token.ThrowIfCancellationRequested();
                        Apply(lyrics);
                    }
                }
                catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested) { break; }
                catch (OperationCanceledException)
                {
                    bool enabled;
                    bool wakeCancelled;
                    lock (_gate) { enabled = _feedEnabled; wakeCancelled = _wakeCancelled; }
                    if (!enabled || !wakeCancelled)
                        Clear(enabled ? "offline" : "disabled", enabled, enabled ? "request_timeout" : null);
                }
                catch (Exception error) when (error is IOException or SocketException or UnauthorizedAccessException or JsonException or ArgumentException)
                {
                    Clear("offline", false, error is ProtocolException protocol ? protocol.Code : "connection_unavailable");
                }
                finally
                {
                    lock (_gate) _request = null;
                }
                await WriteStatusAsync(cancellationToken).ConfigureAwait(false);
                await _wake.WaitAsync(_options.PollInterval, cancellationToken).ConfigureAwait(false);
            }
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested) { }
        finally
        {
            Clear("stopped", false);
            RemoveStaleShown();
            await WriteStatusAsync(CancellationToken.None, force: true).ConfigureAwait(false);
        }
    }

    private async Task<T> WithHeartbeatAsync<T>(Task<T> operation, CancellationToken cancellationToken)
    {
        using var heartbeat = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        try
        {
            while (!operation.IsCompleted)
            {
                var pulse = Task.Delay(TimeSpan.FromSeconds(1), heartbeat.Token);
                if (await Task.WhenAny(operation, pulse).ConfigureAwait(false) == operation || cancellationToken.IsCancellationRequested) break;
                await WriteStatusAsync(cancellationToken).ConfigureAwait(false);
            }
            // Always observe completion, including cancellation, before starting
            // another connection or disposing the request cancellation source.
            return await operation.ConfigureAwait(false);
        }
        finally { heartbeat.Cancel(); }
    }

    private void Apply(LyricsSnapshot lyrics)
    {
        if (!lyrics.Enabled) { Clear("disabled", false); return; }
        if (lyrics.Boss) { Clear("boss", true); return; }
        if (lyrics.State != "playing") { Clear(lyrics.State == "paused" ? "paused" : "stopped", true); return; }
        if (lyrics.TrackId.Length == 0 || (lyrics.Line.Length == 0 && lyrics.Title.Length == 0)) { Clear("waiting_for_lyrics", true); return; }

        var now = DateTimeOffset.UtcNow;
        var primary = lyrics.Line.Length > 0 ? lyrics.Line : lyrics.Title;
        var fingerprint = Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes(
            JsonSerializer.Serialize(new[] { lyrics.TrackId, lyrics.Title, lyrics.Artist, primary }))));
        // Never leave a previous track/line on screen while waiting for our rate
        // limit. Only the most recently polled line is considered next time.
        if (_activeId is not null && (fingerprint != _lastFingerprint || now >= _expiresAt) && !RemoveActive())
        {
            lock (_gate) _snapshot = _snapshot with { State = "unavailable", TrackId = "", Title = "", Artist = "", Line = "" };
            return;
        }
        lock (_gate)
        {
            if (_staleShownId is null) _lastErrorCode = null;
            _snapshot = _snapshot with
            {
                Enabled = true, TrackId = lyrics.TrackId, Title = lyrics.Title,
                Artist = lyrics.Artist, Line = lyrics.Line, LastError = _staleShownId is null ? null : _snapshot.LastError,
                State = _activeId is null ? "waiting" : _snapshot.State
            };
        }
        if (fingerprint == _lastFingerprint) return;
        if (_lastRequestTick is { } tick && Stopwatch.GetElapsedTime(tick) < _options.RequestInterval) return;
        bool feedEnabled;
        lock (_gate) feedEnabled = _feedEnabled;
        if (!feedEnabled) { Clear("disabled", false); return; }
        _lastRequestTick = Stopwatch.GetTimestamp();
        _lastFingerprint = fingerprint;
        // A chorus/restart may repeat identical text; an older shell callback
        // must never be mistaken for this new request.
        _activeId = "wocli-" + Guid.NewGuid().ToString("N");
        _expiresAt = now + _options.AnnouncementLifetime;
        var announcement = new Announcement(_activeId, primary,
            lyrics.Line.Length == 0 ? lyrics.Artist : string.Join(" · ", new[] { lyrics.Title, lyrics.Artist }.Where(text => text.Length > 0)),
            _expiresAt, _options.AnnouncementDuration);
        lock (_gate)
        {
            _requestedId = _activeId;
            _requestedCount++;
            _snapshot = _snapshot with { State = "requested", AnnouncementId = _activeId, LastRequestedAt = now };
        }
        try
        {
            _sink.Request(announcement);
            // SetEnabled can race the external COM call. Retract immediately
            // after it returns, even if the next polling iteration is delayed.
            lock (_gate) feedEnabled = _feedEnabled;
            if (!feedEnabled) Clear("disabled", false);
        }
        catch (Exception)
        {
            // A COM request may partly succeed before throwing. Retract its id.
            RemoveActive();
            lock (_gate)
            {
                _lastErrorCode = "announcement_request_failed";
                _snapshot = _snapshot with { State = "unavailable", LastError = "System announcement request failed." };
            }
        }
    }

    private void RemoveStaleShown()
    {
        string? id;
        lock (_gate)
        {
            id = _staleShownId;
            _staleShownId = null;
        }
        if (id is null) return;
        try { _sink.Remove(id); }
        catch (Exception)
        {
            lock (_gate)
            {
                _staleShownId ??= id;
                _lastErrorCode = "announcement_remove_failed";
                _snapshot = _snapshot with { LastError = "System announcement removal failed." };
            }
        }
    }

    private void Clear(string state, bool enabled, string? error = null)
    {
        var removed = RemoveActive();
        _lastFingerprint = null;
        lock (_gate)
        {
            var cleared = removed && _staleShownId is null;
            if (cleared) _lastErrorCode = error;
            _snapshot = _snapshot with
            {
                Enabled = enabled, State = state, TrackId = "", Title = "", Artist = "", Line = "",
                AnnouncementId = null,
                LastError = cleared ? error is null ? null : "Music service unavailable or response invalid." : _snapshot.LastError
            };
        }
    }

    private bool RemoveActive()
    {
        var id = _activeId;
        lock (_gate) _snapshot = _snapshot with { AnnouncementId = null };
        if (id is null) return true;
        try
        {
            _sink.Remove(id);
            _activeId = null;
            return true;
        }
        catch (Exception)
        {
            lock (_gate)
            {
                _lastErrorCode = "announcement_remove_failed";
                _snapshot = _snapshot with { LastError = "System announcement removal failed." };
            }
            return false;
        }
    }

    private async Task WriteStatusAsync(CancellationToken cancellationToken, bool force = false)
    {
        Dictionary<string, object?> status;
        lock (_gate)
        {
            _snapshot = _snapshot with { UpdatedAt = DateTimeOffset.UtcNow };
            status = new Dictionary<string, object?>
            {
                ["endpoint_path"] = _endpointPath, ["state"] = _snapshot.State, ["enabled"] = _snapshot.Enabled,
                ["announcement_id"] = _snapshot.AnnouncementId, ["requested_id"] = _requestedId, ["shown_id"] = _shownId,
                ["requested_count"] = _requestedCount, ["shown_count"] = _shownCount,
                ["last_requested_at"] = _snapshot.LastRequestedAt, ["last_shown_at"] = _snapshot.LastShownAt,
                ["last_engaged_at"] = _snapshot.LastEngagedAt, ["last_error"] = _snapshot.LastError, ["last_error_code"] = _lastErrorCode
            };
        }
        var content = JsonSerializer.Serialize(status);
        if (!force && content == _lastStatusContent && _lastStatusTick is { } tick && Stopwatch.GetElapsedTime(tick) < TimeSpan.FromSeconds(1)) return;
        status["updated_at_unix_ms"] = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();
        var temporary = _statusPath + "." + Environment.ProcessId + ".tmp";
        try
        {
            Directory.CreateDirectory(Path.GetDirectoryName(Path.GetFullPath(_statusPath))!);
            await File.WriteAllTextAsync(temporary, JsonSerializer.Serialize(status), cancellationToken).ConfigureAwait(false);
            File.Move(temporary, _statusPath, true);
            _lastStatusContent = content;
            _lastStatusTick = Stopwatch.GetTimestamp();
        }
        catch (Exception error) when (error is IOException or UnauthorizedAccessException or OperationCanceledException) { }
        finally
        {
            try { File.Delete(temporary); } catch (IOException) { } catch (UnauthorizedAccessException) { }
        }
    }

    public async ValueTask DisposeAsync()
    {
        lock (_gate)
        {
            if (_disposed) return;
            _disposed = true;
        }
        await StopAsync().ConfigureAwait(false);
        lock (_gate)
        {
            _lifetime?.Dispose();
            _lifetime = null;
        }
        _wake.Dispose();
    }
}
