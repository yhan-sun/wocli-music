using System.Diagnostics;
using System.Net.Sockets;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;

namespace Wocli.Widgets.Core;

/// <summary>One worker owns TCP requests and shell updates; callbacks only change bounded state.</summary>
public sealed class BoardRuntime : IAsyncDisposable
{
    private sealed class Entry(CardSize size, bool active)
    {
        public CardSize Size = size;
        public bool Active = active;
        public long Revision;
        public string? Fingerprint;
        public string Nonce = "";
        public Binding? Binding;
    }
    private sealed record Binding(string Endpoint, string TrackId, string State);
    private sealed record PendingAction(string WidgetId, string Verb, Binding Binding, long Generation);
    private readonly object _gate = new();
    private readonly Dictionary<string, Entry> _entries = new(StringComparer.Ordinal);
    private readonly IWidgetSink _sink;
    private readonly DaemonClient _client;
    private readonly BoardOptions _options;
    private readonly string _configPath;
    private readonly string _statusPath;
    private readonly SemaphoreSlim _wake = new(0, 1);
    private CancellationTokenSource? _lifetime;
    private CancellationTokenSource? _request;
    private Task? _worker;
    private PendingAction? _action;
    private bool _actionInFlight;
    private long _generation;
    private long _entryRevision;
    private string _endpoint = "";
    private string _state = "starting";
    private string? _error;
    private bool _enabled;
    private bool _disposed;
    private LyricsSnapshot? _lyrics;
    private long _updatedCount;
    private long _actionCount;
    private long? _statusTick;

