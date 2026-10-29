using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Windows.Widgets.Feeds.Providers;
using Microsoft.Windows.Widgets.Notifications;
using Windows.Storage.Streams;
using Wocli.Widgets.Core;

namespace Wocli.Widgets;

[ComVisible(true)]
[ClassInterface(ClassInterfaceType.None)]
[ComDefaultInterface(typeof(IFeedProvider))]
[Guid(ProviderIdentity.ClassId)]
public sealed class FeedProvider : IFeedProvider, IFeedAnnouncementInvokedTarget, IFeedResourceProvider
{
    private readonly RuntimeLoop _runtime;
    private readonly Action<bool> _enabledChanged;
    private readonly object _gate = new();
    private long _enableGeneration;
    private bool _enabled;
    private bool _stopping;

    internal FeedProvider(RuntimeLoop runtime, Action<bool> enabledChanged)
    {
        _runtime = runtime;
        _enabledChanged = enabledChanged;
    }

    public void OnFeedProviderEnabled(FeedProviderEnabledArgs args)
    {
        if (args.FeedProviderDefinitionId == ProviderIdentity.ProviderId)
            SetEnabled(true);
    }

    public void OnFeedProviderDisabled(FeedProviderDisabledArgs args)
    {
        if (args.FeedProviderDefinitionId == ProviderIdentity.ProviderId)
            SetEnabled(false);
    }

    public void OnFeedEnabled(FeedEnabledArgs args)
    {
        if (args.FeedProviderDefinitionId == ProviderIdentity.ProviderId && args.FeedDefinitionId == ProviderIdentity.FeedId)
            SetEnabled(true);
    }

    public void OnFeedDisabled(FeedDisabledArgs args)
    {
        if (args.FeedProviderDefinitionId == ProviderIdentity.ProviderId && args.FeedDefinitionId == ProviderIdentity.FeedId)
            SetEnabled(false);
    }

    public void OnCustomQueryParametersRequested(CustomQueryParametersRequestedArgs args)
    {
        if (args.FeedProviderDefinitionId != ProviderIdentity.ProviderId)
            return;
        // No authentication data, song information or local file paths enter a URL.
        try
        {
            FeedManager.GetDefault().SetCustomQueryParameters(
                new CustomQueryParametersUpdateOptions(ProviderIdentity.ProviderId, ""));
        }
        catch (Exception error)
        {
            ProviderLog.Error("query_parameters", error);
        }
    }

    internal bool RestoreEnabledState()
    {
        long generation;
        lock (_gate) generation = _enableGeneration;
        try
        {
            bool enabled = FeedManager.GetDefault().GetEnabledFeedProviders().Any(provider =>
                provider.FeedProviderDefinitionId == ProviderIdentity.ProviderId &&
                provider.EnabledFeedDefinitionIds.Contains(ProviderIdentity.FeedId));
            lock (_gate)
            {
                // An activation/disable callback newer than this query takes precedence.
                if (!_stopping && generation == _enableGeneration)
                    SetEnabledLocked(enabled);
            }
            return true;
        }
        catch (Exception error)
        {
            ProviderLog.Error("restore_enabled", error);
            // Stay disabled until the platform explicitly enables this provider.
            return false;
        }
    }

    internal void SetEnabled(bool enabled)
    {
        lock (_gate)
        {
            if (_stopping) return;
            _enableGeneration++;
            SetEnabledLocked(enabled);
        }
    }

    internal void BeginShutdown()
    {
        lock (_gate)
        {
            _stopping = true;
            _runtime.SetEnabled(false);
        }
    }

    internal bool TryBeginDisabledShutdown()
    {
        lock (_gate)
        {
            // Timer.Change cannot cancel a callback already queued. Commit the
            // exit under the same lock as enable callbacks so a stale timer
            // cannot close a provider which has since been enabled again.
            if (_enabled || _stopping) return false;
            _stopping = true;
            _runtime.SetEnabled(false);
            return true;
        }
    }

    private void SetEnabledLocked(bool enabled)
    {
        _enabled = enabled;
        _runtime.SetEnabled(enabled);
        _enabledChanged(enabled);
    }

    public void OnAnnouncementInvoked(FeedAnnouncementInvokedArgs args)
    {
        if (args.FeedProviderDefinitionId != ProviderIdentity.ProviderId || args.FeedDefinitionId != ProviderIdentity.FeedId)
            return;
        // Copy callback data now. WinRT event argument objects must not be retained.
        var id = args.AnnouncementId;
        switch (args.ActionKind)
        {
            case AnnouncementActionKind.Shown:
                _runtime.MarkShown(id);
                break;
            case AnnouncementActionKind.Engaged:
                _runtime.MarkEngaged(id);
                break;
        }
    }

    public void OnResourceRequested(FeedResourceRequestedArgs args)
    {
        // Every intercepted request gets a response, including errors. Returning with
        // Response unset asks Windows to fetch the URL over the network.
        try
        {
            bool allowed = args.FeedProviderDefinitionId == ProviderIdentity.ProviderId &&
                args.FeedDefinitionId == ProviderIdentity.FeedId &&
                args.Request.Method == "GET" &&
                Uri.TryCreate(args.Request.Uri, UriKind.Absolute, out var uri) &&
                uri.Scheme == Uri.UriSchemeHttps && uri.Host == ProviderIdentity.Host &&
                uri.IsDefaultPort && uri.AbsolutePath == "/lyrics" &&
                string.IsNullOrEmpty(uri.UserInfo);
            args.Response = allowed
                ? CreateResponse(PanelRenderer.Render(_runtime.GetSnapshot()), 200, "OK")
                : CreateResponse("Not found", 404, "Not Found");
        }
        catch (Exception error)
        {
            ProviderLog.Error("local_resource", error);
            args.Response = CreateResponse("Local lyrics are unavailable.", 503, "Service Unavailable");
        }
    }

    private static FeedResourceResponse CreateResponse(string text, int status, string reason)
    {
        // The stream reference owns the stream for as long as the host needs it.
        var stream = new MemoryStream(Encoding.UTF8.GetBytes(text), writable: false).AsRandomAccessStream();
        return new FeedResourceResponse(RandomAccessStreamReference.CreateFromStream(stream), reason, status)
        {
            Headers = new Dictionary<string, string>
            {
                ["Content-Type"] = "text/html; charset=utf-8",
                ["Cache-Control"] = "no-store",
                ["X-Content-Type-Options"] = "nosniff",
                ["Content-Security-Policy"] = "default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'"
            }
        };
    }
}
