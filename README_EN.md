<div align="center">

  <img src="docs/screenshots/library.png" width="820" alt="wocli-music" style="border-radius: 10px; box-shadow: 0 8px 32px rgba(0,0,0,0.35); max-width: 100%;" />

  # wocli-music

  **Rediscover pure listening. Seamless music for your terminal and workflow.**
  <br />
  *Blazing-fast, zero-bloat, native music player for your terminal, scripts, and AI workflows.*

  <p align="center">
    <a href="https://github.com/yhan-sun/wocli-music/releases/latest"><img src="https://img.shields.io/github/v/release/yhan-sun/wocli-music?color=7aa874&label=release&logo=github&style=flat-square" alt="Release" /></a>
    <img src="https://img.shields.io/badge/Rust-1.93%2B-e43716?logo=rust&style=flat-square" alt="Rust" />
    <img src="https://img.shields.io/badge/Platform-macOS%20%7C%20Linux%20%7C%20Windows-blue?style=flat-square" alt="Platforms" />
    <img src="https://img.shields.io/badge/Binary_Size-~3.5MB-success?style=flat-square" alt="Binary Size" />
    <img src="https://img.shields.io/badge/RAM_Footprint-~20MB-brightgreen?style=flat-square" alt="RAM Footprint" />
    <img src="https://img.shields.io/badge/Auth-No_Login_Required-blueviolet?style=flat-square" alt="No Login" />
    <a href="docs/mcp.md"><img src="https://img.shields.io/badge/AI-MCP_Native-8a2be2?logo=anthropic&style=flat-square" alt="MCP" /></a>
    <a href="LICENSE"><img src="https://img.shields.io/badge/License-GPL--3.0-lightgrey?style=flat-square" alt="License" /></a>
  </p>

  <p align="center">
    <a href="#quick-start">Quick Start</a> •
    <a href="#why-did-we-build-this">Why Build It</a> •
    <a href="#side-by-side-experience-comparison">Comparison</a> •
    <a href="#core-user-experience">Experience</a> •
    <a href="#keybindings-cheat-sheet">Keybindings</a> •
    <a href="#ai-agent-integration-mcp">AI MCP</a> •
    <a href="#cli-reference">CLI</a> •
    <a href="#advanced-configuration">Config</a> •
    <a href="README.md">中文文档</a>
  </p>

</div>

---

## Why Did We Build This?

All we wanted was to listen to a song in peace. But modern music apps have lost their way.

You have probably experienced these frustrating moments:
- You just want to play a song, but you have to download a 300MB installer and sit through a 5-second splash screen advertisement every single time you launch it.
- You find the song you want, only to be immediately blocked by popups: "Scan QR code to log in", "Subscribe to VIP to unlock full track".
- The interface is cluttered with live streams, short videos, algorithmic feeds, and marketing banners—silently hogging hundreds of megabytes or even gigabytes of RAM in the background.
- At your desk or study space, switching tracks requires fumbling across windows; when a colleague or manager walks by, massive album art glares from the center of your screen before you can even hit pause.

Listening to music should be lightweight, pure, and non-intrusive.

`wocli-music` was created to restore that freedom:

- **100% Login-Free, Zero Annoyances**: No phone numbers, no QR codes, no VIP paywalls, and zero ads. Type a song name and it plays immediately.
- **Invisibly Small, Blazing Fast**: The entire executable is only ~3MB. It starts and plays in milliseconds. Running in the background consumes only ~20MB of RAM—effortless on any laptop or cloud server.
- **Work Camouflage & Instant Stealth (Boss Mode)**: Purpose-built for the workplace. When someone approaches, hit a quick hotkey: audio mutes instantly, and the full screen turns into an authentic compilation and unit-test terminal. Hit it again when the coast is clear to resume right where you left off.
- **Native Desktop Companion, Zero Screen Clutter**: Pick your music in the terminal, close the window, and music continues playing silently in the background. Glance at rolling lyrics in your macOS menu bar or interact with the taskbar lyrics strip on Windows without switching windows.
- **Let AI Be Your Personal DJ**: When coding or writing in Claude or Cursor, simply ask: *"Play some chill lo-fi beats for deep focus"*. Your AI assistant searches, plays, adjusts volume, or manages playlists directly via MCP.

