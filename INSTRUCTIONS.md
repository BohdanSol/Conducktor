# Claude Code Usage Menu Bar App — Implementation Instructions

Step-by-step instructions for a coding agent to build a macOS menu bar (tray) app with Tauri 2 that shows Claude Code token-usage percentages and reset countdowns.

## Product summary

A macOS menu-bar-only app (no dock icon, no main window). The menu bar shows a compact text indicator like:

```
S 5% (in 5h) · W 2% (in 3d 20h) · F 1%
```

- **S** = Session (5-hour rolling limit), **W** = Weekly (7-day limit), **F** = Weekly Fable (7-day Fable 5 limit).
- **F never shows a reset time** — it resets at the same moment as W.
- Clicking the tray icon opens a menu with: current usage details, "Settings…", "Refresh now", "Quit".
- A small Settings window lets the user choose which meters to show (checkboxes: Session / Weekly / Weekly Fable) and the refresh interval.

**Polling is required.** There is no push mechanism — the app must poll an HTTP endpoint on an interval, so the refresh-interval setting stays in the design (default 60s, minimum 30s — the endpoint rate-limits aggressive polling).

---

## Data source (the core of the app)

### Where the data comes from

Claude Code's `/usage` screen is backed by an internal OAuth endpoint. The app calls it directly using the OAuth token Claude Code already stores on the machine.

```
GET https://api.anthropic.com/api/oauth/usage
Headers:
  Authorization: Bearer <accessToken>
  anthropic-beta: oauth-2025-04-20
  User-Agent: Claude-Code-Usage-Menubar/1.0
```

> ⚠️ **User-Agent is mandatory.** Without a User-Agent header the endpoint returns 429 aggressively even at 60s intervals. Community reports say a `Claude Code/<version>` UA works reliably (e.g. `Claude Code/2.1.207`); try the app's own UA first, and if 429s persist, fall back to a Claude Code-style UA. Make the UA string a single constant so it's easy to change.

> ⚠️ **Unofficial endpoint.** This is not in the public API docs and may change. Isolate all parsing in one module and fail soft (show `--` in the tray) on unknown response shapes.

### Response shape (success 200)

```json
{
  "five_hour": { "utilization": 15.0, "resets_at": 1744372800 },
  "seven_day": { "utilization": 8.0, "resets_at": 1744804800 },
  "seven_day_opus": { "utilization": 5.0, "resets_at": 1744804800 },
  "seven_day_sonnet": { "utilization": 6.0, "resets_at": 1744804800 },
  "seven_day_fable": { "utilization": 12.0, "resets_at": 1744804800 }
}
```

- `utilization` is a percentage 0–100 (may be fractional — round to nearest integer for display).
- `resets_at` is a Unix timestamp in **seconds**.
- Mapping to the UI: `five_hour` → **S**, `seven_day` → **W**, `seven_day_fable` → **F**.
- **Model-specific buckets may be absent** depending on the user's plan. Treat every field as optional. If `seven_day_fable` is missing, hide F (and gray out / annotate its checkbox in Settings).
- Only Claude.ai subscription users (Pro/Max/Team/Enterprise) have these limits; API-key users won't get useful data — show a friendly error state.

### Where the OAuth token comes from (macOS)

Claude Code stores credentials in the macOS Keychain as a generic password with service name `Claude Code-credentials`. Read it by shelling out to:

```bash
security find-generic-password -s "Claude Code-credentials" -w
```

The output is JSON:

```json
{
  "claudeAiOauth": {
    "accessToken": "…",
    "refreshToken": "…",
    "expiresAt": 1775212290694,
    "subscriptionType": "pro"
  }
}
```

- Use `claudeAiOauth.accessToken` as the Bearer token. `expiresAt` is in **milliseconds**.
- Access tokens live ~60 minutes. **Do not implement token refresh.** Claude Code refreshes the token itself and writes it back to the Keychain. On a 401 from the usage endpoint, simply re-read the Keychain and retry once.
- Fallback: if the Keychain item doesn't exist, try `~/.claude/.credentials.json` (same JSON shape).
- Cache the token in memory; re-read the Keychain only when (a) the cached token's `expiresAt` has passed, or (b) a request returned 401. Reading the Keychain can trigger a macOS permission prompt on first access — minimizing reads minimizes prompts. The app binary should be signed so macOS remembers the "Always Allow" choice.
- If neither source yields a token: tray shows `Claude: no login` and the menu explains "Run `claude` and log in, then click Refresh".

