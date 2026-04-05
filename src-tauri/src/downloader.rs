use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_shell::ShellExt;
use url::Url;

// ── Cookie storage directory ──────────────────────────────────────────────────
// Stored at a clean OS-appropriate path independent of the bundle identifier.

fn cookies_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    if let Ok(appdata) = std::env::var("APPDATA") {
        return PathBuf::from(appdata).join("OpticalCortex");
    }
    #[cfg(target_os = "macos")]
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home)
            .join("Library").join("Application Support").join("OpticalCortex");
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
            return PathBuf::from(xdg).join("OpticalCortex");
        }
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(".config").join("OpticalCortex");
        }
    }
    PathBuf::from(".")
}

// ── Playlist / video metadata types ───────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistInfo {
    pub id: String,
    pub title: String,
    pub thumbnail: String,
    pub video_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoInfo {
    pub id: String,
    pub title: String,
    pub thumbnail: String,
    pub duration_secs: u64,
    pub url: String,
}

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadRequest {
    pub url: String,
    pub output_dir: String,
    pub quality: String,       // e.g. "bestvideo+bestaudio/best" or "audio_mp3"
    pub filename: String,      // user-provided stem (empty → auto)
    pub subtitles: bool,
    pub embed_thumbnail: bool,
    pub no_playlist: bool,
    // auth
    pub auth_mode: String,     // "none" | "browser" | "cookies_file"
    pub auth_browser: String,  // "chrome" | "firefox" | "edge" | "brave" | "opera" | "chromium"
    pub cookies_file: String,  // path to Netscape cookies.txt (auth_mode == "cookies_file")
}

