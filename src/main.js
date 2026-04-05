// __TAURI__ is injected by withGlobalTauri - wait for it if needed
async function waitForTauri() {
  while (!window.__TAURI__?.core) {
    await new Promise(r => setTimeout(r, 20));
  }
}

let invoke, listen;

async function initTauri() {
  await waitForTauri();
  invoke = window.__TAURI__.core.invoke;
  listen = window.__TAURI__.event.listen;
}

// Dialog plugin - available via global when withGlobalTauri:true
async function pickDirectory() {
  try {
    const { open } = window.__TAURI__?.dialog ?? {};
    if (open) return await open({ directory: true, multiple: false });
  } catch {}
  return prompt("Enter output folder path:");
}

async function pickFile(filters) {
  try {
    const { open } = window.__TAURI__?.dialog ?? {};
    if (open) return await open({ multiple: false, filters });
  } catch {}
  return prompt("Path to file:");
}

// ── Auth state (per platform) ─────────────────────────────────────────────────
const ytState = { enabled: false, browser: "chrome", cookiesFile: "" };
const igState = { enabled: false, browser: "chrome", cookiesFile: "" };
const fbState = { enabled: false, browser: "chrome", cookiesFile: "" };

// Pick the right auth for a given URL based on domain
function getAuthForUrl(url) {
  const u = url.toLowerCase();
  let state;
  if (u.includes("instagram.com")) {
    state = igState;
  } else if (u.includes("facebook.com") || u.includes("fb.com") || u.includes("fb.watch")) {
    state = fbState;
  } else {
    state = ytState;
  }
  if (!state.enabled) return { authMode: "none", authBrowser: "", cookiesFile: "" };
  if (state.cookiesFile) return { authMode: "cookies_file", authBrowser: "", cookiesFile: state.cookiesFile };
  return { authMode: "browser", authBrowser: state.browser, cookiesFile: "" };
}

// YouTube-specific auth args for playlist commands
function getYtAuthArgs() {
  return {
    authMode:    ytState.enabled ? (ytState.cookiesFile ? "cookies_file" : "browser") : "none",
    authBrowser: ytState.browser,
    cookiesFile: ytState.cookiesFile,
  };
}

// ── State ─────────────────────────────────────────────────────────────────────
const downloads = new Map(); // id → { el, logEl, progressFill, progressPct, statusText, statusIcon }

// ── DOM refs - Download tab ───────────────────────────────────────────────────
const urlInput      = document.getElementById("url-input");
const urlBadge      = document.getElementById("url-badge");
const qualitySelect = document.getElementById("quality-select");
const filenameInput = document.getElementById("filename-input");
const dirInput      = document.getElementById("dir-input");
const browseBtn     = document.getElementById("browse-btn");
const downloadBtn   = document.getElementById("download-btn");
const toolBadges    = document.getElementById("tool-badges");
const dlSection     = document.getElementById("downloads-section");
const dlList        = document.getElementById("downloads-list");
const clearDoneBtn  = document.getElementById("clear-done-btn");
const chkPlaylist   = document.getElementById("chk-playlist");
const chkSubs       = document.getElementById("chk-subs");
const chkThumb      = document.getElementById("chk-thumb");

// ── DOM refs - Auth tab (YouTube) ─────────────────────────────────────────────
const ytAuthEnabled    = document.getElementById("yt-auth-enabled");
const ytAuthControls   = document.getElementById("yt-auth-controls");
const ytAuthChip       = document.getElementById("yt-auth-chip");
const ytBrowserSelect  = document.getElementById("yt-browser-select");
const ytVerifyBtn      = document.getElementById("yt-verify-btn");
const ytCookiesFile    = document.getElementById("yt-cookies-file");
const ytBrowseCookiesBtn = document.getElementById("yt-browse-cookies-btn");
const signinBtn        = document.getElementById("signin-btn");
const signinStatus     = document.getElementById("signin-status");
const dpapiNotice      = document.getElementById("dpapi-notice");
const dpapiBrowserName = document.getElementById("dpapi-browser-name");
const ytAuthResult     = document.getElementById("yt-auth-result");

// ── DOM refs - Auth tab (Instagram) ──────────────────────────────────────────
const igAuthEnabled      = document.getElementById("ig-auth-enabled");
const igAuthControls     = document.getElementById("ig-auth-controls");
const igAuthChip         = document.getElementById("ig-auth-chip");
const igBrowserSelect    = document.getElementById("ig-browser-select");
const igCookiesFile      = document.getElementById("ig-cookies-file");
const igBrowseCookiesBtn = document.getElementById("ig-browse-cookies-btn");
const igSigninBtn        = document.getElementById("ig-signin-btn");
const igSigninStatus     = document.getElementById("ig-signin-status");
const igAuthResult       = document.getElementById("ig-auth-result");