    public BoardRuntime(IWidgetSink sink, string? configPath = null, string? statusPath = null, BoardOptions? options = null)
    {
        _sink = sink ?? throw new ArgumentNullException(nameof(sink));
        _options = options ?? new BoardOptions();
        if (_options.PollInterval <= TimeSpan.Zero || _options.IoTimeout <= TimeSpan.Zero) throw new ArgumentOutOfRangeException(nameof(options));
        _client = new DaemonClient(_options.IoTimeout);
        var directory = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "wocli-music");
        _configPath = configPath ?? Path.Combine(directory, "widgets.json");
        _statusPath = statusPath ?? Path.Combine(directory, "widgets-status.json");
    }

    public void Start()
    {
        lock (_gate)
        {
            ObjectDisposedException.ThrowIf(_disposed, this);
            if (_worker is { IsCompleted: false }) return;
            _lifetime?.Dispose();
            _lifetime = new CancellationTokenSource();
            _worker = Task.Run(() => RunWorkerAsync(_lifetime.Token));
        }
    }

    // Kept for hosts with their own lifetime token; repeated starts never add a worker.
    public async Task RunAsync(CancellationToken cancellationToken)
    {
        Start();
        using var registration = cancellationToken.Register(() => { lock (_gate) _lifetime?.Cancel(); Wake(); });
        Task worker;
        lock (_gate) worker = _worker!;
        await worker.ConfigureAwait(false);
    }

    public void Upsert(string id, CardSize size, bool active = false)
    {
        if (string.IsNullOrEmpty(id) || id.Length > 512 || !Enum.IsDefined(size)) return;
        lock (_gate)
        {
            if (_disposed) return;
            if (_entries.TryGetValue(id, out var entry))
            {
                if (entry.Size == size && entry.Active == active) return;
                entry.Size = size;
                entry.Active = active;
                entry.Revision = ++_entryRevision;
                entry.Binding = null;
                entry.Nonce = "";
            }
            else
            {
                // The host normally pins only a handful; also bound a malformed callback flood.
                if (_entries.Count >= 64) return;
                _entries.Add(id, new Entry(size, active) { Revision = ++_entryRevision });
            }
            InvalidateLocked();
        }
        Wake();
    }

    public void Restore(string id, CardSize size, bool active = false) => Upsert(id, size, active);
    public void Activate(string id) => SetActive(id, true);
    public void Deactivate(string id) => SetActive(id, false);
    public void SetActive(string id, bool active)
    {
        lock (_gate)
        {
            if (_disposed || !_entries.TryGetValue(id, out var entry) || entry.Active == active) return;
            entry.Active = active;
            entry.Revision = ++_entryRevision;
            entry.Binding = null;
            entry.Nonce = "";
            InvalidateLocked();
            if (!_entries.Values.Any(value => value.Active)) _lyrics = null;
        }
        Wake();
    }

    public void Delete(string id)
    {
        lock (_gate)
        {
            if (!_entries.Remove(id)) return;
            InvalidateLocked();
            if (!_entries.Values.Any(value => value.Active)) _lyrics = null;
        }
        Wake();
    }

    public Task HandleActionAsync(string id, string verb, JsonElement data)
    {
        if (verb is not ("prev" or "next" or "pause" or "resume") || data.ValueKind != JsonValueKind.Object ||
            !data.TryGetProperty("nonce", out var nonce) || nonce.ValueKind != JsonValueKind.String) return Task.CompletedTask;
        var value = nonce.GetString();
        if (value?.Length != 32) return Task.CompletedTask;
        lock (_gate)
        {
            if (_disposed || _action is not null || _actionInFlight || !_enabled || !_entries.TryGetValue(id, out var entry) || !entry.Active ||
                entry.Binding is not { } binding || entry.Nonce != value ||
                (verb == "pause" && binding.State != "playing") || (verb == "resume" && binding.State != "paused")) return Task.CompletedTask;
            _action = new PendingAction(id, verb, binding, _generation);
            // A double click or replay cannot execute the same rendered command twice.
            entry.Binding = null;
            entry.Nonce = "";
            _request?.Cancel();
        }
        Wake();
        return Task.CompletedTask;
    }

    public BoardRuntimeSnapshot GetSnapshot()
    {
        lock (_gate) return new(_enabled, _state, _entries.Count, _entries.Values.Count(entry => entry.Active), _error);
    }

    private void InvalidateLocked()
    {
        _generation++;
        _action = null;
        _request?.Cancel();
    }

    private void Wake()
    {
        try { _wake.Release(); } catch (SemaphoreFullException) { } catch (ObjectDisposedException) { }
    }

    private async Task RunWorkerAsync(CancellationToken cancellationToken)
    {
        try
        {
            while (!cancellationToken.IsCancellationRequested)
            {
                // Reconcile inactive cards before any potentially slow I/O.
                Publish();
                using var request = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
                request.CancelAfter(_options.IoTimeout);
                long generation;
                lock (_gate) { _request = request; generation = _generation; }
                try
                {
                    var config = await DaemonClient.ReadConfigAsync(_configPath, request.Token).ConfigureAwait(false);
                    bool active;
                    PendingAction? action;
                    lock (_gate)
                    {
                        if (_endpoint != config.EndpointPath)
                        {
                            _endpoint = config.EndpointPath;
                            ClearLocked("switching_source", config.Enabled);
                            _action = null;
                        }
                        _enabled = config.Enabled;
                        active = _entries.Values.Any(entry => entry.Active);
                        action = _action;
                        _action = null;
                        _actionInFlight = action is not null;
                    }
                    if (!config.Enabled || !active)
                    {
                        lock (_gate) ClearLocked(config.Enabled ? "inactive" : "disabled", config.Enabled);
                    }
                    else
                    {
                        Publish();
                        LyricsSnapshot lyrics;
                        if (action is not null && IsCurrent(action))
                        {
                            lyrics = await _client.ActionAsync(config.EndpointPath, action.Verb, action.Binding.TrackId, action.Binding.State, request.Token).ConfigureAwait(false);
                            lock (_gate) _actionCount++;
                        }
                        else lyrics = await _client.ReadAsync(config.EndpointPath, request.Token).ConfigureAwait(false);
                        // A source can be changed while a server is responding. Do not
                        // publish that response under the newly-selected library.
                        var current = await DaemonClient.ReadConfigAsync(_configPath, request.Token).ConfigureAwait(false);
                        request.Token.ThrowIfCancellationRequested();
                        lock (_gate)
                        {
                            if (_generation != generation) continue;
                            if (current != config)
                            {
                                _endpoint = current.EndpointPath;
                                ClearLocked(current.Enabled ? "switching_source" : "disabled", current.Enabled);
                                _action = null;
                            }
                            else ApplyLocked(lyrics);
                        }
                    }
                }
                catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested) { break; }
                catch (OperationCanceledException)
                {
                    lock (_gate)
                    {
                        // A lifecycle/action wake will be reconciled immediately. A real
                        // network timeout removes all private text and action bindings.
                        if (_generation == generation && _action is null) ClearLocked("offline", _enabled, "request_timeout");
                    }
                }
                catch (ProtocolException error) when (error.Code == "stale_widget_action")
                {
                    // Track/state may advance between rendering and clicking.
                    // The daemon rejected the action without executing it; this
                    // is a refresh, not a connection failure. Immediately poll
                    // for a new snapshot and issue a fresh action nonce.
                    lock (_gate) ClearLocked("refreshing", _enabled, error.Code);
                    Wake();
                }
                catch (Exception error) when (error is IOException or SocketException or UnauthorizedAccessException or JsonException or ArgumentException)
                {
                    lock (_gate) ClearLocked("offline", _enabled, error is ProtocolException protocol ? protocol.Code : "connection_unavailable");
                }
                finally { lock (_gate) { _request = null; _actionInFlight = false; } }
                Publish();
                await WriteStatusAsync(cancellationToken).ConfigureAwait(false);
                await _wake.WaitAsync(_options.PollInterval, cancellationToken).ConfigureAwait(false);
            }
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested) { }
        finally
        {
            lock (_gate) ClearLocked("stopped", false);
            Publish();
            await WriteStatusAsync(CancellationToken.None, true).ConfigureAwait(false);
        }
    }

    private bool IsCurrent(PendingAction action)
    {
        lock (_gate) return action.Generation == _generation && action.Binding.Endpoint == _endpoint &&
            _entries.TryGetValue(action.WidgetId, out var entry) && entry.Active &&
            _lyrics is { Boss: false, Enabled: true } lyrics && lyrics.TrackId == action.Binding.TrackId && lyrics.State == action.Binding.State;
    }

    private void ApplyLocked(LyricsSnapshot lyrics)
    {
        if (!lyrics.Enabled) { ClearLocked("disabled", false); return; }
        if (lyrics.Boss) { ClearLocked("boss", true); return; }
        if (lyrics.State is not ("playing" or "paused") || lyrics.TrackId.Length == 0) { ClearLocked("stopped", true); return; }
        _lyrics = lyrics;
        _enabled = true;
        _state = lyrics.State;
        _error = null;
    }

    private void ClearLocked(string state, bool enabled, string? error = null)
    {
        _lyrics = null;
        _action = null;
        _state = state;
        _enabled = enabled;
        _error = error;
        foreach (var entry in _entries.Values) { entry.Binding = null; entry.Nonce = ""; }
    }

    private void Publish()
    {
        string[] ids;
        lock (_gate) ids = _entries.Keys.ToArray();
        foreach (var id in ids)
        {
            // Reconcile a lifecycle callback that arrived during a shell update
            // without reentering COM from inside that callback.
            for (var retry = 0; retry < 3; retry++)
            {
                WidgetCard card;
                string fingerprint;
                long revision;
                lock (_gate)
                {
                    if (!_entries.TryGetValue(id, out var entry)) break;
                    revision = entry.Revision;
                    var lyrics = entry.Active && _enabled ? _lyrics : null;
                    if (lyrics is not null && _action is null && !_actionInFlight)
                    {
                        var binding = new Binding(_endpoint, lyrics.TrackId, lyrics.State);
                        if (entry.Binding != binding)
                        {
                            entry.Binding = binding;
                            entry.Nonce = Guid.NewGuid().ToString("N");
                        }
                    }
                    else { entry.Binding = null; entry.Nonce = ""; }
                    card = WidgetCards.Create(entry.Size, entry.Active ? _state : "inactive", lyrics, entry.Nonce);
                    fingerprint = Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes(card.Template + card.Data)));
                    if (fingerprint == entry.Fingerprint) break;
                }
                try { _sink.Update(id, card); }
                catch (Exception)
                {
                    lock (_gate) _error = "widget_update_failed";
                    break; // Fingerprint remains dirty; the next iteration retries.
                }
                lock (_gate)
                {
                    _updatedCount++;
                    if (!_entries.TryGetValue(id, out var entry)) break;
                    if (entry.Revision != revision) continue;
                    entry.Fingerprint = fingerprint;
                    break;
                }
            }
        }
    }

    private async Task WriteStatusAsync(CancellationToken cancellationToken, bool force = false)
    {
        if (!force && _statusTick is { } tick && Stopwatch.GetElapsedTime(tick) < TimeSpan.FromSeconds(1)) return;
        string content;
        lock (_gate) content = JsonSerializer.Serialize(new
        {
            mode = "system-widget-card", endpoint_path = _endpoint, state = _state, enabled = _enabled,
            widget_count = _entries.Count, active_count = _entries.Values.Count(entry => entry.Active),
            updated_count = _updatedCount, action_count = _actionCount, last_error_code = _error,
            updated_at_unix_ms = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds()
        });
        var temporary = _statusPath + "." + Environment.ProcessId + ".tmp";
        try
        {
            Directory.CreateDirectory(Path.GetDirectoryName(Path.GetFullPath(_statusPath))!);
            await File.WriteAllTextAsync(temporary, content, cancellationToken).ConfigureAwait(false);
            File.Move(temporary, _statusPath, true);
            _statusTick = Stopwatch.GetTimestamp();
        }
        catch (Exception error) when (error is IOException or UnauthorizedAccessException or OperationCanceledException) { }
        finally { try { File.Delete(temporary); } catch (IOException) { } catch (UnauthorizedAccessException) { } }
    }

    public async Task StopAsync()
    {
        Task? worker;
        lock (_gate) { _lifetime?.Cancel(); _request?.Cancel(); worker = _worker; }
        Wake();
        if (worker is not null) await worker.ConfigureAwait(false);
    }

    public async ValueTask DisposeAsync()
    {
        lock (_gate) { if (_disposed) return; _disposed = true; }
        await StopAsync().ConfigureAwait(false);
        lock (_gate) { _lifetime?.Dispose(); _lifetime = null; _entries.Clear(); }
        _wake.Dispose();
    }
}