#[derive(Debug, Clone, Serialize)]
pub struct ProgressEvent {
    pub id: String,
    pub kind: String,          // "progress" | "log" | "done" | "error"
    pub message: String,
    pub percent: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolStatus {
    pub yt_dlp: bool,
    pub ffmpeg: bool,
    pub yt_dlp_version: Option<String>,
    pub yt_dlp_outdated: bool,
    pub has_js_runtime: bool,   // node, nodejs, or deno — needed for n-challenge with cookies
}

#[derive(Debug, Serialize)]
pub struct AuthState {
    pub yt_cookies: Option<String>,
    pub ig_cookies: Option<String>,
    pub fb_cookies: Option<String>,
}

/// Return the paths of any previously saved cookie files so the frontend
/// can restore auth state without requiring the user to log in again.
#[tauri::command]
pub fn load_auth_state() -> AuthState {
    let dir = cookies_dir();
    fn existing(path: PathBuf) -> Option<String> {
        if path.exists() { Some(path.to_string_lossy().into_owned()) } else { None }
    }
    AuthState {
        yt_cookies: existing(dir.join("youtube_cookies.txt")),
        ig_cookies: existing(dir.join("instagram_cookies.txt")),
        fb_cookies: existing(dir.join("facebook_cookies.txt")),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn emit(app: &AppHandle, id: &str, kind: &str, message: &str, percent: f64) {
    let _ = app.emit(
        "dl-progress",
        ProgressEvent {
            id: id.to_string(),
            kind: kind.to_string(),
            message: message.to_string(),
            percent,
        },
    );
}

/// Return true if a command is on PATH (Windows: try `where`, Unix: `which`).
fn command_exists(cmd: &str) -> bool {
    let check = if cfg!(windows) { "where" } else { "which" };
    std::process::Command::new(check)
        .arg(cmd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Detect whether a URL looks like an M3U8 stream.
#[tauri::command]
pub fn detect_type(url: String) -> String {
    let u = url.to_lowercase();
    if u.contains(".m3u8") || u.contains("m3u8") {
        "m3u8".to_string()
    } else {
        "ytdlp".to_string()
    }
}

/// Check whether yt-dlp and ffmpeg are available on PATH, and read the yt-dlp version.
#[tauri::command]
pub fn check_tools() -> ToolStatus {
    let yt_dlp = command_exists("yt-dlp");
    let (yt_dlp_version, yt_dlp_outdated) = if yt_dlp {
        match std::process::Command::new("yt-dlp").arg("--version").output() {
            Ok(out) => {
                let ver = String::from_utf8_lossy(&out.stdout).trim().to_string();
                // Version is YYYY.MM.DD — outdated if year < 2024
                let outdated = ver.split('.').next()
                    .and_then(|y| y.parse::<u32>().ok())
                    .map(|y| y < 2024)
                    .unwrap_or(false);
                (Some(ver), outdated)
            }
            Err(_) => (None, false),
        }
    } else {
        (None, false)
    };
    let has_js_runtime = command_exists("node") || command_exists("nodejs") || command_exists("deno");
    ToolStatus { yt_dlp, ffmpeg: command_exists("ffmpeg"), yt_dlp_version, yt_dlp_outdated, has_js_runtime }
}

/// Test whether the browser cookies give valid YouTube access.
///
/// Strategy: fetch the "Liked Videos" playlist (list=LL) which always exists
/// for any logged-in account. We use --flat-playlist --playlist-items 1 so it
/// only fetches one item's metadata - fast and low-impact.
/// We also tolerate a non-zero exit code and inspect stderr ourselves so we can
/// distinguish "auth error" from "empty list / extractor quirk".
#[tauri::command]
pub async fn verify_auth(
    app: AppHandle,
    auth_mode: String,
    auth_browser: String,
    cookies_file: String,
) -> Result<String, String> {
    // ── attempt 1: Liked Videos playlist (requires login, always exists) ─────
    let result = try_auth_url(
        &app,
        "https://www.youtube.com/playlist?list=LL",
        &auth_mode,
        &auth_browser,
        &cookies_file,
    ).await;

    match result {
        AuthResult::Ok(msg)   => return Ok(msg),
        AuthResult::AuthError(e) => return Err(e),
        AuthResult::OtherError(_) => {} // fall through to attempt 2
    }

    // ── attempt 2: Watch Later (also always exists) ───────────────────────────
    let result2 = try_auth_url(
        &app,
        "https://www.youtube.com/playlist?list=WL",
        &auth_mode,
        &auth_browser,
        &cookies_file,
    ).await;

    match result2 {
        AuthResult::Ok(msg)      => Ok(msg),
        AuthResult::AuthError(e) => Err(e),
        AuthResult::OtherError(e) => {
            // Both failed but no clear auth error - cookies may be fine,
            // the playlists may just be empty or region-restricted.
            // Give the user the benefit of the doubt and show a warning.
            Err(format!(
                "Could not verify - cookies were read from {auth_browser} but YouTube returned an error.\n\n\
                 This can happen if:\n\
                 • The browser ({auth_browser}) is open - try closing it and testing again\n\
                 • You are not logged into YouTube in {auth_browser}\n\
                 • Your account has no Liked Videos or Watch Later items\n\n\
                 Raw error: {e}"
            ))
        }
    }
}

enum AuthResult {
    Ok(String),
    AuthError(String),
    OtherError(String),
}

async fn try_auth_url(
    app: &AppHandle,
    url: &str,
    auth_mode: &str,
    auth_browser: &str,
    cookies_file: &str,
) -> AuthResult {
    let mut args: Vec<String> = vec![
        "--flat-playlist".into(),
        "--dump-single-json".into(),
        "--playlist-items".into(), "1".into(),
        "--no-warnings".into(),
        "--ignore-errors".into(),
    ];
    append_auth_args(&mut args, auth_mode, auth_browser, cookies_file);
    args.push(url.into());

    let (stdout, stderr, code) = run_capture_full(app, "yt-dlp", &args).await
        .unwrap_or_else(|e| (String::new(), e, -1));

    let combined = format!("{stdout}{stderr}").to_lowercase();

    // ── DPAPI / App-Bound Encryption (Chrome 127+, Brave, Edge) ─────────────
    // This is a Windows OS-level encryption issue, not an auth failure.
    // Prefix with "DPAPI:" so the frontend can show targeted guidance.
    let is_dpapi = combined.contains("failed to decrypt")
        || combined.contains("dpapi")
        || combined.contains("app-bound")
        || combined.contains("ciphertext");
    if is_dpapi {
        return AuthResult::AuthError(format!(
            "DPAPI:{auth_browser}"
        ));
    }

    // ── Definitive YouTube auth failures ─────────────────────────────────────
    let auth_errors = [
        "sign in to confirm",
        "login required",
        "this playlist is private",
        "not authenticated",
        "could not get cookies",
    ];
    let is_auth_error = auth_errors.iter().any(|kw| combined.contains(kw));

    if is_auth_error {
        let detail = stderr.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
        return AuthResult::AuthError(format!(
            "YouTube rejected the cookies from {auth_browser}. \
             Make sure you are logged into YouTube in {auth_browser}.\n\nDetail: {detail}"
        ));
    }

    if code == 0 || stdout.contains("\"id\"") || stdout.contains("\"entries\"") {
        let hint = if code != 0 {
            " (playlist may be empty, but cookies loaded successfully)"
        } else {
            ""
        };
        return AuthResult::Ok(format!(
            "✓ Auth OK - {auth_browser} cookies loaded successfully{hint}"
        ));
    }

    // Hint about browser being open (SQLite lock)
    let lock_hint = if combined.contains("database") || combined.contains("sqlite")
        || combined.contains("lock") || combined.contains("unable to open")
    {
        format!("\n\nHint: Close {auth_browser} completely and try again - the cookie database may be locked.")
    } else {
        String::new()
    };

    let first_err = stderr.lines().find(|l| !l.trim().is_empty()).unwrap_or("unknown error").trim().to_string();
    AuthResult::OtherError(format!("{first_err}{lock_hint}"))
}

/// Like run_capture but always returns (stdout, stderr, exit_code) - never Err.
async fn run_capture_full(
    app: &AppHandle,
    program: &str,
    args: &[String],
) -> Result<(String, String, i32), String> {
    use tauri_plugin_shell::process::CommandEvent;

    let mut cmd = app.shell().command(program);
    for a in args {
        cmd = cmd.arg(a);
    }

    let (mut rx, _child) = cmd.spawn().map_err(|e| {
        format!("Failed to spawn `{program}`: {e}")
    })?;

    let mut stdout_buf: Vec<u8> = Vec::new();
    let mut stderr_buf: Vec<u8> = Vec::new();
    let mut code = -1i32;

    while let Some(event) = rx.recv().await {
        match event {
            CommandEvent::Stdout(b) => stdout_buf.extend_from_slice(&b),
            CommandEvent::Stderr(b) => stderr_buf.extend_from_slice(&b),
            CommandEvent::Error(e)  => return Err(format!("Process error: {e}")),
            CommandEvent::Terminated(s) => {
                code = s.code.unwrap_or(-1);
                break;
            }
            _ => {}
        }
    }

    let stdout = String::from_utf8_lossy(&stdout_buf).into_owned();
    let stderr = String::from_utf8_lossy(&stderr_buf).into_owned();
    Ok((stdout, stderr, code))
}

// ── In-app YouTube login ──────────────────────────────────────────────────────

/// Open a dedicated WebView window for YouTube login.
/// An injected script watches for the SAPISID cookie (proof of login) on
/// youtube.com, then calls `save_yt_cookies` via Tauri IPC to capture them.
///
/// IMPORTANT: WebviewWindowBuilder::build() must run on the main thread.
/// Tauri commands run on a tokio worker thread, so we use run_on_main_thread
/// and a channel to dispatch the build and return any error.
#[tauri::command]
pub fn open_youtube_login(app: AppHandle) -> Result<(), String> {
    // The window was pre-created hidden in setup() to avoid a WRY/WebView2
    // deadlock. Here we just navigate it to Google sign-in and show it.
    let win = app
        .get_webview_window("youtube-login")
        .ok_or_else(|| "Login window not initialised".to_string())?;

    let login_url = Url::parse(
        "https://accounts.google.com/AccountChooser\
         ?continue=https%3A%2F%2Fwww.youtube.com%2F",
    )
    .expect("hardcoded URL is valid");

    win.navigate(login_url).map_err(|e| format!("Navigate failed: {e}"))?;
    win.show().map_err(|e| format!("Show failed: {e}"))?;
    win.set_focus().map_err(|e| format!("Focus failed: {e}"))?;
    //win.open_devtools();
    Ok(())
}

/// Called by the injected login script when the user is detected as logged in.
/// Formats the cookies as a Netscape cookies.txt file and saves it to the
/// app data directory, then emits `yt-cookies-saved` to the main window.
#[tauri::command]
pub fn save_yt_cookies(
    app: AppHandle,
    cookie_str: String,
    domain: String,
) -> Result<String, String> {
    // Hide the login window (don't close - it's reused)
    if let Some(w) = app.get_webview_window("youtube-login") {
        let _ = w.hide();
    }

    if cookie_str.trim().is_empty() {
        return Err("No cookies received".into());
    }

    let netscape = build_netscape_cookies(&cookie_str, &domain);

    let data_dir = cookies_dir();
    std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
    let path = data_dir.join("youtube_cookies.txt");
    std::fs::write(&path, &netscape).map_err(|e| e.to_string())?;

    let path_str = path.to_string_lossy().to_string();

    // Notify the main window
    let _ = app.emit("yt-cookies-saved", path_str.clone());

    Ok(path_str)
}

/// Extract ALL cookies (including HttpOnly) from the YouTube login WebView2
/// using Chrome DevTools Protocol `Network.getAllCookies`. Called by the
/// initialization_script when the user lands on www.youtube.com after sign-in.
/// The CDP callback is async and fires on the main thread; this command returns
/// immediately and the `yt-cookies-saved` event is emitted when done.
#[tauri::command]
pub fn extract_yt_cookies(app: AppHandle) -> Result<(), String> {
    let win = app
        .get_webview_window("youtube-login")
        .ok_or_else(|| "Login window not found".to_string())?;

    #[cfg(target_os = "windows")]
    {
        let app2 = app.clone();
        // with_webview dispatches to the main thread; our closure is fast
        // (just starts the async CDP call) so no deadlock risk.
        // We use webview2_com's built-in CallDevToolsProtocolMethodCompletedHandler::create
        // which wraps the closure - no #[implement] needed (avoids windows_core version issues).
        win.with_webview(move |wv| {
            use webview2_com::CallDevToolsProtocolMethodCompletedHandler;
            use windows::core::w;

            unsafe {
                let controller = wv.controller();
                match controller.CoreWebView2() {
                    Ok(wv2) => {
                        // Closure receives (Result<()>, String) after HRESULT/PCWSTR conversion
                        let handler = CallDevToolsProtocolMethodCompletedHandler::create(
                            Box::new(move |result: windows::core::Result<()>, json: String| {
                                if let Err(e) = &result {
                                    eprintln!("[CDP] invoke error: {e}");
                                    return Ok(());
                                }
                                eprintln!("[CDP] got {} chars of cookie JSON", json.len());
                                match save_cdp_cookies_as(&json, &app2, "youtube_cookies.txt") {
                                    Ok(path) => {
                                        eprintln!("[CDP] saved to {path}");
                                        let app3 = app2.clone();
                                        tauri::async_runtime::spawn(async move {
                                            if let Some(w) = app3.get_webview_window("youtube-login") {
                                                let _ = w.hide();
                                            }
                                            let _ = app3.emit("yt-cookies-saved", path);
                                        });
                                    }
                                    Err(e) => eprintln!("[CDP] save error: {e}"),
                                }
                                Ok(())
                            }),
                        );
                        if let Err(e) = wv2.CallDevToolsProtocolMethod(
                            w!("Network.getAllCookies"),
                            w!("{}"),
                            &handler,
                        ) {
                            eprintln!("[CDP] CallDevToolsProtocolMethod: {e}");
                        }
                    }
                    Err(e) => eprintln!("[CDP] CoreWebView2: {e}"),
                }
            }
        })
        .map_err(|e| format!("with_webview: {e}"))?;
    }

    #[cfg(target_os = "macos")]
    do_macos_extract(&win, app, "youtube_cookies.txt", "youtube-login", "yt-cookies-saved")?;

    #[cfg(target_os = "linux")]
    {
        const YT_DOMAINS: &[&str] = &[
            "https://www.youtube.com",
            "https://youtube.com",
            "https://accounts.google.com",
            "https://www.google.com",
        ];
        do_linux_extract(&win, app, YT_DOMAINS, "youtube_cookies.txt", "youtube-login", "yt-cookies-saved")?;
    }

    Ok(())
}

/// Parse CDP `Network.getAllCookies` JSON response and save as Netscape cookies.txt.
#[allow(dead_code)]
fn save_cdp_cookies_as(json: &str, _app: &AppHandle, filename: &str) -> Result<String, String> {
    let root: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| format!("JSON: {e}"))?;

    let cookies = root["cookies"].as_array()
        .ok_or("No cookies array in CDP response")?;

    let mut lines = vec![
        "# Netscape HTTP Cookie File".to_string(),
        "# Generated by Video Downloader - do not edit".to_string(),
        String::new(),
    ];

    for ck in cookies {
        let name      = ck["name"].as_str().unwrap_or("");
        let value     = ck["value"].as_str().unwrap_or("");
        let domain    = ck["domain"].as_str().unwrap_or("");
        let path      = ck["path"].as_str().unwrap_or("/");
        let secure    = if ck["secure"].as_bool().unwrap_or(false) { "TRUE" } else { "FALSE" };
        let expires   = ck["expires"].as_f64().map(|f| f as i64).unwrap_or(2_147_483_647);
        let subdomain = if domain.starts_with('.') { "TRUE" } else { "FALSE" };

        if name.is_empty() { continue; }
        lines.push(format!("{domain}\t{subdomain}\t{path}\t{secure}\t{expires}\t{name}\t{value}"));
    }

    let data_dir = cookies_dir();
    std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
    let path = data_dir.join(filename);
    std::fs::write(&path, lines.join("\n")).map_err(|e| e.to_string())?;

    Ok(path.to_string_lossy().into_owned())
}

// ── Native cookie extraction: macOS (WKHTTPCookieStore) + Linux (WebKitGTK) ──

#[cfg(any(target_os = "macos", target_os = "linux"))]
struct NativeCookie {
    name: String,
    value: String,
    domain: String,
    path: String,
    secure: bool,
    expires: i64,
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn save_native_cookies(cookies: &[NativeCookie], filename: &str) -> Result<String, String> {
    let mut lines = vec![
        "# Netscape HTTP Cookie File".to_string(),
        "# Generated by OpticalCortex".to_string(),
        String::new(),
    ];
    for ck in cookies {
        if ck.name.is_empty() { continue; }
        let subdomain = if ck.domain.starts_with('.') { "TRUE" } else { "FALSE" };
        let secure = if ck.secure { "TRUE" } else { "FALSE" };
        lines.push(format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}",
            ck.domain, subdomain, ck.path, secure, ck.expires, ck.name, ck.value
        ));
    }
    let data_dir = cookies_dir();
    std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
    let path = data_dir.join(filename);
    std::fs::write(&path, lines.join("\n")).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().into_owned())
}

/// macOS: call WKHTTPCookieStore.getAllCookies() via ObjC and emit the event
/// when the async callback fires. StackBlock is copied to the heap by WKHTTPCookieStore.
#[cfg(target_os = "macos")]
fn do_macos_extract(
    win: &tauri::WebviewWindow,
    app: AppHandle,
    filename: &'static str,
    window_label: &'static str,
    event_name: &'static str,
) -> Result<(), String> {
    win.with_webview(move |wv| unsafe {
        use block2::StackBlock;
        use objc2::msg_send;
        use objc2::runtime::AnyObject;
        use objc2_foundation::{NSArray, NSHTTPCookie};

        // wv.inner() returns a raw pointer to the WKWebView (subclass) instance
        let raw_wv: *mut AnyObject = wv.inner() as *mut AnyObject;

        let config:       *mut AnyObject = msg_send![raw_wv,   configuration];
        let data_store:   *mut AnyObject = msg_send![config,   websiteDataStore];
        let cookie_store: *mut AnyObject = msg_send![data_store, httpCookieStore];

        let app2 = app.clone();
        let block = StackBlock::new(move |cookies: &NSArray<NSHTTPCookie>| {
            let n = cookies.count();
            let mut out = Vec::with_capacity(n);
            for i in 0..n {
                let ck = cookies.objectAtIndex(i);
                let expires = ck.expiresDate()
                    .map(|d| d.timeIntervalSince1970() as i64)
                    .unwrap_or(2_147_483_647);
                out.push(NativeCookie {
                    name:   ck.name().to_string(),
                    value:  ck.value().to_string(),
                    domain: ck.domain().to_string(),
                    path:   ck.path().to_string(),
                    secure: ck.isSecure(),
                    expires,
                });
            }
            match save_native_cookies(&out, filename) {
                Ok(path) => {
                    let app3 = app2.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Some(w) = app3.get_webview_window(window_label) {
                            let _ = w.hide();
                        }
                        let _ = app3.emit(event_name, path);
                    });
                }
                Err(e) => eprintln!("[macos-cookies] {e}"),
            }
        });

        let _: () = msg_send![cookie_store, getAllCookies: &*block];
    })
    .map_err(|e| format!("with_webview: {e}"))
}