---

## Side-by-Side Experience Comparison

Whether you are hacking in a remote terminal or looking for a distraction-free, memory-efficient desktop companion, see how `wocli-music` compares:

| Comparison Metric | wocli-music | Official Clients<br>*(Spotify / NetEase / QQ)* | LX Music Desktop | go-musicfox | ncmpcpp + MPD |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **Account / Login Required** | **100% Login-Free**<br>Instant access, zero paywalls | **Mandatory Login**<br>Limited previews without VIP subscription | **Login-Free**<br>Relies on user-provided scripts | **Mandatory Login**<br>Requires QR/Cookie NetEase binding | **Login-Free**<br>Local audio library |
| **RAM Footprint** | **~15 – 30 MB**<br>Native Rust minimal footprint | **300 MB – 1.2 GB+**<br>Bulky multi-process Chromium/Electron | **150 – 450 MB**<br>Electron + Web engine | **40 – 95 MB**<br>Go runtime + terminal UI | **15 – 40 MB**<br>Native C/C++ daemon + client |
| **App / Binary Size** | **~3.0 – 4.2 MB**<br>Self-contained single binary | **150 – 350 MB**<br>Massive desktop installer | **80 – 120 MB**<br>Electron cross-platform package | **20 – 35 MB**<br>Go compiled binary | **5 – 15 MB**<br>Requires system-level MPD packages |
| **Cold Startup Latency** | **< 10ms instantaneous**<br>No splash screen, no ads | **2 – 6 seconds**<br>Splash ads, newsfeed & auth check | **1.5 – 3 seconds**<br>Webview engine loading time | **< 100ms**<br>Fast terminal launch | **< 50ms**<br>Requires pre-running MPD daemon |
| **Interface Modalities** | **TUI + CLI + AI MCP + Tray Lyrics**<br>Unified shared background state | **Desktop GUI Window**<br>Heavy social feed, banners & streams | **Desktop GUI Window**<br>Clean standalone player | **Terminal TUI**<br>NetEase ecosystem focused | **Terminal TUI (C/S)**<br>Local tag & library management |
| **AI Agent Support (MCP)** | **Built-in native stdio MCP**<br>Claude & Cursor play & curate via prompt | **None**<br>Walled garden | **Limited**<br>Requires custom local HTTP API wrapper | **None** | **None**<br>Legacy MPD socket protocol only |
| **Boss Mode (Stealth)** | **Simulates cargo test terminal**<br>Mutes audio & hides all OS indicators | **None**<br>Minimizing window keeps audio playing | **None**<br>Standard minimize to tray | **None** | **None** |
| **External Dependencies** | **Zero Dependencies**<br>Built-in decoders, audio stack & JS engine | Bundled Chromium & Node runtime | Bundled Chromium & Node runtime | Requires system audio decoders | Requires separate MPD server setup |

> **Methodology**: Benchmark data verified as of October 2026. `wocli-music` package size reflects the official release archive; RAM footprint reflects actual Resident Set Size (RSS) during 320kbps playback. See [Detailed Comparison Notes](docs/comparison.md) for full technical context.

---

## Quick Start

No need to install Rust, Python, Node.js, mpv, or FFmpeg. Copy and run the single command below:

### 1. One-Line Install

#### macOS / Linux (Terminal)
```bash
curl -fsSL https://wocli.yhan.fun/install.sh | bash
```

#### Windows (PowerShell)
```powershell
irm https://wocli.yhan.fun/install.ps1 | iex
```

### 2. Basic Usage

Open any terminal and start playing:

```bash
wocli                          # Launch interactive TUI (/ search, Enter play, Space pause)
wocli play 'Jay Chou Sunny'    # CLI: search and start playback immediately
wocli status --json             # Output structured JSON status for scripting
wocli mcp prompt                # Output copy-paste AI connection prompt
```

> **Tip**: Press <kbd>q</kbd> in the TUI to exit the interface anytime; **music playback and queued downloads continue uninterrupted in the background daemon**. Run `wocli stop` to pause/stop playback, or `wocli daemon stop` to terminate the background service.

