# wocli-music

A native terminal music player built with Rust. Search, stream, view synchronized lyrics, and manage playlists. Lightweight, single-binary, keyboard-first, and keeps playing in the background even after you exit the TUI.

[中文文档 (Chinese)](README.md) · [Latest Release](https://github.com/yhan-sun/wocli-music/releases/latest) · [Installation](#installation) · [Keybindings](#keybindings) · [CLI Usage](#cli-usage) · [Configuration](#environment-variables--configuration) · [Changelog](CHANGELOG.md)

![wocli-music TUI: Library, synced lyrics, and playback controls](docs/screenshots/library.png)

---

## Highlights

- **Instant Progressive Streaming**: Powered by Symphonia & Rodio, probing and playback starts in under **200ms** after buffering only the initial 128KB, while the remainder buffers smoothly in the background.
- **Keyboard-First & Vim Navigation**: Ergonomic Vim keybindings (`j`/`k`, `g`/`G`, `[`/`]`, `Ctrl+U`/`Ctrl+D`, `/`).
- **Resilient Background Daemon**: Decoupled client-server architecture with bounded worker pools and IPC connection limiting. Exit the TUI or terminal window anytime without interrupting playback.
- **Synced Lyrics & System Integration**: High-precision LRC parsing, native macOS menu bar status with SF Symbols, Windows notification area tray, and desktop transparent floating lyrics overlay.
- **Unified Library Management**: Search public online streams or play local audio files (MP3, FLAC, AAC, M4A, WAV); manage Favorites, custom playlists, queue, and playback history.
- **CLI Automation**: Every player operation can be controlled via CLI commands with `--json` output, easily integrated into shell scripts, shortcuts, and tmux status bars.
- **Native Single Binary**: Zero Python, Node.js, mpv, or FFmpeg runtime dependencies. QuickJS is embedded for compatible LX sources.

---

## Installation

No prerequisites needed (no Rust, Python, uv, mpv, or FFmpeg required).

### One-Line Install Script (Recommended)

#### macOS / Linux (Terminal)

```bash
curl -fsSL https://raw.githubusercontent.com/yhan-sun/wocli-music/main/install.sh | bash
```

- **Custom install directory** (e.g., `/usr/local/bin`):
  ```bash
  curl -fsSL https://raw.githubusercontent.com/yhan-sun/wocli-music/main/install.sh | bash -s -- --dir /usr/local/bin
  ```
- **Uninstall**:
  ```bash
  curl -fsSL https://raw.githubusercontent.com/yhan-sun/wocli-music/main/install.sh | bash -s -- --uninstall
  ```

#### Windows (PowerShell)

```powershell
irm https://raw.githubusercontent.com/yhan-sun/wocli-music/main/install.ps1 | iex
```

The installer updates both the user PATH and the current PowerShell session. Other open terminals must fully restart their terminal application. To refresh a stale session from an older default installation:

```powershell
$env:Path += ";$env:LOCALAPPDATA\Programs\wocli-music"
wocli --version
```

For a custom installation, use its actual directory instead.

- **Custom install directory**:
  ```powershell
  & ([scriptblock]::Create((irm https://raw.githubusercontent.com/yhan-sun/wocli-music/main/install.ps1))) -InstallDir "C:\Tools\wocli"
  ```
- **Uninstall**:
  ```powershell
  & ([scriptblock]::Create((irm https://raw.githubusercontent.com/yhan-sun/wocli-music/main/install.ps1))) -Uninstall
  ```

---

To download in the TUI, select a song and press `d`, or open the download tasks panel and choose the new-download button. Submission errors stay visible in the form. Retry/cancel buttons keep the task list open; keyboard shortcuts are `r` and `c`. Local files show their existing location instead of creating a redundant download.

## Updating

Starting with v0.3.1:

```bash
wocli update --check   # Check the latest stable release
wocli update           # Verify SHA256 and update the current installation
wocli update --force   # Reinstall the current stable release
```

Also supports `--json` and `--direct`. The installation directory must be writable. Library data, favorites, queues and settings are preserved. Running daemon/tray processes are restarted; playback must be started again. Linux launcher wrappers are preserved while the actual binary and bundled ALSA library are updated. Versions v0.3.0 and earlier need one upgrade using the installation commands above before `wocli update` becomes available.

## Keybindings

| Key | Description |
| :--- | :--- |
| `j` / `k` or `↓` / `↑` | Move down / up in playlist or menu |
| `g` / `G` or `Home` / `End` | Jump to top / bottom of list |
| `[` / `]` or `<` / `>` | Previous / Next track |
| `Space` | Toggle Play / Pause |
| `Enter` | Play selected track / Confirm |
| `/` or `Ctrl+F` | Focus search input |
| `←` / `→` | Seek backward / forward 5 seconds |
| `+` / `-` | Volume up / down |
| `Ctrl+U` / `Ctrl+D` | Page up / Page down |
| `v` | Expand / collapse full-screen lyrics |
| `m` | Cycle play mode (List, Loop, Single, Shuffle) |
| `c` | Add selected track to a playlist |
| `f` | Add to / remove from Favorites ("Liked") |
| `d` | Download selected track |
| `a` | Append track to queue |
| `x` | Open track context menu |
| `Tab` | Switch focus between sidebar and track list |
| `b` | Open library navigation menu |
| `?` | Toggle keybindings help dialog |
| `Esc` | Close popup / Exit lyrics view / Cancel search |
| `q` | Exit TUI (playback continues in background) |

---

## CLI Usage

Use `wocli help`, `wocli thelp download`, or `wocli download --help` for topic help. `wocli help keys` lists terminal shortcuts.

Sources default to direct Kuwo endpoints in China (native and mobile fallback). Playback and downloads try the next enabled source after a failure. Manage them with `wocli source list`, `source prefer ID`, `source enable ID`, and `source disable ID`. Import an LX script with `wocli source add ./source.js`, then test it with `wocli source test ID --id 228908 --quality 128k`. `wocli retry` restarts the current track's source chain.

LX scripts run in embedded QuickJS with resource limits; no Node.js installation is required. Current compatibility covers Kuwo `kw/musicUrl`, HTTP, promises, timers, basic buffers, MD5 and zlib. AES/RSA, multipart form data, Node modules and other platforms' song IDs are not supported. Scripts are stored locally; playback does not fetch scripts from GitHub. See [source configuration](docs/sources.md) for HTTP JSON templates and details.

Control the player daemon directly from any terminal or script:

```bash
# Search and playback
wocli search '心做し majiko'
wocli play '心做し majiko' --pick 1
wocli play --id 61631119 --quality 320k
wocli play ~/Music/song.flac

# Playback controls
wocli pause
wocli resume
wocli toggle               # or wocli play-pause
wocli next                 # Next track (alias: wocli skip, optional: --no-wait)
wocli prev                 # Previous track (alias: wocli previous)
wocli seek +15             # Seek forward 15 seconds
wocli volume 65            # Set volume to 65%
wocli mode shuffle         # list / repeat / single / shuffle
wocli status --json        # Query current player status in JSON
wocli stop                 # Stop playback
```

### System Tray & Floating Lyrics Bar

```bash
# Tray service (macOS Menu Bar / Windows System Tray)
wocli tray start
wocli tray status
wocli tray stop

# Desktop transparent floating lyrics overlay
wocli lyrics-bar on
wocli lyrics-bar off
wocli lyrics-bar toggle
```

### Playlists & Daemon Management

```bash
wocli like                              # Like / unlike current track
wocli favorites                         # Show favorite songs
wocli playlist create 'Workout'         # Create a new playlist
wocli playlist add 'Workout' --id 61631119
wocli playlist play 'Workout'
wocli queue add --id 61631119 --next    # Play next
wocli download --id 61631119 --quality 320k --lyrics --output ~/Music

wocli daemon status                     # Check daemon status
wocli daemon stop                       # Stop daemon process
```

---

## Environment Variables & Configuration

wocli-music supports both persistent settings and runtime environment variable overrides.

### Environment Variables

| Variable | Default | Purpose |
| :--- | :--- | :--- |
| `WOCLI_HOME` | `~/.config/wocli-music` (Linux/macOS)<br>`%APPDATA%\wocli-music` (Windows) | Data & configuration root. Stores SQLite database (`library.sqlite3`), preferences (`settings.json`), daemon log (`service-rust.log`), and cached lyrics. |
| `WOCLI_RUNTIME_DIR` | `/tmp/wocli-rust-{uid}-{hash}` (Linux/macOS)<br>`%LOCALAPPDATA%\wocli-music\run` (Windows) | Runtime IPC socket/endpoint directory. |
| `WOCLI_DOWNLOAD_DIR` | `~/Music/wocli` (TUI) / `downloads` (CLI) | Default download directory for tracks and lyrics. |
| `WOCLI_AUDIO_OUTPUT` | *(System default audio device)* | Set to `null` to disable physical audio hardware output (decodes without sound, useful for CI or headless servers). |
| `HTTP_PROXY` / `HTTPS_PROXY` | *(None)* | Proxy URL for network stream requests. |

### Diagnostic Command

Run `wocli doctor` to inspect the runtime environment, active configuration, and daemon status:

```bash
wocli doctor
```

---

## Building from Source

Requires Rust **1.93.1+** and a standard C compiler (Xcode Command Line Tools on macOS, MSVC on Windows, `libasound2-dev` and `pkg-config` on Linux).

```bash
git clone https://github.com/yhan-sun/wocli-music.git
cd wocli-music
cargo build --release --locked
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
```

Binary will be produced at `target/release/wocli`.

---

## License

Licensed under [MIT](LICENSE). Third-party license notices are documented in `THIRD_PARTY.txt`.
