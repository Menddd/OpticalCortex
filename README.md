# OpticalCortex

A fast, lightweight desktop app for downloading videos and converting media files - powered by [yt-dlp](https://github.com/yt-dlp/yt-dlp) and [ffmpeg](https://ffmpeg.org/).

## Features

- **Download** videos from YouTube, Instagram, Facebook, X (Twitter), TikTok, and hundreds of other sites
- **M3U8 / HLS streams** via ffmpeg
- **My Playlists** - browse and download your YouTube playlists (requires authentication)
- **In-app authentication** - sign in to YouTube, Instagram, and Facebook directly within the app (Windows/macOS); cookies are captured securely and stored locally. On Linux, use the browser cookie fallback
- **File conversion** - convert between MP4, MKV, WebM, MOV, AVI, GIF, MP3, M4A, AAC, FLAC, WAV, Opus
- **Quality selection** - from 360p up to best available, or audio-only extraction
- Real-time download progress and log output

## Prerequisites

Both tools must be installed and available on your `PATH`:

| Tool | Install |
|---|---|
| **yt-dlp** | [yt-dlp releases](https://github.com/yt-dlp/yt-dlp/releases) - `winget install yt-dlp.yt-dlp` |
| **ffmpeg** | [ffmpeg.org](https://ffmpeg.org/download.html) - `winget install Gyan.FFmpeg` |

On Linux: `sudo apt install yt-dlp ffmpeg` (or distro equivalent)  
On macOS: `brew install yt-dlp ffmpeg`

## Installation

Download the latest release for your platform from the [Releases](../../releases) page:

| Platform | File |
|---|---|
| Windows | `.msi` installer or `.exe` setup |
| macOS (Apple Silicon) | `_aarch64.dmg` |
| macOS (Intel) | `_x64.dmg` |
| Linux | `.AppImage` or `.deb` |

## Building from Source

**Requirements:** [Rust](https://rustup.rs/) (stable), [Node.js](https://nodejs.org/) 18+ (required for npm dependency resolution), platform build tools

```bash
git clone https://github.com/Menddd/OpticalCortex.git
cd OpticalCortex
npm install
npm run tauri build
```

For development with hot-reload:

```bash
npm run tauri dev
```

### Linux additional dependencies

```bash
sudo apt update
sudo apt install libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf
```

### macOS

> **Note:** macOS builds are untested. Use at your own risk.

Xcode Command Line Tools required:

```bash
xcode-select --install
```

## Authentication

OpticalCortex can access private videos, members-only content, and your personal playlists using saved cookies. Cookies are stored locally and never transmitted. They are only passed to the local `yt-dlp` process.

### Windows / macOS

Use **Sign in with YouTube / Instagram / Facebook** in the Authentication tab. An in-app browser window opens, you log in normally, and cookies are captured automatically.

### Linux

Google blocks in-app browser logins on Linux. Use the **browser fallback** instead:

1. Log into YouTube (or Instagram/Facebook) in your regular browser
2. In the Authentication tab, expand **"Or use an installed browser's cookies"**
3. Select your browser from the dropdown and click **Test**

> **Note:** Chrome, Edge, and Brave use App-Bound Encryption which prevents direct cookie access. Use Firefox, or export cookies manually with the **Get cookies.txt LOCALLY** extension and load the file via the cookies.txt option.

Cookies are stored locally at `~/.config/OpticalCortex/`.

## Contributing

Contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

[MIT](LICENSE)