// ── DOM refs - Auth tab (Facebook) ────────────────────────────────────────────
const fbAuthEnabled      = document.getElementById("fb-auth-enabled");
const fbAuthControls     = document.getElementById("fb-auth-controls");
const fbAuthChip         = document.getElementById("fb-auth-chip");
const fbBrowserSelect    = document.getElementById("fb-browser-select");
const fbCookiesFile      = document.getElementById("fb-cookies-file");
const fbBrowseCookiesBtn = document.getElementById("fb-browse-cookies-btn");
const fbSigninBtn        = document.getElementById("fb-signin-btn");
const fbSigninStatus     = document.getElementById("fb-signin-status");
const fbAuthResult       = document.getElementById("fb-auth-result");

// ── Init ──────────────────────────────────────────────────────────────────────
function applyPlatformAuthUI() {
  const isLinux = navigator.userAgent.includes('Linux');
  if (!isLinux) return;

  // In-app sign-in doesn't work on Linux — Google blocks embedded webviews.
  // Hide the sign-in buttons and auto-expand the browser fallback instead.
  for (const id of ['signin-btn', 'ig-signin-btn', 'fb-signin-btn']) {
    const el = document.getElementById(id);
    if (el) el.style.display = 'none';
  }
  for (const id of ['signin-status', 'ig-signin-status', 'fb-signin-status']) {
    const el = document.getElementById(id);
    if (el) { el.textContent = 'Use the browser fallback below to authenticate on Linux.'; el.style.opacity = '0.6'; }
  }
  for (const el of document.querySelectorAll('.browser-fallback')) {
    el.setAttribute('open', '');
  }
}

async function init() {
  try {
    const dir = await window.__TAURI__.path.downloadDir();
    if (dir) dirInput.value = dir;
  } catch {}
  applyPlatformAuthUI();
  await checkTools();
  await restoreAuthState();
  await listen("dl-progress", onProgress);
  await setupCookieListener();
}

async function restoreAuthState() {
  try {
    const saved = await invoke("load_auth_state");

    if (saved.yt_cookies) {
      ytState.cookiesFile = saved.yt_cookies;
      ytState.enabled = true;
      ytAuthEnabled.checked = true;
      ytCookiesFile.value = saved.yt_cookies;
      ytAuthControls.classList.remove("hidden");
      updateYtAuthChip();
    }

    if (saved.ig_cookies) {
      igState.cookiesFile = saved.ig_cookies;
      igState.enabled = true;
      igAuthEnabled.checked = true;
      igCookiesFile.value = saved.ig_cookies;
      igAuthControls.classList.remove("hidden");
      updateIgAuthChip();
    }

    if (saved.fb_cookies) {
      fbState.cookiesFile = saved.fb_cookies;
      fbState.enabled = true;
      fbAuthEnabled.checked = true;
      fbCookiesFile.value = saved.fb_cookies;
      fbAuthControls.classList.remove("hidden");
      updateFbAuthChip();
    }
  } catch (e) {
    console.error("restoreAuthState:", e);
  }
}

// ── Tool check ────────────────────────────────────────────────────────────────
async function checkTools() {
  try {
    const status = await invoke("check_tools");
    renderToolBadge("yt-dlp", status.yt_dlp);
    renderToolBadge("ffmpeg", status.ffmpeg);
    if (status.yt_dlp && status.yt_dlp_outdated) {
      const warn = document.createElement("span");
      warn.className = "tool-chip err";
      warn.textContent = `⚠ yt-dlp ${status.yt_dlp_version} outdated`;
      warn.title = "yt-dlp is too old and may fail on YouTube downloads. Update: sudo wget https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp -O /usr/local/bin/yt-dlp && sudo chmod +x /usr/local/bin/yt-dlp";
      warn.style.cursor = "help";
      toolBadges.appendChild(warn);
    }
    if (status.yt_dlp && !status.has_js_runtime) {
      const warn = document.createElement("span");
      warn.className = "tool-chip err";
      warn.textContent = "⚠ no JS runtime";
      warn.title = "Node.js or Deno is required for authenticated YouTube downloads (n-challenge). Install with: sudo apt install nodejs";
      warn.style.cursor = "help";
      toolBadges.appendChild(warn);
    }
  } catch (e) {
    console.error("check_tools failed:", e);
  }
}

function renderToolBadge(name, ok) {
  const chip = document.createElement("span");
  chip.className = `tool-chip ${ok ? "ok" : "err"}`;
  chip.textContent = ok ? `✓ ${name}` : `✗ ${name}`;
  chip.title = ok ? `${name} found on PATH` : `${name} not found - install it and add to PATH`;
  toolBadges.appendChild(chip);
}

// ── URL detection ─────────────────────────────────────────────────────────────
urlInput.addEventListener("input", debounce(detectUrl, 300));

async function detectUrl() {
  const url = urlInput.value.trim();
  downloadBtn.disabled = !url;

  if (!url) {
    urlBadge.className = "badge hidden";
    return;
  }

  try {
    const type = await invoke("detect_type", { url });
    if (type === "m3u8") {
      urlBadge.className = "badge m3u8";
      urlBadge.textContent = "M3U8";
      qualitySelect.closest(".field").style.opacity = "0.4";
      qualitySelect.disabled = true;
    } else {
      urlBadge.className = "badge ytdlp";
      urlBadge.textContent = "yt-dlp";
      qualitySelect.closest(".field").style.opacity = "";
      qualitySelect.disabled = false;
    }
  } catch {
    urlBadge.className = "badge hidden";
  }
}