/// Linux: call CookieManager.cookies() for each domain and emit the event
/// after all per-domain callbacks have returned.
#[cfg(target_os = "linux")]
fn do_linux_extract(
    win: &tauri::WebviewWindow,
    app: AppHandle,
    domains: &'static [&'static str],
    filename: &'static str,
    window_label: &'static str,
    event_name: &'static str,
) -> Result<(), String> {
    use std::sync::{Arc, Mutex};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use webkit2gtk::glib;

    let collected = Arc::new(Mutex::new(Vec::<NativeCookie>::new()));
    let remaining = Arc::new(AtomicUsize::new(domains.len()));

    win.with_webview(move |wv| {
        use webkit2gtk::{WebViewExt, WebContextExt, CookieManagerExt};

        let webview = wv.inner();
        let ctx = webview.web_context().expect("no WebContext");
        let cookie_mgr = ctx.cookie_manager().expect("no CookieManager");

        for &uri in domains {
            let coll2 = collected.clone();
            let rem2  = remaining.clone();
            let app2  = app.clone();

            cookie_mgr.cookies(uri, None::<&gio::Cancellable>, move |result: Result<Vec<soup::Cookie>, glib::Error>| {
                if let Ok(cookies) = result {
                    let mut guard = coll2.lock().unwrap();
                    for mut ck in cookies {
                        let name = ck.name().map(|s: glib::GString| s.to_string()).unwrap_or_default();
                        if name.is_empty() { continue; }
                        let expires = ck.expires()
                            .map(|d: glib::DateTime| d.to_unix())
                            .unwrap_or(2_147_483_647);
                        guard.push(NativeCookie {
                            name,
                            value:  ck.value().map(|s: glib::GString| s.to_string()).unwrap_or_default(),
                            domain: ck.domain().map(|s: glib::GString| s.to_string()).unwrap_or_default(),
                            path:   ck.path().map(|s: glib::GString| s.to_string()).unwrap_or_default(),
                            secure: ck.is_secure(),
                            expires,
                        });
                    }
                }
                if rem2.fetch_sub(1, Ordering::SeqCst) == 1 {
                    let guard = coll2.lock().unwrap();
                    match save_native_cookies(&guard, filename) {
                        Ok(path) => {
                            let app3 = app2.clone();
                            tauri::async_runtime::spawn(async move {
                                if let Some(w) = app3.get_webview_window(window_label) {
                                    let _ = w.hide();
                                }
                                let _ = app3.emit(event_name, path);
                            });
                        }
                        Err(e) => eprintln!("[linux-cookies] {e}"),
                    }
                }
            });
        }
    })
    .map_err(|e| format!("with_webview: {e}"))
}