---

## Visual Showcase

<div align="center">
  <table>
    <tr>
      <td width="50%">
        <img src="docs/screenshots/library.png" alt="Library and Lyrics" /><br />
        <p align="center"><b>Library Browsing, Transport Controls & Synced Lyrics</b></p>
      </td>
      <td width="50%">
        <img src="docs/screenshots/collections.png" alt="Playlists & Collections" /><br />
        <p align="center"><b>Playlists, Custom Collections & Library Switching</b></p>
      </td>
    </tr>
    <tr>
      <td colspan="2">
        <img src="docs/screenshots/compact.png" alt="80x26 Compact Mode" /><br />
        <p align="center"><b>Responsive Layout: Gracefully adapts down to an 80×26 mini terminal or tmux pane</b></p>
      </td>
    </tr>
  </table>
</div>

---

## Core User Experience

### 1. Interactive Terminal UI (TUI)
- **Keyboard & Mouse**: Full Vim-style navigation (`j` / `k` move, `g` / `G` top/bottom, `/` search), alongside full mouse support for seeking, clicking buttons, and double-clicking to play.
- **Dynamic Synced Lyrics**: Real-time line-by-line highlight; press <kbd>v</kbd> to enter full-screen immersive lyrics mode.
- **Unified Online & Local Library**: Combine local FLAC/MP3 files and online streams into custom playlists.
- **Background Downloads**: Press <kbd>d</kbd> on any song to queue high-quality audio and `.lrc` downloads.

### 2. Authentic Boss Mode (Work Camouflage)
A dedicated feature designed for office, classroom, or public environments:
- **Terminal Camouflage**: Press <kbd>Shift</kbd>+<kbd>B</kbd> or <kbd>Ctrl</kbd>+<kbd>B</kbd> in the TUI to **immediately pause/mute audio** and switch the entire screen to a simulated terminal running `cargo test --workspace` (with 18 passing unit tests and an active command prompt `█`).
- **Instant Resume**: Press <kbd>B</kbd>, <kbd>Ctrl</kbd>+<kbd>B</kbd>, or <kbd>Esc</kbd> to return to your player and **seamlessly resume music**.
- **Desktop Stealth**:
  - **Windows**: Global hotkeys <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>B</kbd> or <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>B</kbd> pause audio and hide the taskbar lyrics bar and tray tooltip details.
  - **macOS**: Menu bar item provides a one-click "Boss Mode" option that clears Now Playing info and hides lyrics.
  - **Linux**: Easily bind `wocli boss` to any desktop shortcut.

### 3. Taskbar & Menu Bar Lyrics
Keep an eye on what's playing without switching windows:
- **macOS**: Native menu bar item displays live synced lyrics; integrates with `MPRemoteCommandCenter` and hardware media keys.
- **Windows**: **Exclusive embedded taskbar lyrics bar** positioned neatly inside the Windows taskbar, featuring built-in previous, play/pause, and next buttons.
- Command-line controls:
  ```bash
  wocli tray start           # Start the system tray daemon
  wocli lyrics-bar on        # Show taskbar / menu bar lyrics
  wocli lyrics-bar off       # Hide taskbar lyrics
  wocli lyrics-bar toggle    # Toggle lyrics visibility
  ```

---

## AI Agent Integration (MCP)

`wocli-music` implements the official **Model Context Protocol (MCP)** specification over standard I/O (stdio). It allows local AI clients to control music playback safely without opening network ports.

```
 ┌──────────────────────┐        stdio (JSON-RPC)       ┌────────────────────────┐
 │  Claude / Cursor /   │ ────────────────────────────> │  wocli mcp serve       │
 │  AI Clients          │ <──────────────────────────── │  (17 Built-in Tools)   │
 └──────────────────────┘                               └───────────┬────────────┘
                                                                    │ IPC
                                                        ┌───────────▼────────────┐
                                                        │  wocli Background Svc  │
                                                        └────────────────────────┘
```

### Configuration Example (Claude Desktop / Cursor)