// ── Browse (output dir) ───────────────────────────────────────────────────────
browseBtn.addEventListener("click", async () => {
  const selected = await pickDirectory();
  if (selected) dirInput.value = selected;
});

// ── Auth chip helpers ─────────────────────────────────────────────────────────
function updateYtAuthChip() {
  if (!ytState.enabled) { ytAuthChip.className = "badge hidden"; return; }
  ytAuthChip.className = "badge ytdlp";
  ytAuthChip.textContent = ytState.cookiesFile ? "cookies.txt" : ytState.browser;
}

function updateIgAuthChip() {
  if (!igState.enabled) { igAuthChip.className = "badge hidden"; return; }
  igAuthChip.className = "badge ig";
  igAuthChip.textContent = igState.cookiesFile ? "cookies.txt" : igState.browser;
}

function updateFbAuthChip() {
  if (!fbState.enabled) { fbAuthChip.className = "badge hidden"; return; }
  fbAuthChip.className = "badge fb";
  fbAuthChip.textContent = fbState.cookiesFile ? "cookies.txt" : fbState.browser;
}

// ── YouTube auth controls ─────────────────────────────────────────────────────
ytAuthEnabled.addEventListener("change", () => {
  ytState.enabled = ytAuthEnabled.checked;
  ytAuthControls.classList.toggle("hidden", !ytState.enabled);
  if (!ytState.enabled) {
    dpapiNotice.classList.add("hidden");
    signinStatus.textContent = "";
    signinStatus.className = "signin-status";
    ytAuthResult.classList.add("hidden");
  }
  updateYtAuthChip();
});

ytBrowserSelect.addEventListener("change", () => {
  ytState.browser = ytBrowserSelect.value;
  updateYtAuthChip();
  ytAuthResult.classList.add("hidden");
  dpapiNotice.classList.add("hidden");
});

ytBrowseCookiesBtn.addEventListener("click", async () => {
  const file = await pickFile([
    { name: "Cookies", extensions: ["txt"] },
    { name: "All Files", extensions: ["*"] },
  ]);
  if (file) {
    ytState.cookiesFile = file;
    ytCookiesFile.value = file;
    updateYtAuthChip();
    runVerifyAuth();
  }
});

// ── In-app YouTube sign-in ────────────────────────────────────────────────────
signinBtn.addEventListener("click", async () => {
  signinBtn.disabled = true;
  signinStatus.className = "signin-status";
  signinStatus.textContent = "Opening sign-in window…";
  try {
    await invoke("open_youtube_login");
    signinStatus.textContent = "Log in to YouTube in the window that just opened…";
  } catch (e) {
    signinStatus.className = "signin-status err";
    signinStatus.textContent = `Failed to open window: ${e}`;
    signinBtn.disabled = false;
  }
});

// Rust emits these events after each login webview captures cookies
async function setupCookieListener() {
  await listen("yt-cookies-saved", (event) => {
    const path = event.payload;
    ytState.cookiesFile = path;
    ytCookiesFile.value = path;
    updateYtAuthChip();
    signinBtn.disabled = false;
    signinStatus.className = "signin-status ok";
    signinStatus.textContent = "✓ Signed in - cookies saved";
    dpapiNotice.classList.add("hidden");
    ytAuthResult.className = "auth-result ok";
    ytAuthResult.classList.remove("hidden");
    ytAuthResult.textContent = "✓ Signed in successfully via YouTube";
  });

  await listen("ig-cookies-saved", (event) => {
    const path = event.payload;
    igState.cookiesFile = path;
    igCookiesFile.value = path;
    updateIgAuthChip();
    igSigninBtn.disabled = false;
    igSigninStatus.className = "signin-status ok";
    igSigninStatus.textContent = "✓ Signed in - cookies saved";
    igAuthResult.className = "auth-result ok";
    igAuthResult.classList.remove("hidden");
    igAuthResult.textContent = "✓ Signed in successfully via Instagram";
  });

  await listen("fb-cookies-saved", (event) => {
    const path = event.payload;
    fbState.cookiesFile = path;
    fbCookiesFile.value = path;
    updateFbAuthChip();
    fbSigninBtn.disabled = false;
    fbSigninStatus.className = "signin-status ok";
    fbSigninStatus.textContent = "✓ Signed in - cookies saved";
    fbAuthResult.className = "auth-result ok";
    fbAuthResult.classList.remove("hidden");
    fbAuthResult.textContent = "✓ Signed in successfully via Facebook";
  });
}

ytVerifyBtn.addEventListener("click", runVerifyAuth);