// ── In-app Instagram login ────────────────────────────────────────────────────

#[tauri::command]
pub fn open_instagram_login(app: AppHandle) -> Result<(), String> {
    let win = app
        .get_webview_window("instagram-login")
        .ok_or_else(|| "Login window not initialised".to_string())?;
    let login_url = Url::parse("https://www.instagram.com/accounts/login/")
        .expect("hardcoded URL is valid");
    win.navigate(login_url).map_err(|e| format!("Navigate failed: {e}"))?;
    win.show().map_err(|e| format!("Show failed: {e}"))?;
    win.set_focus().map_err(|e| format!("Focus failed: {e}"))?;
    Ok(())
}

/// Extract all cookies (including HttpOnly like `sessionid`) from the Instagram
/// login WebView via CDP, then save and emit `ig-cookies-saved`.
#[tauri::command]
pub fn extract_ig_cookies(app: AppHandle) -> Result<(), String> {
    let win = app
        .get_webview_window("instagram-login")
        .ok_or_else(|| "Login window not found".to_string())?;

    #[cfg(target_os = "windows")]
    {
        let app2 = app.clone();
        win.with_webview(move |wv| {
            use webview2_com::CallDevToolsProtocolMethodCompletedHandler;
            use windows::core::w;
            unsafe {
                let controller = wv.controller();
                match controller.CoreWebView2() {
                    Ok(wv2) => {
                        let handler = CallDevToolsProtocolMethodCompletedHandler::create(
                            Box::new(move |result: windows::core::Result<()>, json: String| {
                                if let Err(e) = &result {
                                    eprintln!("[CDP/ig] invoke error: {e}");
                                    return Ok(());
                                }
                                match save_cdp_cookies_as(&json, &app2, "instagram_cookies.txt") {
                                    Ok(path) => {
                                        let app3 = app2.clone();
                                        tauri::async_runtime::spawn(async move {
                                            if let Some(w) = app3.get_webview_window("instagram-login") {
                                                let _ = w.hide();
                                            }
                                            let _ = app3.emit("ig-cookies-saved", path);
                                        });
                                    }
                                    Err(e) => eprintln!("[CDP/ig] save error: {e}"),
                                }
                                Ok(())
                            }),
                        );
                        if let Err(e) = wv2.CallDevToolsProtocolMethod(
                            w!("Network.getAllCookies"),
                            w!("{}"),
                            &handler,
                        ) {
                            eprintln!("[CDP/ig] CallDevToolsProtocolMethod: {e}");
                        }
                    }
                    Err(e) => eprintln!("[CDP/ig] CoreWebView2: {e}"),
                }
            }
        })
        .map_err(|e| format!("with_webview: {e}"))?;
    }

    #[cfg(target_os = "macos")]
    do_macos_extract(&win, app, "instagram_cookies.txt", "instagram-login", "ig-cookies-saved")?;

    #[cfg(target_os = "linux")]
    {
        const IG_DOMAINS: &[&str] = &[
            "https://www.instagram.com",
            "https://instagram.com",
        ];
        do_linux_extract(&win, app, IG_DOMAINS, "instagram_cookies.txt", "instagram-login", "ig-cookies-saved")?;
    }

    Ok(())
}

