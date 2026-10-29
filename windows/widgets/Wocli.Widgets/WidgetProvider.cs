using System.Runtime.InteropServices;
using System.Text.Json;
using Microsoft.Windows.Widgets;
using Microsoft.Windows.Widgets.Providers;
using Wocli.Widgets.Core;

namespace Wocli.Widgets;

// Only value data crosses the WinRT callback boundary. WidgetContext and event
// argument objects are valid only for the duration of the callback.
internal sealed record WidgetRegistration(string Id, CardSize Size, bool Active);

[ComVisible(true)]
[ClassInterface(ClassInterfaceType.None)]
[ComDefaultInterface(typeof(IWidgetProvider))]
[Guid(ProviderIdentity.ClassId)]
public sealed class WidgetProvider : IWidgetProvider
{
    private const int MaximumWidgets = 64;
    private readonly BoardRuntime _runtime;
    private readonly object _gate = new();
    private readonly Dictionary<string, WidgetRegistration> _widgets = new(StringComparer.Ordinal);
    private readonly HashSet<string> _deletedDuringRestore = new(StringComparer.Ordinal);
    private readonly HashSet<string> _inactiveDuringRestore = new(StringComparer.Ordinal);
    private bool _restoring = true;
    private bool _restoreOverflow;
    private bool _stopping;

    internal WidgetProvider(BoardRuntime runtime) => _runtime = runtime;

    public void CreateWidget(WidgetContext widgetContext) => Callback("create_widget", () =>
    {
        if (CopyContext(widgetContext) is { } context) ObserveContext(context);
    });

    public void Activate(WidgetContext widgetContext) => Callback("activate_widget", () =>
    {
        if (CopyContext(widgetContext) is { } context) ObserveContext(context with { Active = true });
    });

    public void OnWidgetContextChanged(WidgetContextChangedArgs contextChangedArgs) => Callback("resize_widget", () =>
    {
        if (CopyContext(contextChangedArgs.WidgetContext) is { } context)
            ObserveContext(context, preserveActive: true);
    });

    public void Deactivate(string widgetId) => Callback("deactivate_widget", () =>
    {
        if (!ValidId(widgetId)) return;
        lock (_gate)
        {
            if (_stopping) return;
            RecordRestoreChange(_inactiveDuringRestore, widgetId);
            if (_widgets.TryGetValue(widgetId, out var widget))
                _widgets[widgetId] = widget with { Active = false };
            _runtime.Deactivate(widgetId);
        }
    });

    public void DeleteWidget(string widgetId, string customState) => Callback("delete_widget", () =>
    {
        if (!ValidId(widgetId)) return;
        lock (_gate)
        {
            if (_stopping) return;
            RecordRestoreChange(_deletedDuringRestore, widgetId);
            _widgets.Remove(widgetId);
            _runtime.Delete(widgetId);
        }
    });

    public void OnActionInvoked(WidgetActionInvokedArgs actionInvokedArgs) => Callback("widget_action", () =>
    {
        var context = CopyContext(actionInvokedArgs.WidgetContext);
        if (context is null) return;
        string verb = actionInvokedArgs.Verb;
        string json = actionInvokedArgs.Data;
        if (verb.Length > 32 || json.Length > 8_192) return;
        using var document = JsonDocument.Parse(string.IsNullOrWhiteSpace(json) ? "{}" : json,
            new JsonDocumentOptions { MaxDepth = 16 });
        if (document.RootElement.ValueKind != JsonValueKind.Object) return;
        // Clone before returning; neither the document nor WinRT args are retained.
        JsonElement data = document.RootElement.Clone();
        lock (_gate)
        {
            if (_stopping || !_widgets.TryGetValue(context.Id, out var current) || !current.Active) return;
            _ = DispatchActionAsync(context.Id, verb, data);
        }
    });

    private async Task DispatchActionAsync(string id, string verb, JsonElement data)
    {
        try { await _runtime.HandleActionAsync(id, verb, data).ConfigureAwait(false); }
        catch (Exception error) { ProviderLog.Error("widget_action_dispatch", error); }
    }