async function runVerifyAuth() {
  ytVerifyBtn.disabled = true;
  ytAuthResult.className = "auth-result testing";
  ytAuthResult.classList.remove("hidden");
  ytAuthResult.textContent = "Testing… checking YouTube access";

  try {
    const msg = await invoke("verify_auth", {
      authMode:    ytState.cookiesFile ? "cookies_file" : "browser",
      authBrowser: ytState.browser,
      cookiesFile: ytState.cookiesFile,
    });
    ytAuthResult.className = "auth-result ok";
    ytAuthResult.textContent = msg;
    dpapiNotice.classList.add("hidden");
  } catch (e) {
    const err = String(e);
    if (err.startsWith("DPAPI:")) {
      const browser = err.slice(6);
      dpapiBrowserName.textContent = browser;
      dpapiNotice.classList.remove("hidden");
      ytAuthResult.className = "auth-result err";
      ytAuthResult.textContent =
        `${browser} uses Windows App-Bound Encryption - direct cookie access is blocked. ` +
        `Use Sign in with YouTube or the cookies.txt file option.`;
    } else {
      ytAuthResult.className = "auth-result err";
      ytAuthResult.textContent = err;
    }
  } finally {
    ytVerifyBtn.disabled = false;
  }
}

// ── Instagram auth controls ───────────────────────────────────────────────────
igAuthEnabled.addEventListener("change", () => {
  igState.enabled = igAuthEnabled.checked;
  igAuthControls.classList.toggle("hidden", !igState.enabled);
  if (!igState.enabled) {
    igSigninStatus.textContent = "";
    igSigninStatus.className = "signin-status";
    igAuthResult.classList.add("hidden");
  }
  updateIgAuthChip();
});

igBrowserSelect.addEventListener("change", () => {
  igState.browser = igBrowserSelect.value;
  updateIgAuthChip();
});

igBrowseCookiesBtn.addEventListener("click", async () => {
  const file = await pickFile([
    { name: "Cookies", extensions: ["txt"] },
    { name: "All Files", extensions: ["*"] },
  ]);
  if (file) {
    igState.cookiesFile = file;
    igCookiesFile.value = file;
    updateIgAuthChip();
  }
});

igSigninBtn.addEventListener("click", async () => {
  igSigninBtn.disabled = true;
  igSigninStatus.className = "signin-status";
  igSigninStatus.textContent = "Opening sign-in window…";
  try {
    await invoke("open_instagram_login");
    igSigninStatus.textContent = "Log in to Instagram in the window that just opened…";
  } catch (e) {
    igSigninStatus.className = "signin-status err";
    igSigninStatus.textContent = `Failed to open window: ${e}`;
    igSigninBtn.disabled = false;
  }
});

// ── Facebook auth controls ────────────────────────────────────────────────────
fbAuthEnabled.addEventListener("change", () => {
  fbState.enabled = fbAuthEnabled.checked;
  fbAuthControls.classList.toggle("hidden", !fbState.enabled);
  if (!fbState.enabled) {
    fbSigninStatus.textContent = "";
    fbSigninStatus.className = "signin-status";
    fbAuthResult.classList.add("hidden");
  }
  updateFbAuthChip();
});

fbBrowserSelect.addEventListener("change", () => {
  fbState.browser = fbBrowserSelect.value;
  updateFbAuthChip();
});

fbBrowseCookiesBtn.addEventListener("click", async () => {
  const file = await pickFile([
    { name: "Cookies", extensions: ["txt"] },
    { name: "All Files", extensions: ["*"] },
  ]);
  if (file) {
    fbState.cookiesFile = file;
    fbCookiesFile.value = file;
    updateFbAuthChip();
  }
});

fbSigninBtn.addEventListener("click", async () => {
  fbSigninBtn.disabled = true;
  fbSigninStatus.className = "signin-status";
  fbSigninStatus.textContent = "Opening sign-in window…";
  try {
    await invoke("open_facebook_login");
    fbSigninStatus.textContent = "Log in to Facebook in the window that just opened…";
  } catch (e) {
    fbSigninStatus.className = "signin-status err";
    fbSigninStatus.textContent = `Failed to open window: ${e}`;
    fbSigninBtn.disabled = false;
  }
});

// ── Download ──────────────────────────────────────────────────────────────────
downloadBtn.addEventListener("click", startDownload);
urlInput.addEventListener("keydown", e => { if (e.key === "Enter") startDownload(); });

async function startDownload() {
  const url = urlInput.value.trim();
  const outputDir = dirInput.value.trim();

  if (!url)       return alert("Please enter a URL.");
  if (!outputDir) return alert("Please choose an output folder.");

  downloadBtn.disabled = true;
  setTimeout(() => { downloadBtn.disabled = !urlInput.value.trim(); }, 500);

  const auth = getAuthForUrl(url);
  const req = {
    url,
    output_dir:      outputDir,
    quality:         qualitySelect.value,
    filename:        filenameInput.value.trim(),
    subtitles:       chkSubs.checked,
    embed_thumbnail: chkThumb.checked,
    no_playlist:     !chkPlaylist.checked,
    auth_mode:       auth.authMode,
    auth_browser:    auth.authBrowser,
    cookies_file:    auth.cookiesFile,
  };

  try {
    const id = await invoke("start_download", { req });
    createDownloadItem(id, url);
  } catch (e) {
    alert(`Failed to start download:\n${e}`);
  }
}

