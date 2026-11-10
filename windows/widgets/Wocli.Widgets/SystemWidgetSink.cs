using Microsoft.Windows.Widgets.Providers;
using Wocli.Widgets.Core;

namespace Wocli.Widgets;

/// <summary>Windows renders the Adaptive Card with its own typography and theme.</summary>
internal sealed class SystemWidgetSink : IWidgetSink
{
    public void Update(string widgetId, WidgetCard card) => WidgetManager.GetDefault().UpdateWidget(
        new WidgetUpdateRequestOptions(widgetId)
        {
            Template = card.Template,
            Data = card.Data,
            // Song data and action authorization are ephemeral, never custom state.
            CustomState = ""
        });
}