// ── In-app Facebook login ─────────────────────────────────────────────────────

#[tauri::command]
pub fn open_facebook_login(app: AppHandle) -> Result<(), String> {
    let win = app
        .get_webview_window("facebook-login")
        .ok_or_else(|| "Login window not initialised".to_string())?;
    let login_url = Url::parse("https://www.facebook.com/login/")
        .expect("hardcoded URL is valid");
    win.navigate(login_url).map_err(|e| format!("Navigate failed: {e}"))?;
    win.show().map_err(|e| format!("Show failed: {e}"))?;
    win.set_focus().map_err(|e| format!("Focus failed: {e}"))?;
    Ok(())
}

/// Extract all cookies (including HttpOnly like `xs`) from the Facebook
/// login WebView via CDP, then save and emit `fb-cookies-saved`.
#[tauri::command]
pub fn extract_fb_cookies(app: AppHandle) -> Result<(), String> {
    let win = app
        .get_webview_window("facebook-login")
        .ok_or_else(|| "Login window not found".to_string())?;

    #[cfg(target_os = "windows")]
    {
        let app2 = app.clone();
        win.with_webview(move |wv| {
            use webview2_com::CallDevToolsProtocolMethodCompletedHandler;
            use windows::core::w;
            unsafe {
                let controller = wv.controller();
                match controller.CoreWebView2() {
                    Ok(wv2) => {
                        let handler = CallDevToolsProtocolMethodCompletedHandler::create(
                            Box::new(move |result: windows::core::Result<()>, json: String| {
                                if let Err(e) = &result {
                                    eprintln!("[CDP/fb] invoke error: {e}");
                                    return Ok(());
                                }
                                match save_cdp_cookies_as(&json, &app2, "facebook_cookies.txt") {
                                    Ok(path) => {
                                        let app3 = app2.clone();
                                        tauri::async_runtime::spawn(async move {
                                            if let Some(w) = app3.get_webview_window("facebook-login") {
                                                let _ = w.hide();
                                            }
                                            let _ = app3.emit("fb-cookies-saved", path);
                                        });
                                    }
                                    Err(e) => eprintln!("[CDP/fb] save error: {e}"),
                                }
                                Ok(())
                            }),
                        );
                        if let Err(e) = wv2.CallDevToolsProtocolMethod(
                            w!("Network.getAllCookies"),
                            w!("{}"),
                            &handler,
                        ) {
                            eprintln!("[CDP/fb] CallDevToolsProtocolMethod: {e}");
                        }
                    }
                    Err(e) => eprintln!("[CDP/fb] CoreWebView2: {e}"),
                }
            }
        })
        .map_err(|e| format!("with_webview: {e}"))?;
    }

    #[cfg(target_os = "macos")]
    do_macos_extract(&win, app, "facebook_cookies.txt", "facebook-login", "fb-cookies-saved")?;

    #[cfg(target_os = "linux")]
    {
        const FB_DOMAINS: &[&str] = &[
            "https://www.facebook.com",
            "https://facebook.com",
            "https://m.facebook.com",
        ];
        do_linux_extract(&win, app, FB_DOMAINS, "facebook_cookies.txt", "facebook-login", "fb-cookies-saved")?;
    }

    Ok(())
}

