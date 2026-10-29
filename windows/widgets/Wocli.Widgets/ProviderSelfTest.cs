using System.Runtime.InteropServices;
using System.Text.Json;
using Microsoft.Windows.Widgets.Providers;
using Windows.ApplicationModel;
using Wocli.Widgets.Core;

namespace Wocli.Widgets;

internal static class ProviderSelfTest
{
    public static int Run(bool verifyPackagedProxy = false)
    {
        BoardRuntime? runtime = null;
        string stage = "activation arguments";
        try
        {
            Report($"Starting ({RuntimeInformation.FrameworkDescription}, {RuntimeInformation.OSDescription}, {RuntimeInformation.ProcessArchitecture})");
            Report(stage);
            if (!Program.IsServerActivation([ProviderIdentity.ServerArgument]) ||
                !Program.IsServerActivation([ProviderIdentity.ServerArgument, "-Embedding"]) ||
                !Program.IsServerActivation(["-embedding", ProviderIdentity.ServerArgument]) ||
                Program.IsServerActivation(["-Embedding"]) ||
                Program.IsServerActivation([ProviderIdentity.ServerArgument, "--self-test"]))
                throw new InvalidOperationException("COM activation argument parsing failed.");
            Report(stage = "initialize COM MTA");
            using var apartment = new ComApartment();
            Report(stage = "initialize C#/WinRT wrappers");
            WinRT.ComWrappersSupport.InitializeComWrappers();
            if (verifyPackagedProxy)
            {
                Report(stage = "require package identity");
                Report($"Packaged identity verified: {Package.Current.Id.FullName}");
            }
            Report(stage = "read projected interface identity");
            var type = typeof(IWidgetProvider);
            Report($"{type.FullName} IID={type.GUID:D}, GuidAttribute={type.GetCustomAttributes(typeof(GuidAttribute), false).Cast<GuidAttribute>().SingleOrDefault()?.Value ?? "missing"}");
            // This runtime is never started; no config, daemon, WidgetManager or
            // Widgets host is contacted by this test.
            Report(stage = "construct inactive provider");
            runtime = new BoardRuntime(new NoSystemSink());
            var provider = new WidgetProvider(runtime);
            var classId = Guid.NewGuid();
            Report(stage = "register temporary COM class");
            using var server = new ComServer(provider, classId, Report);
            var iid = typeof(IWidgetProvider).GUID;
            if (verifyPackagedProxy)
            {
                Report(stage = "inspect registered IWidgetProvider proxy");
                int proxyStatus = CoGetPSClsid(ref iid, out var proxyClassId);
                Report($"CoGetPSClsid HRESULT=0x{proxyStatus:X8}, CLSID={proxyClassId:D}; diagnostic only, real activation below determines success.");
            }
            // REGCLS_MULTIPLEUSE + CLSCTX_LOCAL_SERVER also registers the factory
            // in-process. The basic test validates the CCW without assuming SDK
            // proxy registration; packaged mode verifies the actual SDK proxy.
            // https://learn.microsoft.com/windows/win32/api/combaseapi/ne-combaseapi-regcls
            Report(verifyPackagedProxy
                ? "Scope: packaged local-server proxy activation for a temporary IWidgetProvider class; no Widgets host or display test."
                : "Scope: in-process COM factory/CCW and IWidgetProvider; SDK proxies, packaged activation and Widgets display require separate integration verification.");
            Report(stage = verifyPackagedProxy ? "CoCreateInstance packaged local-server IWidgetProvider" : "CoCreateInstance in-process IWidgetProvider");
            Marshal.ThrowExceptionForHR(CoCreateInstance(ref classId, 0, verifyPackagedProxy ? 4u : 1u, ref iid, out var instance));
            try
            {
                Report(stage = "QueryInterface IWidgetProvider");
                VerifyInterface(instance, typeof(IWidgetProvider).GUID);
                Report(stage = "reject unsupported interface");
                var unsupported = Guid.NewGuid();
                int result = Marshal.QueryInterface(instance, ref unsupported, out var unexpected);
                if (unexpected != 0) Marshal.Release(unexpected);
                if (result != unchecked((int)0x80004002))
                    throw new InvalidOperationException("Unexpected unsupported-interface result.");
            }
            finally { Marshal.Release(instance); }

            Report(stage = "reject COM aggregation");
            var factory = new WidgetClassFactory(provider);
            if (factory.CreateInstance((nint)1, ref iid, out var aggregated) != unchecked((int)0x80040110) || aggregated != 0)
                throw new InvalidOperationException("Factory accepted aggregation.");

            Report(stage = "restore and concurrent widget lifecycle");
            if (!provider.RestoreWidgets(() =>
            {
                provider.ObserveContext(new WidgetRegistration("newer", CardSize.Large, true));
                provider.Deactivate("inactive");
                // Resize may carry the older IsActive value while the startup
                // query is in flight. Only Activate may reverse Deactivate.
                provider.ObserveContext(new WidgetRegistration("inactive", CardSize.Large, true), preserveActive: true);
                provider.DeleteWidget("deleted", "");
                provider.ObserveContext(new WidgetRegistration("deleted", CardSize.Large, true), preserveActive: true);
                return new[]
                {
                    new WidgetRegistration("newer", CardSize.Small, false),
                    new WidgetRegistration("inactive", CardSize.Small, true),
                    new WidgetRegistration("deleted", CardSize.Small, true),
                    new WidgetRegistration("restored", CardSize.Medium, false)
                };
            })) throw new InvalidOperationException("Restoration failed.");
            CheckCounts(runtime, 3, 1);
            provider.ObserveContext(new WidgetRegistration("newer", CardSize.Small, false), preserveActive: true);
            CheckCounts(runtime, 3, 1);
            provider.Deactivate("newer");
            CheckCounts(runtime, 3, 0);
            provider.ObserveContext(new WidgetRegistration("newer", CardSize.Large, true));
            provider.DeleteWidget("inactive", "");
            provider.ObserveContext(new WidgetRegistration("inactive", CardSize.Small, true), preserveActive: true);
            CheckCounts(runtime, 2, 1);
            provider.BeginShutdown();
            provider.ObserveContext(new WidgetRegistration("after-shutdown", CardSize.Large, true));
            CheckCounts(runtime, 2, 0);

            Report(stage = "Adaptive Card data isolation and system styling");
            var lyrics = new LyricsSnapshot(true, false, "playing", "private-track",
                "<script>song</script>", "A&B", "\"${title}\" <img src=x>");
            var card = WidgetCards.Create(CardSize.Large, "playing", lyrics, new string('a', 32));
            using (var template = JsonDocument.Parse(card.Template))
            using (var data = JsonDocument.Parse(card.Data))
            {
                if (template.RootElement.GetProperty("type").GetString() != "AdaptiveCard" ||
                    data.RootElement.GetProperty("title").GetString() != lyrics.Title ||
                    data.RootElement.GetProperty("line").GetString() != lyrics.Line ||
                    card.Template.Contains("<script>", StringComparison.Ordinal) ||
                    card.Template.Contains("Action.Submit", StringComparison.Ordinal) ||
                    card.Template.Contains("backgroundImage", StringComparison.Ordinal) ||
                    card.Template.Contains("fontFamily", StringComparison.Ordinal) ||
                    card.Template.Contains("\"color\"", StringComparison.Ordinal) ||
                    !card.Template.Contains("Action.Execute", StringComparison.Ordinal) ||
                    card.Data.Contains("private-track", StringComparison.Ordinal))
                    throw new InvalidOperationException("Adaptive Card rendering contract failed.");
            }
            var hidden = WidgetCards.Create(CardSize.Small, "boss", lyrics with { Boss = true });
            using (var data = JsonDocument.Parse(hidden.Data))
                if (data.RootElement.GetProperty("title").GetString() != "" ||
                    data.RootElement.GetProperty("line").GetString() != "" ||
                    hidden.Template.Contains("Action.Execute", StringComparison.Ordinal))
                    throw new InvalidOperationException("Hidden card retained private content or controls.");
            if (verifyPackagedProxy)
            {
                Report(stage = "activate packaged WidgetUpdateRequestOptions without contacting the host");
                var options = new WidgetUpdateRequestOptions(Guid.NewGuid().ToString())
                {
                    Template = card.Template,
                    Data = card.Data,
                    CustomState = ""
                };
                if (options.Template != card.Template || options.Data != card.Data || options.CustomState != "")
                    throw new InvalidOperationException("Packaged widget request activation failed.");
            }

            Report("All provider self-tests passed.");
            return 0;
        }
        catch (Exception error)
        {
            ProviderLog.Error("self_test", error);
            Console.Error.WriteLine($"Provider self-test failed at '{stage}': {error.GetType().FullName}, HRESULT=0x{error.HResult:X8}: {error}");
            Console.Error.Flush();
            return 1;
        }
        finally { runtime?.DisposeAsync().AsTask().GetAwaiter().GetResult(); }
    }

