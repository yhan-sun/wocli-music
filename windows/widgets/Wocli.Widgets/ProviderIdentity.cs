namespace Wocli.Widgets;

internal static class ProviderIdentity
{
    // Must match Package.appxmanifest. These are application identities, not OS ABI IDs.
    public const string ClassId = "8B3EF0C4-86E8-4D42-A154-76492840B5EB";
    public const string ProviderId = "com.wocli.music.lyrics";
    public const string FeedId = "Wocli_Lyrics";
    public const string Host = "wocli.invalid";
    public const string ServerArgument = "-RegisterProcessAsComServer";
    public const string MutexName = @"Local\Wocli.Widgets.8B3EF0C4.Instance";
    public const string StopEventName = @"Local\Wocli.Widgets.8B3EF0C4.Stop";
}
