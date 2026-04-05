# Contributing to OpticalCortex

## Getting Started

1. Fork the repository and clone your fork
2. Install prerequisites: [Rust](https://rustup.rs/), [Node.js](https://nodejs.org/) 18+
3. Install Linux deps if applicable (see README)
4. Run `npm install` then `npm run tauri dev`

## Project Structure

```
src/                   # Frontend - vanilla HTML/CSS/JS, no bundler
  index.html
  main.js
  styles.css
src-tauri/
  src/
    lib.rs             # Tauri setup, window pre-creation
    downloader.rs      # All Tauri commands and backend logic
  capabilities/        # Tauri IPC permission scopes
  tauri.conf.json
```

## Architecture Notes

- **No frontend framework or bundler** - the `src/` directory is served directly by Tauri
- **Tauri commands** use snake_case struct fields (serde) for the `DownloadRequest` / `ConvertRequest` types. Individual command parameters are auto-converted from camelCase in JS
- **Login windows** must be pre-created in `setup()` before the event loop starts - creating them from a Tauri command causes a WRY/WebView2 deadlock on Windows
- **HttpOnly cookies** (required for YouTube auth) are extracted via the Chrome DevTools Protocol `Network.getAllCookies` method, not `document.cookie`

## Pull Requests

- Keep changes focused - one feature or fix per PR
- Test on your platform before submitting
- Update `README.md` if adding user-facing functionality

## Reporting Issues

Please include your OS, app version, and the relevant log output from the in-app log viewer.