Run `wocli mcp prompt` to generate a tailored setup prompt, or add this to your `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "wocli-music": {
      "command": "wocli",
      "args": ["mcp", "serve"]
    }
  }
}
```

Once connected, your AI assistant can:
- *"Play some relaxing instrumental music for deep work"*
- *"Save the currently playing song to my Favorites"*
- *"Skip to the next song and set the volume to 50"*
- *"Download the top 3 tracks by Coldplay in lossless quality"*

> See [MCP Protocol Reference & Guide](docs/mcp.md) for full details.

---

## CLI Reference

Every action can be invoked directly from your command line:

```bash
# Playback Controls
wocli play 'Radiohead Creep'      # Search and play top result
wocli play --id 61631119 --quality best  # Play by ID with highest quality
wocli play ~/Music/song.flac      # Play a local audio file
wocli toggle                      # Play / pause toggle
wocli next                        # Skip to next track (--no-wait for async)
wocli prev                        # Skip to previous track
wocli seek +15                    # Seek forward 15s (supports negative offset)
wocli volume 65                   # Set volume (0 - 100)
wocli mode shuffle                # Set playback mode: list / repeat / single / shuffle

# Library & Queue
wocli status --json               # Get structured JSON status
wocli like                        # Toggle favorite for the current track
wocli playlist create 'Coding'    # Create a new playlist
wocli playlist add 'Coding' --id 61631119 # Add track to playlist
wocli playlist play 'Coding'      # Play entire playlist

# Boss Mode & Stealth
wocli boss                        # Toggle Boss Mode (mute & hide visual indicators)
wocli boss on                     # Enable Boss Mode
wocli boss off                    # Exit Boss Mode and resume playback

# Downloads & Sources
wocli download 'Song' --pick 1 --quality 320k --lyrics --output ~/Music
wocli source list                 # List active resolution sources
wocli retry                       # Retry playback across fallback sources
```

---

## Keybindings Cheat Sheet

In the TUI, physical media keys and the following shortcuts are supported:

| Shortcut | Description |
| :--- | :--- |
| <kbd>j</kbd> / <kbd>k</kbd> or <kbd>↓</kbd> / <kbd>↑</kbd> | Move cursor down / up |
| <kbd>g</kbd> / <kbd>G</kbd> or <kbd>Home</kbd> / <kbd>End</kbd> | Jump to top / bottom of list |
| <kbd>[</kbd> / <kbd>]</kbd> or Media Prev / Next | Previous / next track |
| <kbd>Space</kbd> or Media Play/Pause | Toggle play / pause (auto-starts if stopped) |
| <kbd>Enter</kbd> | Play selected song / confirm |
| <kbd>/</kbd> or <kbd>Ctrl</kbd>+<kbd>F</kbd> | Focus search input |
| <kbd>←</kbd> / <kbd>→</kbd> | Seek backward / forward 5 seconds |
| <kbd>+</kbd> / <kbd>-</kbd> or Media Volume Keys | Adjust volume |
| <kbd>Ctrl</kbd>+<kbd>U</kbd> / <kbd>Ctrl</kbd>+<kbd>D</kbd> | Page up / down |
| <kbd>v</kbd> | Toggle full-screen synced lyrics |
| <kbd>m</kbd> | Cycle playback mode (list / repeat / single / shuffle) |
| <kbd>c</kbd> | Add selected song to playlist |
| <kbd>f</kbd> | Toggle favorite heart |
| <kbd>d</kbd> / <kbd>D</kbd> | Download track / Open download tasks |
| <kbd>a</kbd> / <kbd>x</kbd> | Append to queue / Open track actions menu |
| <kbd>Shift</kbd>+<kbd>B</kbd> / <kbd>Ctrl</kbd>+<kbd>B</kbd> | **Boss Mode**: Mute & switch to cargo test camouflage (press <kbd>B</kbd>/<kbd>Esc</kbd> to restore) |
| <kbd>Tab</kbd> / <kbd>b</kbd> | Switch pane focus / Open library menu |
| <kbd>?</kbd> | Open help dialog |
| <kbd>Esc</kbd> | Close popup / Cancel search / Exit full-screen lyrics |
| <kbd>q</kbd> | Exit TUI (background daemon continues playback) |