    private static void CheckCounts(BoardRuntime runtime, int expectedWidgets, int expectedActive)
    {
        var snapshot = runtime.GetSnapshot();
        if (snapshot.WidgetCount != expectedWidgets || snapshot.ActiveCount != expectedActive)
            throw new InvalidOperationException("Widget lifecycle state did not match the latest callbacks.");
    }

    private static void Report(string text)
    {
        // This diagnostic path uses only synthetic test data and never starts IPC.
        Console.WriteLine($"[provider self-test] {text}");
        Console.Out.Flush();
    }

    private static void VerifyInterface(nint instance, Guid iid)
    {
        Marshal.ThrowExceptionForHR(Marshal.QueryInterface(instance, ref iid, out var result));
        if (result == 0) throw new InvalidOperationException("COM returned a null interface.");
        Marshal.Release(result);
    }

    private sealed class NoSystemSink : IWidgetSink
    {
        public void Update(string widgetId, WidgetCard card) => throw new InvalidOperationException("Unexpected host call.");
    }

    [DllImport("ole32.dll", ExactSpelling = true)]
    private static extern int CoCreateInstance(
        ref Guid classId, nint outer, uint context, ref Guid iid, out nint result);

    [DllImport("ole32.dll", ExactSpelling = true)]
    private static extern int CoGetPSClsid(ref Guid iid, out Guid classId);
}