---

## Step-by-step implementation plan

### Step 1 — Scaffold the Tauri 2 project

1. Create a Tauri 2 app (`npm create tauri-app@latest`) with vanilla TypeScript or React (agent's choice; the UI is one small settings page — vanilla TS + a few styled inputs is enough).
2. Target macOS. In `tauri.conf.json`:
   - Set the app identifier (e.g. `com.solovei.conducktor`).
   - Configure the main window to **not** show on startup (`"visible": false`) — it will serve as the Settings window, shown on demand.
3. Make it a menu-bar-only app: set `ActivationPolicy::Accessory` in Rust setup (`app.set_activation_policy(tauri::ActivationPolicy::Accessory)`) so there is no Dock icon.
4. Add Rust dependencies: `reqwest` (with `json` feature), `serde`/`serde_json`, `tokio` (Tauri already provides an async runtime).
5. Add Tauri plugins: `tauri-plugin-store` (settings persistence), `tauri-plugin-shell` **not** needed — call `security` via `std::process::Command` from Rust directly.

### Step 2 — Credentials module (Rust)

Create `src-tauri/src/credentials.rs`:

1. `fn read_credentials() -> Result<OauthCreds, CredError>`:
   - Run `security find-generic-password -s "Claude Code-credentials" -w`; on non-zero exit, fall back to reading `~/.claude/.credentials.json`.
   - Parse JSON into a struct: `accessToken: String`, `expiresAt: i64` (ms). Unknown fields ignored (`#[serde(default)]` / non-exhaustive parsing).
2. Keep a cached copy (e.g. in a `tokio::sync::Mutex<Option<OauthCreds>>` in Tauri managed state). Expose `fn get_token(force_refresh: bool)` that returns the cached token unless expired or `force_refresh`.
3. Error enum: `NotLoggedIn`, `KeychainDenied`, `ParseError` — each maps to a distinct tray/menu message.

### Step 3 — Usage-fetch module (Rust)

Create `src-tauri/src/usage.rs`:

1. Define serde structs:

   ```rust
   #[derive(Deserialize)]
   struct Bucket { utilization: f64, resets_at: Option<i64> } // seconds

   #[derive(Deserialize)]
   struct UsageResponse {
       five_hour: Option<Bucket>,
       seven_day: Option<Bucket>,
       seven_day_fable: Option<Bucket>,
       // ignore everything else
   }
   ```

2. `async fn fetch_usage(token: &str) -> Result<UsageResponse, UsageError>` — GET the endpoint with the three headers above, 10s timeout.
3. Handle statuses:
   - **200** → parse and return.
   - **401** → return `UsageError::Unauthorized` (caller re-reads Keychain, retries once; if still 401 → "no login" state).
   - **429** → return `UsageError::RateLimited`; caller backs off (skip the next 2 poll cycles / double the interval temporarily).
   - Other / network error → `UsageError::Network`; keep showing the **last good data** with a stale marker (see Step 5).

### Step 4 — Polling loop + state (Rust)

1. On app setup, spawn a `tokio` task that loops: read settings → get token → fetch usage → update tray → `sleep(interval)`.
2. The loop must react to settings changes (interval or visible meters). Simplest approach: use a `tokio::sync::watch` or `Notify` channel; the `save_settings` command and the "Refresh now" menu item both trigger an immediate re-poll.
3. Store the latest `UsageResponse` + fetch timestamp in managed state so the settings window / tray menu can display details without refetching.

### Step 5 — Tray indicator (Rust)

1. Build a `TrayIcon` with `tauri::tray::TrayIconBuilder`. On macOS, the **title** (text next to the icon) is what displays the indicator: `tray.set_title(Some("S 5% (in 5h) · W 2% (in 3d 20h) · F 1%"))`. Use a small template icon (or icon-less, title only).
2. Formatting rules:
   - Round `utilization` to whole percent: `S 5%`.
   - Reset countdown from `resets_at` (seconds) minus now:
     - `< 1h` → `(in 42m)`
     - `< 24h` → `(in 5h)` (hours only, drop minutes to keep the bar short; include minutes only under 1h)
     - `>= 24h` → `(in 3d 20h)`
   - **F shows no reset time** — it resets with W.
   - Only include meters enabled in settings _and_ present in the response. Join segments with `·`.
   - Recompute the title every poll; also recompute countdowns every 60s between polls (cheap timer) so `(in 42m)` stays fresh without hitting the network.
3. Error/edge states for the title:
   - No login → `Claude: login required`
   - Rate limited / network error with previous data → keep last values, append `⚠` (e.g. `S 5% (in 5h) ⚠`).
   - No data at all → `Claude: …`
4. Tray menu (click):
   - One disabled info line per meter with full detail, e.g. `Session: 5% — resets in 4h 55m`, `Weekly: 2% — resets in 3d 20h`, `Weekly (Fable): 1%`.
   - `Last updated: 12:03` line.
   - `Refresh now`, `Settings…`, separator, `Quit`.

### Step 6 — Settings (persistence + window)

1. Persist with `tauri-plugin-store` (`settings.json`):
   ```json
   {
     "show_session": true,
     "show_weekly": true,
     "show_fable": true,
     "refresh_interval_secs": 60
   }
   ```
2. Tauri commands: `get_settings`, `save_settings(settings)`, `get_latest_usage`, `refresh_now`.
3. Settings window (the pre-created hidden window; `Settings…` menu item shows and focuses it; closing hides instead of destroying — intercept `WindowEvent::CloseRequested`, call `prevent_close()` + `hide()`):
   - Three checkboxes: **Session**, **Weekly**, **Weekly Fable**. If a bucket was absent in the last response, show the checkbox disabled with hint "not available on your plan".
   - Refresh interval: dropdown or number input, options 30s / 60s / 2m / 5m. **Clamp minimum to 30s** and note in the UI that lower values get rate-limited.
   - A small status area: last fetch time, current values, and any error message (e.g. "Not logged in — run `claude` in a terminal and sign in").
   - Save applies immediately (write store, notify the polling loop).

### Step 7 — Polish & packaging

1. `Cmd+Q` / Quit menu item exits the app; there's no window-close-quits behavior.
2. Autostart (optional, nice-to-have): add `tauri-plugin-autostart` with a "Launch at login" checkbox in Settings.
3. Build: `npm run tauri build`. For local personal use an unsigned build is fine (Keychain prompt: click "Always Allow"). Document in the README that Gatekeeper may require right-click → Open on first launch.
4. README: what it does, the unofficial-endpoint disclaimer, requirement that Claude Code is installed and logged in.

---

## Acceptance checklist

- [ ] Menu bar shows e.g. `S 5% (in 5h) · W 2% (in 3d 20h) · F 1%` matching enabled settings.
- [ ] F never shows a reset time; S and W countdowns use `Xm` / `Xh` / `Xd Yh` formats.
- [ ] Unchecking a meter in Settings removes it from the bar immediately.
- [ ] Changing the refresh interval takes effect without restart; "Refresh now" forces a fetch.
- [ ] 401 → token re-read from Keychain and one retry before showing "login required".
- [ ] 429/network failure → last good values kept with `⚠`, backoff applied; no crash.
- [ ] Missing `seven_day_fable` in the response → F hidden, checkbox disabled with explanation.
- [ ] Not logged in to Claude Code → clear "login required" state in tray and Settings.
- [ ] No Dock icon; app lives only in the menu bar; countdown text stays fresh between polls.

## Known risks (do not "fix" these by design changes)

- `https://api.anthropic.com/api/oauth/usage` is an internal endpoint used by Claude Code's `/usage`; it can change without notice. Keep parsing tolerant and centralized.
- The `User-Agent` requirement is empirical (endpoint 429s without a recognized UA). Keep it configurable via a constant.
- Keychain access may prompt the user once; that's expected macOS behavior.
