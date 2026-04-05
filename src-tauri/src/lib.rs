mod downloader;

use tauri::{WebviewUrl, WebviewWindowBuilder};
use url::Url;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // Pre-create all login windows hidden before the event loop starts.
            // build() must be called here (in setup) to avoid a WRY/WebView2
            // deadlock that occurs when build() is called from run_on_main_thread.

            // ── YouTube ──────────────────────────────────────────────────────
            let yt_js = r#"
// Spoof window.chrome so Google's login flow doesn't treat this as an
// unsupported embedded webview and serve a blank page after email entry.
(function () {
  if (!window.chrome) {
    Object.defineProperty(window, 'chrome', {
      value: {
        app: { isInstalled: false, InstallState: {}, RunningState: {} },
        runtime: { id: undefined, connect: function(){}, sendMessage: function(){} },
        loadTimes: function() { return {}; },
        csi: function() { return {}; }
      },
      configurable: true, writable: true, enumerable: false
    });
  }
})();
(function () {
  var host = window.location.hostname;
  if (window.__ytCookieCaptureActive) return;
  window.__ytCookieCaptureActive = true;
  var captured = false;
  function tryCapture() {
    if (captured) return;
    var c = document.cookie;
    if (!c.includes('SAPISID')) return;
    captured = true;
    function doInvoke() {
      var api = window.__TAURI__ && window.__TAURI__.core;
      if (api && api.invoke) {
        api.invoke('extract_yt_cookies', {})
           .catch(function(e) { console.error('[yt-login] extract FAILED: ' + e); captured = false; });
      } else {
        setTimeout(doInvoke, 400);
      }
    }
    doInvoke();
  }
  if (host === 'www.youtube.com' || host === 'youtube.com') {
    window.addEventListener('load', tryCapture);
    var iv = setInterval(function () { tryCapture(); if (captured) clearInterval(iv); }, 1200);
  }
})();
"#;
            WebviewWindowBuilder::new(
                app,
                "youtube-login",
                WebviewUrl::External(Url::parse("about:blank").expect("static URL")),
            )
            .title("Sign in to YouTube")
            .inner_size(520.0, 720.0)
            .resizable(true)
            .visible(false)
            .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
            .initialization_script(yt_js)
            .build()?;

            // ── Instagram ────────────────────────────────────────────────────
            let ig_js = r#"
(function () {
  var host = window.location.hostname;
  function tryCapture() {
    if (window.__igCaptureDone) return;
    if (host !== 'www.instagram.com' && host !== 'instagram.com') return;
    var path = window.location.pathname;
    var onLoginPage = path.includes('/accounts/login') || path.includes('/accounts/emailsignup');
    if (onLoginPage) return;
    window.__igCaptureDone = true;
    function doInvoke() {
      var api = window.__TAURI__ && window.__TAURI__.core;
      if (api && api.invoke) {
        api.invoke('extract_ig_cookies', {})
           .catch(function(e) { console.error('[ig-login] FAILED: ' + e); window.__igCaptureDone = false; });
      } else {
        setTimeout(doInvoke, 400);
      }
    }
    doInvoke();
  }
  window.addEventListener('load', tryCapture);
  var iv = setInterval(function () { tryCapture(); if (window.__igCaptureDone) clearInterval(iv); }, 1200);
})();
"#;
            WebviewWindowBuilder::new(
                app,
                "instagram-login",
                WebviewUrl::External(Url::parse("about:blank").expect("static URL")),
            )
            .title("Sign in to Instagram")
            .inner_size(520.0, 720.0)
            .resizable(true)
            .visible(false)
            .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
            .initialization_script(ig_js)
            .build()?;

            // ── Facebook ─────────────────────────────────────────────────────
            let fb_js = r#"
(function () {
  var host = window.location.hostname;
  function tryCapture() {
    if (window.__fbCaptureDone) return;
    if (host !== 'www.facebook.com' && host !== 'facebook.com' && host !== 'm.facebook.com') return;
    // c_user is Facebook's user-ID cookie - present once logged in
    if (!document.cookie.includes('c_user')) return;
    window.__fbCaptureDone = true;
    function doInvoke() {
      var api = window.__TAURI__ && window.__TAURI__.core;
      if (api && api.invoke) {
        api.invoke('extract_fb_cookies', {})
           .catch(function(e) { console.error('[fb-login] FAILED: ' + e); window.__fbCaptureDone = false; });
      } else {
        setTimeout(doInvoke, 400);
      }
    }
    doInvoke();
  }
  window.addEventListener('load', tryCapture);
  var iv = setInterval(function () { tryCapture(); if (window.__fbCaptureDone) clearInterval(iv); }, 1200);
})();
"#;
            WebviewWindowBuilder::new(
                app,
                "facebook-login",
                WebviewUrl::External(Url::parse("about:blank").expect("static URL")),
            )
            .title("Sign in to Facebook")
            .inner_size(520.0, 720.0)
            .resizable(true)
            .visible(false)
            .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
            .initialization_script(fb_js)
            .build()?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            downloader::start_download,
            downloader::detect_type,
            downloader::open_folder,
            downloader::check_tools,
            downloader::verify_auth,
            downloader::list_playlists,
            downloader::list_playlist_videos,
            downloader::load_auth_state,
            downloader::convert_file,
            downloader::open_youtube_login,
            downloader::open_instagram_login,
            downloader::open_facebook_login,
            downloader::save_yt_cookies,
            downloader::extract_yt_cookies,
            downloader::extract_ig_cookies,
            downloader::extract_fb_cookies,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
