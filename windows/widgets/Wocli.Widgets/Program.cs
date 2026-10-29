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
        if (args.Length == 1 && args[0] == "--self-test-packaged-proxy")
            return ProviderSelfTest.Run(verifyPackagedProxy: true);
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
            // Widget providers require package identity. Fail before touching IPC
            // when someone runs the EXE outside its MSIX package.
            WinRT.ComWrappersSupport.InitializeComWrappers();
            _ = Package.Current.Id;
            using var stop = new EventWaitHandle(false, EventResetMode.ManualReset, ProviderIdentity.StopEventName);
            stop.Reset();
            var runtime = new BoardRuntime(new SystemWidgetSink());
            var provider = new WidgetProvider(runtime);
            using var server = new ComServer(provider);
            try
            {
                runtime.Start();
                if (!provider.RestoreWidgets()) return 1;
                // No idle-exit timer can race a new host activation. The runtime
                // suspends lyric IPC when no widget is active; COM callbacks run
                // in the MTA while this thread sleeps until explicit shutdown.
                stop.WaitOne();
            }
            finally
            {
                // Revoke first so a new activation cannot race card cleanup.
                provider.BeginShutdown();
                server.Dispose();
                runtime.DisposeAsync().AsTask().GetAwaiter().GetResult();
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