/// Format a `document.cookie` string as a Netscape HTTP Cookie File.
fn build_netscape_cookies(cookie_str: &str, hostname: &str) -> String {
    // Normalise hostname to domain with leading dot
    let base = hostname
        .trim_start_matches("www.")
        .trim_start_matches('.');
    let domain = format!(".{base}");

    let mut lines = vec![
        "# Netscape HTTP Cookie File".to_string(),
        "# Generated by Video Downloader - do not edit".to_string(),
        String::new(),
    ];

    for pair in cookie_str.split(';') {
        let pair = pair.trim();
        if pair.is_empty() { continue; }
        let (name, value) = match pair.find('=') {
            Some(pos) => (pair[..pos].trim(), pair[pos + 1..].trim()),
            None      => (pair, ""),
        };
        if name.is_empty() { continue; }
        // domain | subdomains | path | secure | expires | name | value
        lines.push(format!(
            "{domain}\tTRUE\t/\tTRUE\t2147483647\t{name}\t{value}"
        ));
    }

    lines.join("\n")
}

fn append_auth_args(args: &mut Vec<String>, mode: &str, browser: &str, cookies_file: &str) {
    match mode {
        "browser" => {
            if !browser.is_empty() {
                args.push("--cookies-from-browser".into());
                args.push(browser.to_string());
            }
        }
        "cookies_file" => {
            if !cookies_file.is_empty() {
                args.push("--cookies".into());
                args.push(cookies_file.to_string());
            }
        }
        _ => {} // "none"
    }
}

/// Open a folder in the system file explorer.
#[tauri::command]
pub fn open_folder(path: String) -> Result<(), String> {
    let p = PathBuf::from(&path);
    if !p.exists() {
        return Err(format!("Path does not exist: {path}"));
    }
    #[cfg(target_os = "windows")]
    std::process::Command::new("explorer")
        .arg(&path)
        .spawn()
        .map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    std::process::Command::new("open")
        .arg(&path)
        .spawn()
        .map_err(|e| e.to_string())?;
    #[cfg(target_os = "linux")]
    std::process::Command::new("xdg-open")
        .arg(&path)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ── File conversion ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct ConvertRequest {
    pub input_path:    String,
    pub output_dir:    String,
    pub output_format: String,  // mp4 | mkv | webm | mov | avi | gif | mp3 | m4a | aac | wav | flac | opus
    pub video_quality: String,  // high | medium | low  (video formats only)
}

#[tauri::command]
pub async fn convert_file(app: AppHandle, req: ConvertRequest) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let app2 = app.clone();
    let id2 = id.clone();
    tokio::spawn(async move {
        let result = run_ffmpeg_convert(&app2, &id2, &req).await;
        match result {
            Ok(_) => emit(&app2, &id2, "done", "Conversion complete!", 100.0),
            Err(e) => emit(&app2, &id2, "error", &e, 0.0),
        }
    });
    Ok(id)
}

async fn run_ffmpeg_convert(app: &AppHandle, id: &str, req: &ConvertRequest) -> Result<(), String> {
    let input = PathBuf::from(&req.input_path);
    let stem  = input.file_stem().and_then(|s| s.to_str()).unwrap_or("output");
    let out   = PathBuf::from(&req.output_dir).join(format!("{}.{}", stem, req.output_format));
    let out_str = out.to_string_lossy().into_owned();

    emit(app, id, "log", &format!("Output → {out_str}"), 0.0);

    let crf = match req.video_quality.as_str() { "high" => "18", "low" => "28", _ => "23" };
    let crf_vp9 = match req.video_quality.as_str() { "high" => "24", "low" => "38", _ => "31" };
    let abr = match req.video_quality.as_str() { "high" => "320k", "low" => "128k", _ => "192k" };

    let mut args: Vec<String> = vec!["-y".into(), "-i".into(), req.input_path.clone()];

    match req.output_format.as_str() {
        "mp4" | "mov" | "avi" => {
            args.extend([
                "-c:v".into(), "libx264".into(), "-preset".into(), "fast".into(),
                "-crf".into(), crf.into(), "-c:a".into(), "aac".into(), "-b:a".into(), abr.into(),
            ]);
            if req.output_format == "mp4" {
                args.extend(["-movflags".into(), "+faststart".into()]);
            }
        }
        "mkv" => {
            args.extend([
                "-c:v".into(), "libx264".into(), "-preset".into(), "fast".into(),
                "-crf".into(), crf.into(), "-c:a".into(), "aac".into(), "-b:a".into(), abr.into(),
            ]);
        }
        "webm" => {
            args.extend([
                "-c:v".into(), "libvpx-vp9".into(), "-crf".into(), crf_vp9.into(),
                "-b:v".into(), "0".into(), "-c:a".into(), "libopus".into(), "-b:a".into(), "128k".into(),
            ]);
        }
        "gif" => {
            // Two-pass palette approach for quality GIFs
            args.extend([
                "-vf".into(), "fps=15,scale=480:-1:flags=lanczos".into(),
                "-loop".into(), "0".into(),
            ]);
        }
        "mp3" => { args.extend(["-vn".into(), "-c:a".into(), "libmp3lame".into(), "-b:a".into(), abr.into()]); }
        "m4a" | "aac" => { args.extend(["-vn".into(), "-c:a".into(), "aac".into(), "-b:a".into(), abr.into()]); }
        "wav" => { args.extend(["-vn".into(), "-c:a".into(), "pcm_s16le".into()]); }
        "flac" => { args.extend(["-vn".into(), "-c:a".into(), "flac".into()]); }
        "opus" => { args.extend(["-vn".into(), "-c:a".into(), "libopus".into(), "-b:a".into(), abr.into()]); }
        fmt => return Err(format!("Unsupported output format: {fmt}")),
    }

    args.push(out_str);
    run_streaming(app, id, "ffmpeg", &args).await
}

// ── Main download command ─────────────────────────────────────────────────────