// ── Download item ─────────────────────────────────────────────────────────────
function createDownloadItem(id, url) {
  dlSection.classList.remove("hidden");

  const el = document.createElement("div");
  el.className = "dl-item";
  el.dataset.id = id;

  el.innerHTML = `
    <div class="dl-header">
      <div class="dl-status-icon running" data-icon>⟳</div>
      <div class="dl-info">
        <div class="dl-url">${escHtml(url)}</div>
        <div class="dl-status-text" data-status>Starting…</div>
      </div>
      <div class="dl-actions">
        <button class="dl-action-btn" data-open title="Open folder">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/>
          </svg>
        </button>
        <button class="dl-action-btn" data-remove title="Remove">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/>
          </svg>
        </button>
      </div>
    </div>
    <div class="dl-progress">
      <div class="progress-track"><div class="progress-fill indeterminate" data-fill></div></div>
      <div class="progress-pct" data-pct></div>
    </div>
    <div class="dl-log-toggle" data-log-toggle>▶ Show log</div>
    <div class="dl-log" data-log></div>
  `;

  const openBtn   = el.querySelector("[data-open]");
  const removeBtn = el.querySelector("[data-remove]");
  const logToggle = el.querySelector("[data-log-toggle]");
  const logEl     = el.querySelector("[data-log]");

  openBtn.addEventListener("click", () => {
    invoke("open_folder", { path: dirInput.value.trim() });
  });

  removeBtn.addEventListener("click", () => {
    el.remove();
    downloads.delete(id);
    if (dlList.children.length === 0) dlSection.classList.add("hidden");
  });

  logToggle.addEventListener("click", () => {
    const open = logEl.classList.toggle("open");
    logToggle.textContent = open ? "▼ Hide log" : "▶ Show log";
  });

  dlList.prepend(el);

  downloads.set(id, {
    el,
    logEl,
    progressFill: el.querySelector("[data-fill]"),
    progressPct:  el.querySelector("[data-pct]"),
    statusText:   el.querySelector("[data-status]"),
    statusIcon:   el.querySelector("[data-icon]"),
  });
}

// ── Progress event handler ────────────────────────────────────────────────────
function onProgress({ payload }) {
  const { id, kind, message, percent } = payload;
  const item = downloads.get(id);
  if (!item) return;

  const { el, logEl, progressFill, progressPct, statusText, statusIcon } = item;

  if (kind === "log") {
    appendLog(logEl, message, false);
    const trimmed = message.trim();
    if (trimmed) statusText.textContent = trimmed.slice(0, 120);

    if (percent > 0) {
      progressFill.classList.remove("indeterminate");
      progressFill.style.width = `${percent}%`;
      progressPct.textContent  = `${percent.toFixed(1)}%`;
    } else if (percent === -1) {
      progressFill.classList.add("indeterminate");
      progressPct.textContent = "";
    }

  } else if (kind === "done") {
    el.classList.add("done");
    progressFill.classList.remove("indeterminate");
    progressFill.style.width = "100%";
    progressPct.textContent  = "100%";
    statusText.textContent   = "Complete";
    statusIcon.textContent   = "✓";
    statusIcon.className     = "dl-status-icon done";
    appendLog(logEl, "Done!", true);

  } else if (kind === "error") {
    el.classList.add("error");
    progressFill.style.background = "var(--red)";
    progressFill.classList.remove("indeterminate");
    statusText.textContent = message.slice(0, 200);
    statusIcon.textContent = "✗";
    statusIcon.className   = "dl-status-icon error";
    appendLog(logEl, message, false, true);
    logEl.classList.add("open");
    el.querySelector("[data-log-toggle]").textContent = "▼ Hide log";
  }
}

function appendLog(logEl, text, isOk, isErr) {
  const line = document.createElement("div");
  if (isOk)  line.className = "ok";
  if (isErr) line.className = "err";
  line.textContent = text;
  logEl.appendChild(line);
  logEl.scrollTop = logEl.scrollHeight;
}

// ── Clear finished ────────────────────────────────────────────────────────────
clearDoneBtn.addEventListener("click", () => {
  for (const [id, item] of downloads) {
    if (item.el.classList.contains("done") || item.el.classList.contains("error")) {
      item.el.remove();
      downloads.delete(id);
    }
  }
  if (dlList.children.length === 0) dlSection.classList.add("hidden");
});

