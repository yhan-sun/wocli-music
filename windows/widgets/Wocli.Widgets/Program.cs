using System.Runtime.InteropServices;
using Windows.ApplicationModel;
using Wocli.Widgets.Core;

namespace Wocli.Widgets;

internal static class Program
{
    [MTAThread]
    private static int Main(string[] args)
    {
        if (args.Length == 1 && args[0] == "--self-test")
            return ProviderSelfTest.Run();
        if (args.Length == 1 && args[0] == "--stop")
            return StopRunningProvider();
        if (!IsServerActivation(args))
            return 0;

        using var instance = new Mutex(false, ProviderIdentity.MutexName);
        bool acquired;
        try { acquired = instance.WaitOne(0); }
        catch (AbandonedMutexException) { acquired = true; }
        if (!acquired) return 0;

        try
        {
            using var apartment = new ComApartment();
            // Feed providers require package identity. Fail before touching playback
            // when someone runs the preview EXE outside its MSIX package.
            WinRT.ComWrappersSupport.InitializeComWrappers();
            _ = Package.Current.Id;
            using var stop = new EventWaitHandle(false, EventResetMode.ManualReset, ProviderIdentity.StopEventName);
            stop.Reset();
            FeedProvider? provider = null;
            using var disabledExit = new Timer(_ =>
            {
                if (provider?.TryBeginDisabledShutdown() == true) stop.Set();
            }, null, Timeout.Infinite, Timeout.Infinite);
            var runtime = new RuntimeLoop(new SystemAnnouncementSink());
            provider = new FeedProvider(runtime, enabled => disabledExit.Change(
                enabled ? Timeout.Infinite : 2_000, Timeout.Infinite));
            using var server = new ComServer(provider);
            try
            {
                runtime.Start();
                if (!provider.RestoreEnabledState()) return 1;
                // No busy wait: COM callbacks run in the MTA while this thread sleeps.
                stop.WaitOne();
            }
            finally
            {
                // Revoke first so a new activation cannot race announcement cleanup.
                provider.BeginShutdown();
                server.Dispose();
                disabledExit.Change(Timeout.Infinite, Timeout.Infinite);
                try { runtime.DisposeAsync().AsTask().GetAwaiter().GetResult(); }
                finally { disabledExit.DisposeAsync().AsTask().GetAwaiter().GetResult(); }
            }
            return 0;
        }
        catch (Exception error)
        {
            ProviderLog.Error("provider_process", error);
            return 1;
        }
        finally
        {
            instance.ReleaseMutex();
        }
    }

    internal static bool IsServerActivation(string[] args)
    {
        // COM may append -Embedding to the registered LocalServer32 arguments.
        return args.Length is 1 or 2 &&
            args.Count(arg => arg.Equals(ProviderIdentity.ServerArgument, StringComparison.OrdinalIgnoreCase)) == 1 &&
            args.All(arg => arg.Equals(ProviderIdentity.ServerArgument, StringComparison.OrdinalIgnoreCase) ||
                arg.Equals("-Embedding", StringComparison.OrdinalIgnoreCase));
    }

    private static int StopRunningProvider()
    {
        try
        {
            using var stop = EventWaitHandle.OpenExisting(ProviderIdentity.StopEventName);
            stop.Set();
            return 0;
        }
        catch (WaitHandleCannotBeOpenedException) { return 0; }
        catch (Exception error)
        {
            ProviderLog.Error("stop_provider", error);
            return 1;
        }
    }
}