    internal bool RestoreWidgets(Func<IReadOnlyList<WidgetRegistration>>? query = null)
    {
        try
        {
            // Query outside the lock: the host may send callbacks while enumerating.
            // Copy the results immediately and let newer callbacks win per widget.
            var restored = query is null ? ReadRegisteredWidgets() : query();
            lock (_gate)
            {
                if (_stopping) return false;
                if (_restoreOverflow)
                    throw new InvalidOperationException("Too many widget changes during restoration.");
                foreach (var context in restored)
                {
                    if (!ValidId(context.Id) || _deletedDuringRestore.Contains(context.Id) ||
                        _widgets.ContainsKey(context.Id)) continue;
                    if (_widgets.Count >= MaximumWidgets) break;
                    var current = _inactiveDuringRestore.Contains(context.Id)
                        ? context with { Active = false } : context;
                    _widgets[current.Id] = current;
                    _runtime.Restore(current.Id, current.Size, current.Active);
                }
            }
            return true;
        }
        catch (Exception error)
        {
            ProviderLog.Error("restore_widgets", error);
            return false;
        }
        finally
        {
            lock (_gate)
            {
                _restoring = false;
                _restoreOverflow = false;
                _deletedDuringRestore.Clear();
                _inactiveDuringRestore.Clear();
            }
        }
    }

    private static IReadOnlyList<WidgetRegistration> ReadRegisteredWidgets()
    {
        var result = new List<WidgetRegistration>();
        foreach (var info in WidgetManager.GetDefault().GetWidgetInfos())
        {
            if (CopyContext(info.WidgetContext) is not { } context) continue;
            result.Add(context);
            if (result.Count >= MaximumWidgets) break;
        }
        return result;
    }

    internal void ObserveContext(WidgetRegistration context, bool preserveActive = false)
    {
        if (!ValidId(context.Id)) return;
        lock (_gate)
        {
            if (_stopping) return;
            // A resize cannot recreate a widget whose deletion won the race.
            // Unknown sizes can be restored only during the initial host query.
            if (preserveActive && (_deletedDuringRestore.Contains(context.Id) ||
                (!_restoring && !_widgets.ContainsKey(context.Id)))) return;
            if (!_widgets.ContainsKey(context.Id) && _widgets.Count >= MaximumWidgets) return;
            if (preserveActive && _widgets.TryGetValue(context.Id, out var current))
                context = context with { Active = current.Active };
            else if (preserveActive && _inactiveDuringRestore.Contains(context.Id))
                context = context with { Active = false };
            _deletedDuringRestore.Remove(context.Id);
            if (context.Active) _inactiveDuringRestore.Remove(context.Id);
            _widgets[context.Id] = context;
            _runtime.Upsert(context.Id, context.Size, context.Active);
        }
    }

    // Callback IDs are bounded even if the host stalls during enumeration. On
    // overflow restoration fails closed instead of reviving an untracked delete.
    // All callers hold _gate.
    private void RecordRestoreChange(HashSet<string> changes, string id)
    {
        if (!_restoring || changes.Contains(id)) return;
        if (changes.Count >= MaximumWidgets * 4) { _restoreOverflow = true; return; }
        changes.Add(id);
    }

    internal void BeginShutdown()
    {
        lock (_gate)
        {
            if (_stopping) return;
            _stopping = true;
            foreach (string id in _widgets.Keys) _runtime.Deactivate(id);
            _widgets.Clear();
        }
    }

    private static WidgetRegistration? CopyContext(WidgetContext context)
    {
        if (context.DefinitionId != ProviderIdentity.WidgetId || !ValidId(context.Id)) return null;
        return new WidgetRegistration(context.Id, context.Size switch
        {
            WidgetSize.Small => CardSize.Small,
            WidgetSize.Large => CardSize.Large,
            _ => CardSize.Medium
        }, context.IsActive);
    }

    private static bool ValidId(string? id) => !string.IsNullOrWhiteSpace(id) && id.Length <= 512;

    private static void Callback(string operation, Action action)
    {
        try { action(); }
        catch (Exception error) { ProviderLog.Error(operation, error); }
    }
}
