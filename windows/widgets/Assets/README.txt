The build script exports the existing wocli music-note mark (U+266B, Segoe UI
Symbol, as used in src/tray.rs) to PNG assets on Windows. There is no image
generation service and no runtime taskbar drawing in this provider.

StoreLogo.png, Square44x44Logo.png and Square150x150Logo.png identify the package.
WidgetLight.png and WidgetDark.png are dark and light monochrome attribution
marks for the respective host backgrounds. The Windows host renders the live
Adaptive Card text, colors, controls and layout itself.

WidgetPreviewLight.png and WidgetPreviewDark.png are deterministic illustrative
picker previews, not captured screenshots or evidence of a Windows UI test.
They depict the medium card with sample title, artist and lyric placeholders.
The preview itself is labeled as an illustration. They follow Microsoft's picker
image requirements: 300x304 pixels and transparent rounded corners. These static
picker assets do not participate in rendering the live card or taskbar.
https://learn.microsoft.com/windows/apps/design/widgets/widgets-picker-integration

Generated assets are placed in the build output's Layout/Assets directory;
they are not written into the source checkout.
