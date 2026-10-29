using System.Runtime.InteropServices;
using Microsoft.Windows.Widgets.Feeds.Providers;
using Wocli.Widgets.Core;

namespace Wocli.Widgets;

internal static class ProviderSelfTest
{
    public static int Run()
    {
        RuntimeLoop? runtime = null;
        try
        {
            if (!Program.IsServerActivation([ProviderIdentity.ServerArgument]) ||
                !Program.IsServerActivation([ProviderIdentity.ServerArgument, "-Embedding"]) ||
                !Program.IsServerActivation(["-embedding", ProviderIdentity.ServerArgument]) ||
                Program.IsServerActivation(["-Embedding"]) ||
                Program.IsServerActivation([ProviderIdentity.ServerArgument, "--self-test"]))
                throw new InvalidOperationException("COM activation argument parsing failed.");
            using var apartment = new ComApartment();
            WinRT.ComWrappersSupport.InitializeComWrappers();
            // This runtime is never started; no config, daemon, FeedManager or
            // Widgets host is contacted by this test.
            runtime = new RuntimeLoop(new NoSystemSink());
            var provider = new FeedProvider(runtime, _ => { });
            var classId = Guid.NewGuid();
            using var server = new ComServer(provider, classId);
            var iid = typeof(IFeedProvider).GUID;
            Marshal.ThrowExceptionForHR(CoCreateInstance(ref classId, 0, 4, ref iid, out var instance));
            try
            {
                VerifyInterface(instance, typeof(IFeedProvider).GUID);
                VerifyInterface(instance, typeof(IFeedResourceProvider).GUID);
                VerifyInterface(instance, typeof(IFeedAnnouncementInvokedTarget).GUID);
                var unsupported = Guid.NewGuid();
                int result = Marshal.QueryInterface(instance, ref unsupported, out var unexpected);
                if (unexpected != 0) Marshal.Release(unexpected);
                if (result != unchecked((int)0x80004002))
                    throw new InvalidOperationException("Unexpected unsupported-interface result.");
            }
            finally { Marshal.Release(instance); }

            var html = PanelRenderer.Render(new RuntimeSnapshot(
                true, "playing", "", "<script>song</script>", "A&B", "<img src=x>",
                null, null, null, null, null, DateTimeOffset.UtcNow));
            if (html.Contains("<script>song", StringComparison.Ordinal) ||
                html.Contains("<img src=x>", StringComparison.Ordinal) ||
                !html.Contains("&lt;script&gt;song&lt;/script&gt;", StringComparison.Ordinal) ||
                !html.Contains("A&amp;B", StringComparison.Ordinal))
                throw new InvalidOperationException("Panel encoding failed.");

            int enabledCallbacks = 0;
            var lifecycle = new FeedProvider(runtime, _ => enabledCallbacks++);
            lifecycle.SetEnabled(false);
            lifecycle.SetEnabled(true);
            if (lifecycle.TryBeginDisabledShutdown())
                throw new InvalidOperationException("A stale timer stopped an enabled provider.");
            lifecycle.SetEnabled(false);
            if (!lifecycle.TryBeginDisabledShutdown())
                throw new InvalidOperationException("Disabled provider did not stop.");
            lifecycle.SetEnabled(true);
            if (enabledCallbacks != 3)
                throw new InvalidOperationException("A committed shutdown allowed reactivation.");
            return 0;
        }
        catch (Exception error)
        {
            ProviderLog.Error("self_test", error);
            Console.Error.WriteLine($"Provider self-test failed: 0x{error.HResult:X8}");
            return 1;
        }
        finally { runtime?.DisposeAsync().AsTask().GetAwaiter().GetResult(); }
    }

    private static void VerifyInterface(nint instance, Guid iid)
    {
        Marshal.ThrowExceptionForHR(Marshal.QueryInterface(instance, ref iid, out var result));
        if (result == 0) throw new InvalidOperationException("COM returned a null interface.");
        Marshal.Release(result);
    }

    private sealed class NoSystemSink : IAnnouncementSink
    {
        public void Request(Announcement announcement) => throw new InvalidOperationException("Unexpected host call.");
        public void Remove(string id) => throw new InvalidOperationException("Unexpected host call.");
    }

    [DllImport("ole32.dll", ExactSpelling = true)]
    private static extern int CoCreateInstance(
        ref Guid classId, nint outer, uint context, ref Guid iid, out nint result);
}