#[tauri::command]
pub async fn start_download(app: AppHandle, req: DownloadRequest) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let app2 = app.clone();
    let id2 = id.clone();

    // Spawn so the command returns the id immediately; progress comes via events.
    tokio::spawn(async move {
        let result = if detect_type(req.url.clone()) == "m3u8" {
            run_ffmpeg_m3u8(&app2, &id2, &req).await
        } else {
            run_yt_dlp(&app2, &id2, &req).await
        };

        match result {
            Ok(_) => emit(&app2, &id2, "done", "Download complete!", 100.0),
            Err(e) => emit(&app2, &id2, "error", &e, 0.0),
        }
    });

    Ok(id)
}

// ── yt-dlp backend ────────────────────────────────────────────────────────────

async fn run_yt_dlp(app: &AppHandle, id: &str, req: &DownloadRequest) -> Result<(), String> {
    emit(app, id, "log", "Starting yt-dlp…", 0.0);

    // For playlists, organise into a sub-folder named after the playlist.
    let out_tmpl = if req.filename.trim().is_empty() {
        if !req.no_playlist {
            // playlist mode: PlaylistName/01 - Title.ext
            format!("{}/%(playlist_title)s/%(playlist_index)s - %(title).80s.%(ext)s", req.output_dir)
        } else {
            format!("{}/%(title).100s.%(ext)s", req.output_dir)
        }
    } else {
        format!("{}/{}.%(ext)s", req.output_dir, req.filename.trim())
    };

    let mut args: Vec<String> = vec![
        "--newline".into(),
        "--no-colors".into(),
        "-f".into(),
        format_to_ytdlp_format(&req.quality),
        "--merge-output-format".into(),
        "mp4".into(),
        "-o".into(),
        out_tmpl,
    ];

    // N-challenge strategy:
    // - Without cookies: use ios client (bypasses n-challenge, no JS needed)
    // - With cookies: ios skips cookies, so use node/deno if available,
    //   otherwise fall back to default and hope yt-dlp manages
    let using_auth = !req.auth_mode.is_empty() && req.auth_mode != "none";
    if !using_auth {
        args.push("--extractor-args".into());
        args.push("youtube:player_client=ios,web".into());
    } else if command_exists("node") || command_exists("nodejs") {
        args.push("--js-runtimes".into());
        args.push("node".into());
    } else if command_exists("deno") {
        args.push("--js-runtimes".into());
        args.push("deno".into());
    }

    if req.no_playlist {
        args.push("--no-playlist".into());
    }
    if req.subtitles {
        args.extend(["--write-subs".into(), "--write-auto-subs".into(), "--sub-format".into(), "srt".into()]);
    }
    if req.embed_thumbnail {
        args.push("--embed-thumbnail".into());
    }
    if req.quality == "audio_mp3" {
        args.extend([
            "-x".into(),
            "--audio-format".into(), "mp3".into(),
            "--audio-quality".into(), "192K".into(),
        ]);
    } else if req.quality == "audio_m4a" {
        args.extend(["-x".into(), "--audio-format".into(), "m4a".into()]);
    }

    // Authentication
    append_auth_args(&mut args, &req.auth_mode, &req.auth_browser, &req.cookies_file);

    args.push(req.url.clone());

    run_streaming(app, id, "yt-dlp", &args).await
}

fn format_to_ytdlp_format(q: &str) -> String {
    match q {
        "best"      => "bestvideo+bestaudio/best",
        "1080p"     => "bestvideo[height<=1080]+bestaudio/best[height<=1080]",
        "720p"      => "bestvideo[height<=720]+bestaudio/best[height<=720]",
        "480p"      => "bestvideo[height<=480]+bestaudio/best[height<=480]",
        "360p"      => "bestvideo[height<=360]+bestaudio/best[height<=360]",
        "audio_mp3" => "bestaudio/best",
        "audio_m4a" => "bestaudio[ext=m4a]/bestaudio/best",
        other       => other,
    }.to_string()
}

// ── ffmpeg M3U8 backend ───────────────────────────────────────────────────────

async fn run_ffmpeg_m3u8(app: &AppHandle, id: &str, req: &DownloadRequest) -> Result<(), String> {
    emit(app, id, "log", "Detected M3U8 stream - using ffmpeg…", 0.0);

    let stem = if req.filename.trim().is_empty() {
        "stream_output".to_string()
    } else {
        req.filename.trim().to_string()
    };

    let out_path = format!("{}/{}.mp4", req.output_dir, stem);

    // Re-encode audio to AAC so MP4 mux works universally;
    // video is stream-copied (fast, no quality loss).
    let args: Vec<String> = vec![
        "-y".into(),
        "-loglevel".into(), "info".into(),
        "-stats".into(),
        "-i".into(), req.url.clone(),
        "-c:v".into(), "copy".into(),
        "-c:a".into(), "aac".into(),
        "-bsf:a".into(), "aac_adtstoasc".into(),
        "-movflags".into(), "+faststart".into(),
        out_path.clone(),
    ];

    emit(app, id, "log", &format!("Output → {out_path}"), 0.0);
    run_streaming(app, id, "ffmpeg", &args).await
}

// ── Playlist metadata commands ────────────────────────────────────────────────

/// Fetch all playlists from the authenticated user's YouTube library.
#[tauri::command]
pub async fn list_playlists(
    app: AppHandle,
    auth_mode: String,
    auth_browser: String,
    cookies_file: String,
) -> Result<Vec<PlaylistInfo>, String> {
    let mut args = vec![
        "--flat-playlist".into(),
        "--dump-single-json".into(),
        "--no-warnings".into(),
    ];
    append_auth_args(&mut args, &auth_mode, &auth_browser, &cookies_file);
    args.push("https://www.youtube.com/feed/playlists".into());

    let json_str = run_capture(&app, "yt-dlp", &args).await?;
    let root: serde_json::Value =
        serde_json::from_str(&json_str).map_err(|e| format!("JSON parse error: {e}"))?;

    let entries = root["entries"].as_array().ok_or("No entries in response")?;
    let playlists = entries
        .iter()
        .filter_map(|e| {
            let id = e["id"].as_str()?.to_string();
            let title = e["title"].as_str().unwrap_or("Untitled").to_string();
            let thumbnail = extract_thumbnail(e, &id, false);
            let video_count = e["playlist_count"]
                .as_u64()
                .or_else(|| e["video_count"].as_u64())
                .unwrap_or(0);
            Some(PlaylistInfo { id, title, thumbnail, video_count })
        })
        .collect();

    Ok(playlists)
}

