using System.Globalization;
using System.Net;
using System.Net.Sockets;
using System.Text;
using System.Text.Json;

namespace Wocli.Widgets.Core;

public sealed class DaemonClient
{
    public const int MaxConfigBytes = 16 * 1024;
    public const int MaxReplyBytes = 64 * 1024;
    private static readonly UTF8Encoding Utf8 = new(false, true);
    private readonly TimeSpan _timeout;

    public DaemonClient(TimeSpan? timeout = null)
    {
        _timeout = timeout ?? TimeSpan.FromSeconds(2);
        if (_timeout <= TimeSpan.Zero) throw new ArgumentOutOfRangeException(nameof(timeout));
    }

    public static async Task<WidgetConfig> ReadConfigAsync(string path, CancellationToken cancellationToken)
    {
        using var document = JsonDocument.Parse(await ReadFileAsync(path, MaxConfigBytes, cancellationToken));
        var root = document.RootElement;
        var enabled = Boolean(root, "enabled");
        if (!root.TryGetProperty("schema", out var schema) || schema.ValueKind != JsonValueKind.Number || !schema.TryGetInt32(out var version) || version != 1)
            throw new ProtocolException("config_schema");
        if (!root.TryGetProperty("mode", out var mode) || mode.ValueKind != JsonValueKind.String || mode.GetString() != "system")
            throw new ProtocolException("config_mode");
        if (!root.TryGetProperty("endpoint_path", out var endpointValue) || endpointValue.ValueKind != JsonValueKind.String)
            throw new ProtocolException("endpoint_path");
        // Paths are identifiers: never trim or normalize characters into a
        // different filename. Rust canonicalize may emit a local \\?\C:\ path.
        var endpoint = endpointValue.GetString()!;
        if (endpoint.Length > 4096 || endpoint.Any(char.IsControl) || !IsLocalPath(endpoint))
            throw new ProtocolException("endpoint_path");
        return new WidgetConfig(enabled, endpoint);
    }

    public async Task<LyricsSnapshot> ReadAsync(string endpointPath, CancellationToken cancellationToken)
    {
        using var deadline = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        deadline.CancelAfter(_timeout);
        var token = deadline.Token;
        var content = Utf8.GetString(await ReadFileAsync(endpointPath, 4096, token));
        var lines = content.Split('\n');
        if (lines.Length < 2 || !ushort.TryParse(lines[0].Trim(), NumberStyles.None, CultureInfo.InvariantCulture, out var port) || port == 0)
            throw new ProtocolException("endpoint_format");
        var secret = lines[1].TrimEnd('\r');
        if (secret.Length is 0 or > 256 || secret.Any(character => character < '!' || character > '~'))
            throw new ProtocolException("endpoint_format");
        // Never resolve a hostname or accept an address from the endpoint file.
        using var client = new TcpClient(AddressFamily.InterNetwork) { NoDelay = true };
        await client.ConnectAsync(IPAddress.Loopback, port, token);
        await using var stream = client.GetStream();
        await stream.WriteAsync(Utf8.GetBytes(secret + "\n{\"op\":\"lyrics_snapshot\"}\n"), token);
        var reply = await ReadLineAsync(stream, MaxReplyBytes, token);
        using var document = JsonDocument.Parse(reply);
        var root = document.RootElement;
        if (!Boolean(root, "ok")) throw new ProtocolException("daemon_rejected");
        if (!root.TryGetProperty("data", out var data) || data.ValueKind != JsonValueKind.Object)
            throw new ProtocolException("reply_format");
        return new LyricsSnapshot(
            Boolean(data, "enabled"), Boolean(data, "boss"), Text(data, "state", 32),
            Text(data, "track_id", 320), Text(data, "title", 240), Text(data, "artist", 240), Text(data, "line", 240));
    }

    private static bool Boolean(JsonElement element, string name)
    {
        if (element.ValueKind != JsonValueKind.Object || !element.TryGetProperty(name, out var value) || value.ValueKind is not (JsonValueKind.True or JsonValueKind.False))
            throw new ProtocolException("reply_format");
        return value.GetBoolean();
    }

    private static string Text(JsonElement element, string name, int maxRunes)
    {
        if (element.ValueKind != JsonValueKind.Object || !element.TryGetProperty(name, out var value) || value.ValueKind != JsonValueKind.String)
            throw new ProtocolException("reply_format");
        var text = value.GetString()!;
        if (text.EnumerateRunes().Count() > maxRunes) throw new ProtocolException("text_limit");
        return string.Concat(text.EnumerateRunes().Select(rune => Rune.IsControl(rune) ? " " : rune.ToString())).Trim();
    }

    private static bool IsLocalPath(string path)
    {
        static bool DrivePath(ReadOnlySpan<char> value) => value.Length >= 3 && char.IsAsciiLetter(value[0]) && value[1] == ':' && value[2] is '\\' or '/';
        if (path.StartsWith("\\\\?\\", StringComparison.Ordinal)) return DrivePath(path.AsSpan(4));
        if (path.StartsWith("\\\\", StringComparison.Ordinal) || path.StartsWith("//", StringComparison.Ordinal)) return false;
        return DrivePath(path) || Path.IsPathFullyQualified(path);
    }

    private static async Task<byte[]> ReadFileAsync(string path, int limit, CancellationToken cancellationToken)
    {
        await using var stream = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete, 4096, FileOptions.Asynchronous);
        using var output = new MemoryStream();
        var buffer = new byte[4096];
        while (true)
        {
            var count = await stream.ReadAsync(buffer, cancellationToken);
            if (count == 0) return output.ToArray();
            if (output.Length + count > limit) throw new ProtocolException("file_limit");
            output.Write(buffer, 0, count);
        }
    }

    private static async Task<byte[]> ReadLineAsync(Stream stream, int limit, CancellationToken cancellationToken)
    {
        using var output = new MemoryStream();
        var buffer = new byte[4096];
        while (true)
        {
            var count = await stream.ReadAsync(buffer, cancellationToken);
            if (count == 0) throw new ProtocolException("reply_incomplete");
            var newline = Array.IndexOf(buffer, (byte)'\n', 0, count);
            var used = newline < 0 ? count : newline;
            if (output.Length + used > limit) throw new ProtocolException("reply_limit");
            output.Write(buffer, 0, used);
            if (newline >= 0) return output.ToArray();
        }
    }
}
