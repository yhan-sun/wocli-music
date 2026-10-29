using Microsoft.Windows.Widgets.Feeds.Providers;
using Microsoft.Windows.Widgets.Notifications;
using Wocli.Widgets.Core;

namespace Wocli.Widgets;

/// <summary>Only requests system-owned announcements; it never draws a taskbar window.</summary>
internal sealed class SystemAnnouncementSink : IAnnouncementSink
{
    public void Request(Announcement announcement)
    {
        var notification = new FeedAnnouncement(
            announcement.Id,
            announcement.PrimaryText,
            announcement.SecondaryText,
            new Uri("ms-appx:///Assets/AnnouncementLight.png"),
            new Uri("ms-appx:///Assets/AnnouncementDark.png"))
        {
            PrimaryTextColor = AnnouncementTextColor.Default,
            SecondaryTextColor = AnnouncementTextColor.Default,
            IsSecondaryTextSubtle = true,
            ShowBadgeIfUserNotEngaged = false,
            Duration = announcement.Duration,
            ExpirationTime = announcement.ExpiresAt,
            CustomAccessibilityText = $"{announcement.PrimaryText}。{announcement.SecondaryText}"
        };

        // A successful call is a request, not evidence that Windows displayed it.
        FeedManager.GetDefault().TryShowAnnouncement(
            ProviderIdentity.ProviderId, ProviderIdentity.FeedId, notification);
    }

    public void Remove(string id) => FeedManager.GetDefault().TryRemoveAnnouncementById(
        ProviderIdentity.ProviderId, ProviderIdentity.FeedId, id);
}