---

## Advanced Configuration

<details>
<summary><b>Persistent Settings (wocli config)</b></summary>

```bash
wocli config quality best        # Default audio quality: best / 320k / 192k / 128k
wocli config mode repeat         # Default mode: list / repeat / single / shuffle
wocli config volume 70           # Initial startup volume (0 - 100)
wocli config lyric_offset 150    # Lyric timing offset in ms (+ for early, - for late)
wocli config direct true         # Ignore proxy and connect directly (true / false)
```
</details>

<details>
<summary><b>Environment Variables</b></summary>

| Variable | Default | Purpose |
| :--- | :--- | :--- |
| `WOCLI_HOME` | `~/.config/wocli-music` *(Win: `%APPDATA%\wocli-music`)* | Main directory for SQLite database, settings, and logs. |
| `WOCLI_RUNTIME_DIR` | `/tmp/wocli-rust-{uid}-{hash}` *(Win: `%LOCALAPPDATA%\wocli-music\run`)* | Runtime IPC socket directory. |
| `WOCLI_DOWNLOAD_DIR` | `~/Music/wocli` *(TUI)* / `downloads` *(CLI)* | Target directory for song and lyric downloads. |
| `WOCLI_AUDIO_OUTPUT` | *(System default audio device)* | Set to `null` for headless/test environments without audio hardware. |
| `HTTP_PROXY` / `HTTPS_PROXY` | *(Unset)* | Proxy address for outbound HTTP/HTTPS traffic. |
</details>

<details>
<summary><b>Diagnostics (wocli doctor)</b></summary>

Troubleshoot your environment and audio output:
```bash
wocli doctor
```
Outputs system architecture, audio engine, active paths, and network proxy status in JSON format.
</details>

<details>
<summary><b>Manual Offline Download</b></summary>

If the install script cannot reach the network, download release archives directly from [Cloudflare Fast Mirror](https://wocli.yhan.fun/) or [GitHub Releases](https://github.com/yhan-sun/wocli-music/releases/latest):
- **Apple Silicon Mac**: `wocli-music-*-macos-arm64.tar.gz`
- **Intel Mac**: `wocli-music-*-macos-x86_64.tar.gz`
- **Linux x86_64**: `wocli-music-*-linux-x86_64.tar.gz` (includes `lib/libasound.so.2`)
- **Windows x86_64**: `wocli-music-*-windows-x86_64.zip`
</details>

<details>
<summary><b>Building from Source</b></summary>

Requires Rust **1.93.1+** and a C compiler:
```bash
git clone https://github.com/yhan-sun/wocli-music.git
cd wocli-music
cargo run --release            # Compile and run
cargo test --all-targets       # Run test suite
cargo clippy --all-targets     # Linting check
```
</details>

---

## Frequently Asked Questions (FAQ)

**Q: How do I stop music playback after closing the terminal?**  
A: Run `wocli stop` to stop playback, or `wocli daemon stop` to kill the background daemon.

**Q: Why do some search results fail to play?**  
A: Search indexes and upstream streaming authorizations are sometimes out of sync. `wocli-music` automatically falls back across lower bitrates and backup sources. If every source fails, it safely skips to the next track without freezing the queue.

**Q: Command not found after Windows installation?**  
A: The installer updates your user PATH. If you had existing command prompt or PowerShell windows open, completely restart your terminal app to inherit the updated PATH.

**Q: Can I run this in Docker or a remote server?**  
A: Yes. On headless Linux servers without sound hardware, launch with `WOCLI_AUDIO_OUTPUT=null` to run tests and decode audio silently.

---

## Credits & License

- Terminal interaction is inspired by [MiaowCham/ncm-cli](https://github.com/MiaowCham/ncm-cli).
- Built on top of [Ratatui](https://github.com/ratatui/ratatui), [Rodio](https://github.com/RustAudio/rodio), [Symphonia](https://github.com/pdeljanov/Symphonia), [QuickJS](https://bellard.org/quickjs/), and the official [Rust MCP SDK](https://github.com/modelcontextprotocol/rust-sdk).
- Licensed under the [GPL-3.0 License](LICENSE).