// ── Helpers ───────────────────────────────────────────────────────────────────
function escHtml(s) {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

function debounce(fn, ms) {
  let t;
  return (...args) => { clearTimeout(t); t = setTimeout(() => fn(...args), ms); };
}

// ══════════════════════════════════════════════════════════════════════════════
// ── Tab switching ─────────────────────────────────────────────────────────────
// ══════════════════════════════════════════════════════════════════════════════

function switchTab(name) {
  document.querySelectorAll(".tab-btn").forEach(b =>
    b.classList.toggle("active", b.dataset.tab === name));
  document.querySelectorAll(".tab-panel").forEach(p =>
    p.classList.toggle("hidden", p.id !== `tab-${name}`));
  if (name === "playlists") onPlaylistsTabOpen();
}

document.querySelectorAll(".tab-btn").forEach(btn =>
  btn.addEventListener("click", () => switchTab(btn.dataset.tab)));

// ══════════════════════════════════════════════════════════════════════════════
// ── My Playlists tab ──────────────────────────────────────────────────────────
// ══════════════════════════════════════════════════════════════════════════════

const plAuthNotice  = document.getElementById("pl-auth-notice");
const plLoadBtn     = document.getElementById("pl-load-btn");
const plStatus      = document.getElementById("pl-status");
const plGrid        = document.getElementById("pl-grid");
const plDetail      = document.getElementById("playlist-detail");
const plDetailTitle = document.getElementById("pl-detail-title");
const plDetailBack  = document.getElementById("pl-detail-back");
const plDetailDlAll = document.getElementById("pl-detail-dl-all");
const plDetailVids  = document.getElementById("pl-detail-videos");

let currentPlaylistId = "";

function onPlaylistsTabOpen() {
  plAuthNotice.classList.toggle("hidden", ytState.enabled);
  plLoadBtn.disabled = !ytState.enabled;
}

plLoadBtn.addEventListener("click", loadPlaylists);

async function loadPlaylists() {
  plGrid.innerHTML = "";
  plStatus.className = "pl-status-text";
  plStatus.textContent = "Loading…";
  plLoadBtn.disabled = true;

  try {
    const playlists = await invoke("list_playlists", getYtAuthArgs());
    plStatus.textContent = playlists.length ? `${playlists.length} playlists` : "No playlists found";
    renderPlaylistGrid(playlists);
  } catch (e) {
    plStatus.className = "pl-status-text error";
    plStatus.textContent = `Error: ${String(e).slice(0, 200)}`;
  } finally {
    plLoadBtn.disabled = !ytState.enabled;
  }
}

function renderPlaylistGrid(playlists) {
  plGrid.innerHTML = "";

  for (const pl of playlists) {
    const card = document.createElement("div");
    card.className = "pl-card";

    const thumbHtml = pl.thumbnail
      ? `<img class="pl-card-thumb" alt="" loading="lazy">`
      : `<div class="pl-card-thumb-placeholder">${svgPlaylistIcon()}</div>`;

    card.innerHTML = `
      ${thumbHtml}
      <div class="pl-card-info">
        <div class="pl-card-title">${escHtml(pl.title)}</div>
        <div class="pl-card-count">${pl.video_count ? `${pl.video_count} videos` : "Videos"}</div>
        <div class="pl-card-actions">
          <button class="btn-secondary" data-view>View videos</button>
          <button class="btn-secondary" data-dlall>↓ All</button>
        </div>
      </div>`;

    if (pl.thumbnail) {
      const img = card.querySelector(".pl-card-thumb");
      img.src = pl.thumbnail;
      img.onerror = function() { this.replaceWith(makePlaceholderThumb()); };
    }

    card.querySelector("[data-view]").addEventListener("click", e => {
      e.stopPropagation();
      openPlaylistDetail(pl);
    });
    card.querySelector("[data-dlall]").addEventListener("click", e => {
      e.stopPropagation();
      startPlaylistDownload(pl.id, pl.title);
    });
    card.addEventListener("click", () => openPlaylistDetail(pl));

    plGrid.appendChild(card);
  }
}

// ── Detail panel ──────────────────────────────────────────────────────────────

plDetailBack.addEventListener("click", closeDetail);

function closeDetail() {
  plDetail.classList.remove("open");
}

async function openPlaylistDetail(pl) {
  currentPlaylistId = pl.id;
  plDetailTitle.textContent = pl.title;
  plDetailVids.innerHTML = spinnerHtml("Loading videos…");
  plDetail.classList.add("open");

  plDetailDlAll.onclick = () => startPlaylistDownload(pl.id, pl.title);

  try {
    const videos = await invoke("list_playlist_videos", {
      playlistId: pl.id,
      ...getYtAuthArgs(),
    });
    renderVideoList(videos);
  } catch (e) {
    plDetailVids.innerHTML = `<div class="pl-status-text error">Error: ${escHtml(String(e))}</div>`;
  }
}

function renderVideoList(videos) {
  plDetailVids.innerHTML = "";

  if (!videos.length) {
    plDetailVids.innerHTML = `<div class="pl-status-text">No videos found.</div>`;
    return;
  }

  for (let i = 0; i < videos.length; i++) {
    const v = videos[i];
    const row = document.createElement("div");
    row.className = "pl-video-row";
    row.innerHTML = `
      <span class="pl-video-index">${i + 1}</span>
      <img class="pl-video-thumb" alt="" loading="lazy">
      <div class="pl-video-info">
        <div class="pl-video-title">${escHtml(v.title)}</div>
        <div class="pl-video-duration">${formatDuration(v.duration_secs)}</div>
      </div>
      <button class="btn-secondary" style="flex-shrink:0;font-size:0.80rem;padding:6px 12px">↓ Download</button>`;

    const thumb = row.querySelector(".pl-video-thumb");
    thumb.src = v.thumbnail;
    thumb.onerror = function() { this.style.visibility = "hidden"; };

    row.querySelector("button").addEventListener("click", () => {
      startSingleVideoDownload(v.url);
    });

    plDetailVids.appendChild(row);
  }
}

// ── Playlist download helpers ─────────────────────────────────────────────────

async function startSingleVideoDownload(url) {
  closeDetail();
  switchTab("download");

  const auth = getAuthForUrl(url);
  const req = {
    url,
    output_dir:      dirInput.value.trim() || ".",
    quality:         qualitySelect.value,
    filename:        "",
    subtitles:       chkSubs.checked,
    embed_thumbnail: chkThumb.checked,
    no_playlist:     true,
    auth_mode:       auth.authMode,
    auth_browser:    auth.authBrowser,
    cookies_file:    auth.cookiesFile,
  };

  try {
    const id = await invoke("start_download", { req });
    createDownloadItem(id, url);
    dlSection.classList.remove("hidden");
  } catch (e) {
    alert(`Failed to start download:\n${e}`);
  }
}

async function startPlaylistDownload(playlistId) {
  closeDetail();
  switchTab("download");

  const url = `https://www.youtube.com/playlist?list=${playlistId}`;
  const auth = getAuthForUrl(url);
  const req = {
    url,
    output_dir:      dirInput.value.trim() || ".",
    quality:         qualitySelect.value,
    filename:        "",
    subtitles:       chkSubs.checked,
    embed_thumbnail: chkThumb.checked,
    no_playlist:     false,
    auth_mode:       auth.authMode,
    auth_browser:    auth.authBrowser,
    cookies_file:    auth.cookiesFile,
  };

  try {
    const id = await invoke("start_download", { req });
    createDownloadItem(id, url);
    dlSection.classList.remove("hidden");
  } catch (e) {
    alert(`Failed to start download:\n${e}`);
  }
}

// ── UI helpers ────────────────────────────────────────────────────────────────

function formatDuration(secs) {
  if (!secs) return "";
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = secs % 60;
  if (h > 0) return `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
  return `${m}:${String(s).padStart(2, "0")}`;
}

function spinnerHtml(text) {
  return `<div class="pl-spinner">
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
      <path d="M21 12a9 9 0 1 1-6.219-8.56"/>
    </svg>
    ${escHtml(text)}
  </div>`;
}

function svgPlaylistIcon() {
  return `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
    <rect x="3" y="3" width="7" height="7"/><rect x="14" y="3" width="7" height="7"/>
    <rect x="14" y="14" width="7" height="7"/><rect x="3" y="14" width="7" height="7"/>
  </svg>`;
}

window.makePlaceholderThumb = function() {
  const d = document.createElement("div");
  d.className = "pl-card-thumb-placeholder";
  d.innerHTML = svgPlaylistIcon();
  return d;
};

// ══════════════════════════════════════════════════════════════════════════════
// ── Convert tab ───────────────────────────────────────────────────────────────
// ══════════════════════════════════════════════════════════════════════════════

const cvtDropzone   = document.getElementById("cvt-dropzone");
const cvtFileLabel  = document.getElementById("cvt-file-label");
const cvtDirInput   = document.getElementById("cvt-dir-input");
const cvtBrowseBtn  = document.getElementById("cvt-browse-btn");
const cvtBtn        = document.getElementById("cvt-btn");
const cvtJobSection = document.getElementById("cvt-jobs-section");
const cvtJobList    = document.getElementById("cvt-jobs-list");
const cvtClearBtn   = document.getElementById("cvt-clear-btn");
const cvtQualityRow = document.getElementById("cvt-quality-row");

const AUDIO_FORMATS = new Set(["mp3", "m4a", "aac", "opus", "flac", "wav"]);

let cvtState = { inputPath: "", format: "mp4", quality: "medium" };

// Sync output dir with download tab dir
Object.defineProperty(cvtState, "_dir", { value: "", writable: true });
function getCvtDir() { return cvtDirInput.value.trim(); }

// Format buttons
document.querySelectorAll(".cvt-fmt-btn").forEach(btn => {
  btn.addEventListener("click", () => {
    document.querySelectorAll(".cvt-fmt-btn").forEach(b => b.classList.remove("active"));
    btn.classList.add("active");
    cvtState.format = btn.dataset.fmt;
    cvtQualityRow.classList.toggle("hidden", AUDIO_FORMATS.has(cvtState.format));
    updateCvtBtn();
  });
});
// Default active: mp4
document.querySelector('.cvt-fmt-btn[data-fmt="mp4"]').classList.add("active");

// Quality buttons
document.querySelectorAll(".cvt-quality-btn").forEach(btn => {
  btn.addEventListener("click", () => {
    document.querySelectorAll(".cvt-quality-btn").forEach(b => b.classList.remove("active"));
    btn.classList.add("active");
    cvtState.quality = btn.dataset.q;
  });
});

// File picker (click on dropzone)
cvtDropzone.addEventListener("click", async () => {
  try {
    const { open } = window.__TAURI__?.dialog ?? {};
    const file = open
      ? await open({ multiple: false, filters: [{ name: "Media files", extensions: ["mp4","mkv","webm","mov","avi","mp3","m4a","aac","wav","flac","opus","gif","ts","ogg","wmv","flv","3gp"] }, { name: "All Files", extensions: ["*"] }] })
      : prompt("Path to input file:");
    if (file) setInputFile(file);
  } catch (e) { console.error(e); }
});

// Drag and drop
cvtDropzone.addEventListener("dragover", e => { e.preventDefault(); cvtDropzone.classList.add("drag-over"); });
cvtDropzone.addEventListener("dragleave", () => cvtDropzone.classList.remove("drag-over"));
cvtDropzone.addEventListener("drop", e => {
  e.preventDefault();
  cvtDropzone.classList.remove("drag-over");
  const file = e.dataTransfer?.files?.[0];
  if (file) setInputFile(file.path ?? file.name);
});

function setInputFile(path) {
  cvtState.inputPath = path;
  const name = path.split(/[\\/]/).pop();
  cvtFileLabel.textContent = name;
  cvtDropzone.classList.add("has-file");
  updateCvtBtn();
}

function updateCvtBtn() {
  cvtBtn.disabled = !cvtState.inputPath || !getCvtDir();
}

// Output directory
cvtBrowseBtn.addEventListener("click", async () => {
  const dir = await pickDirectory();
  if (dir) { cvtDirInput.value = dir; updateCvtBtn(); }
});

// Keep convert dir in sync when user picks dir on download tab
browseBtn.addEventListener("click", () => {
  setTimeout(() => {
    if (!cvtDirInput.value) cvtDirInput.value = dirInput.value;
  }, 100);
});

// Convert
cvtBtn.addEventListener("click", startConvert);

async function startConvert() {
  if (!cvtState.inputPath) return;
  const outputDir = getCvtDir();
  if (!outputDir) return alert("Please choose an output folder.");

  cvtBtn.disabled = true;
  setTimeout(() => updateCvtBtn(), 500);

  const req = {
    input_path:    cvtState.inputPath,
    output_dir:    outputDir,
    output_format: cvtState.format,
    video_quality: cvtState.quality,
  };

  try {
    const id = await invoke("convert_file", { req });
    const name = cvtState.inputPath.split(/[\\/]/).pop();
    createConvertItem(id, name, cvtState.format);
  } catch (e) {
    alert(`Failed to start conversion:\n${e}`);
  }
}

function createConvertItem(id, inputName, format) {
  cvtJobSection.classList.remove("hidden");

  const el = document.createElement("div");
  el.className = "dl-item";
  el.dataset.id = id;

  el.innerHTML = `
    <div class="dl-header">
      <div class="dl-status-icon running" data-icon>⟳</div>
      <div class="dl-info">
        <div class="dl-url">${escHtml(inputName)} → <strong>.${escHtml(format)}</strong></div>
        <div class="dl-status-text" data-status>Starting…</div>
      </div>
      <div class="dl-actions">
        <button class="dl-action-btn" data-open title="Open folder">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/>
          </svg>
        </button>
        <button class="dl-action-btn" data-remove title="Remove">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/>
          </svg>
        </button>
      </div>
    </div>
    <div class="dl-progress">
      <div class="progress-track"><div class="progress-fill indeterminate" data-fill></div></div>
      <div class="progress-pct" data-pct></div>
    </div>
    <div class="dl-log-toggle" data-log-toggle>▶ Show log</div>
    <div class="dl-log" data-log></div>`;

  el.querySelector("[data-open]").addEventListener("click", () => {
    invoke("open_folder", { path: getCvtDir() });
  });
  el.querySelector("[data-remove]").addEventListener("click", () => {
    el.remove();
    downloads.delete(id);
    if (cvtJobList.children.length === 0) cvtJobSection.classList.add("hidden");
  });
  const logToggle = el.querySelector("[data-log-toggle]");
  const logEl     = el.querySelector("[data-log]");
  logToggle.addEventListener("click", () => {
    const open = logEl.classList.toggle("open");
    logToggle.textContent = open ? "▼ Hide log" : "▶ Show log";
  });

  cvtJobList.prepend(el);

  // Re-use the existing downloads map + onProgress handler
  downloads.set(id, {
    el, logEl,
    progressFill: el.querySelector("[data-fill]"),
    progressPct:  el.querySelector("[data-pct]"),
    statusText:   el.querySelector("[data-status]"),
    statusIcon:   el.querySelector("[data-icon]"),
  });
}

cvtClearBtn.addEventListener("click", () => {
  for (const [id, item] of downloads) {
    if (!cvtJobList.contains(item.el)) continue;
    if (item.el.classList.contains("done") || item.el.classList.contains("error")) {
      item.el.remove();
      downloads.delete(id);
    }
  }
  if (cvtJobList.children.length === 0) cvtJobSection.classList.add("hidden");
});

// Pre-fill convert dir from download dir on init
function syncCvtDir() {
  if (!cvtDirInput.value && dirInput.value) cvtDirInput.value = dirInput.value;
}

// ── Bootstrap ─────────────────────────────────────────────────────────────────
initTauri().then(init).then(syncCvtDir);
