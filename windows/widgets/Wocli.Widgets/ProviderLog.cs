using System.Diagnostics;

namespace Wocli.Widgets;

internal static class ProviderLog
{
    // Exception messages can contain URLs or media data. Only emit a fixed operation
    // name and numeric HRESULT; playback details and IPC tokens never enter logs.
    public static void Error(string operation, Exception error) =>
        Trace.TraceError("wocli widgets {0}: HRESULT 0x{1:X8}", operation, error.HResult);
}