/// Fetch the videos inside a specific playlist.
#[tauri::command]
pub async fn list_playlist_videos(
    app: AppHandle,
    playlist_id: String,
    auth_mode: String,
    auth_browser: String,
    cookies_file: String,
) -> Result<Vec<VideoInfo>, String> {
    let url = format!("https://www.youtube.com/playlist?list={playlist_id}");
    let mut args = vec![
        "--flat-playlist".into(),
        "--dump-single-json".into(),
        "--no-warnings".into(),
    ];
    append_auth_args(&mut args, &auth_mode, &auth_browser, &cookies_file);
    args.push(url);

    let json_str = run_capture(&app, "yt-dlp", &args).await?;
    let root: serde_json::Value =
        serde_json::from_str(&json_str).map_err(|e| format!("JSON parse error: {e}"))?;

    let entries = root["entries"].as_array().ok_or("No entries in response")?;
    let videos = entries
        .iter()
        .filter_map(|e| {
            let id = e["id"].as_str()?.to_string();
            let title = e["title"].as_str().unwrap_or("Untitled").to_string();
            let thumbnail = extract_thumbnail(e, &id, true);
            let duration_secs = e["duration"].as_f64().unwrap_or(0.0) as u64;
            let url = e["webpage_url"]
                .as_str()
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("https://www.youtube.com/watch?v={id}"));
            Some(VideoInfo { id, title, thumbnail, duration_secs, url })
        })
        .collect();

    Ok(videos)
}

/// Extract the best thumbnail URL from a yt-dlp JSON entry.
fn extract_thumbnail(entry: &serde_json::Value, id: &str, is_video: bool) -> String {
    // Direct thumbnail field
    if let Some(t) = entry["thumbnail"].as_str() {
        if !t.is_empty() { return t.to_string(); }
    }
    // Thumbnails array - pick last (usually highest res)
    if let Some(arr) = entry["thumbnails"].as_array() {
        if let Some(t) = arr.last().and_then(|t| t["url"].as_str()) {
            return t.to_string();
        }
    }
    // Fallback: construct YouTube CDN URL
    if is_video {
        format!("https://i.ytimg.com/vi/{id}/mqdefault.jpg")
    } else {
        // Playlist thumbnails via first video isn't reliable; return empty
        String::new()
    }
}

/// Run a command to completion and return all captured stdout as a String.
async fn run_capture(app: &AppHandle, program: &str, args: &[String]) -> Result<String, String> {
    use tauri_plugin_shell::process::CommandEvent;

    let mut cmd = app.shell().command(program);
    for a in args {
        cmd = cmd.arg(a);
    }

    let (mut rx, _child) = cmd.spawn().map_err(|e| {
        format!("Failed to spawn `{program}`: {e}. Is it installed and on PATH?")
    })?;

    let mut stdout_buf: Vec<u8> = Vec::new();
    let mut stderr_buf: Vec<u8> = Vec::new();

    while let Some(event) = rx.recv().await {
        match event {
            CommandEvent::Stdout(bytes) => stdout_buf.extend_from_slice(&bytes),
            CommandEvent::Stderr(bytes) => stderr_buf.extend_from_slice(&bytes),
            CommandEvent::Error(e) => return Err(format!("Process error: {e}")),
            CommandEvent::Terminated(status) => {
                if !status.code.map(|c| c == 0).unwrap_or(false) {
                    let stderr = String::from_utf8_lossy(&stderr_buf);
                    return Err(format!(
                        "`{program}` exited with code {:?}\n{stderr}",
                        status.code
                    ));
                }
                break;
            }
            _ => {}
        }
    }

    String::from_utf8(stdout_buf).map_err(|e| format!("UTF-8 decode error: {e}"))
}

// ── Shared streaming runner ───────────────────────────────────────────────────

async fn run_streaming(
    app: &AppHandle,
    id: &str,
    program: &str,
    args: &[String],
) -> Result<(), String> {
    use tauri_plugin_shell::process::CommandEvent;

    let mut cmd = app.shell().command(program);
    for a in args {
        cmd = cmd.arg(a);
    }

    let (mut rx, _child) = cmd.spawn().map_err(|e| {
        format!("Failed to spawn `{program}`: {e}. Is it installed and on PATH?")
    })?;

    while let Some(event) = rx.recv().await {
        match event {
            CommandEvent::Stdout(line) | CommandEvent::Stderr(line) => {
                let text = String::from_utf8_lossy(&line).to_string();
                let pct = parse_percent(&text);
                emit(app, id, "log", &text, pct);
            }
            CommandEvent::Error(e) => {
                return Err(format!("Process error: {e}"));
            }
            CommandEvent::Terminated(status) => {
                if !status.code.map(|c| c == 0).unwrap_or(false) {
                    return Err(format!(
                        "`{program}` exited with code {:?}",
                        status.code
                    ));
                }
                break;
            }
            _ => {}
        }
    }
    Ok(())
}

/// Extract a progress percentage from yt-dlp or ffmpeg output lines.
fn parse_percent(line: &str) -> f64 {
    // yt-dlp:  "[download]  42.3% of ..."
    if let Some(pos) = line.find('%') {
        let before = &line[..pos];
        if let Some(start) = before.rfind(|c: char| c == ' ' || c == '\t') {
            if let Ok(pct) = before[start + 1..].trim().parse::<f64>() {
                return pct.clamp(0.0, 100.0);
            }
        }
    }
    // ffmpeg: "time=00:01:23.45" - we can't easily get % without duration,
    // so just return -1 to signal "indeterminate"
    if line.contains("time=") {
        return -1.0;
    }
    0.0
}
