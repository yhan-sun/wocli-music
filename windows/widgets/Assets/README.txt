The build script exports the existing wocli music-note mark (U+266B, Segoe UI
Symbol, as used in src/tray.rs) to PNG assets on Windows. There is no image
generation service and no runtime taskbar drawing in this provider.

StoreLogo.png, Square44x44Logo.png and Square150x150Logo.png identify the package.
AnnouncementLight.png and AnnouncementDark.png are respectively dark and light
monochrome marks for the host's light and dark backgrounds. The Windows host
renders the announcement text, colors and layout itself.

Generated assets are placed in the build output's Layout/Assets directory;
they are not written into the source checkout.
