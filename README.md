# Conducktor

![Conducktor tray bar](readme_sample.png)

A macOS menu-bar app that shows your Claude Code usage — Session (5-hour),
Weekly, and Weekly (Fable 5) — as a compact tray title with reset
countdowns, e.g.:

```
S 5% (in 5h) · W 2% (in 3d 20h) · F 1%
```

Click the tray item for a detail menu, or open **Settings…** to choose
which meters to show and how often to poll.

## Download

- [**Conducktor_0.1.0_aarch64.dmg**](release/Conducktor_0.1.0_aarch64.dmg) — installer (drag to Applications)
- [**Conducktor.app.zip**](release/Conducktor.app.zip) — zipped app bundle

Both are unsigned Apple Silicon (arm64) builds — see [Build](#build) for
the Gatekeeper workaround on first launch.

## How it works

Conducktor doesn't call the public Claude API. It calls the same internal
endpoint Claude Code's own `/usage` screen uses
(`https://api.anthropic.com/api/oauth/usage`), authenticating with the
OAuth token Claude Code already has stored on your machine — read from
the macOS Keychain (`Claude Code-credentials`), falling back to
`~/.claude/.credentials.json`.

Conducktor never writes to, refreshes, or otherwise touches that
credential. Claude Code keeps it fresh on its own; Conducktor just
re-reads it when its cached copy looks stale or the endpoint returns 401.

**Requirements:**

- [Claude Code](https://claude.com/claude-code) installed and logged in
  (`claude` → sign in) — Conducktor has nothing to show otherwise.
- A Claude.ai Pro/Max/Team/Enterprise plan. Usage-limit data isn't
  meaningful for plain API-key usage.

## ⚠️ Unofficial endpoint

`/api/oauth/usage` is not part of Anthropic's public API — it's what
Claude Code's own UI calls internally, reverse-engineered for this app.
It can change shape or disappear without notice. All of the
wire-format parsing lives in one file
(`src-tauri/src/usage.rs`) so a shape change is a small, localized fix
rather than a rewrite.

The endpoint also rate-limits aggressively without a recognized
`User-Agent` header. Conducktor sends its own descriptive UA
(`Conducktor-Claude-Usage-Tray/x.y.z`, defined as a single constant in
`usage.rs`). If you start seeing persistent rate-limiting, community
reports indicate a Claude-Code-style UA string (e.g.
`Claude Code/2.1.207`) is treated more leniently — that's a one-line
change to the same constant.

## Menu bar format

- **S** = Session (5-hour rolling limit)
- **W** = Weekly (7-day limit)
- **F** = Weekly Fable 5 (7-day, model-scoped) — never shows a reset
  time, since it always resets alongside Weekly
- Countdowns: minutes under 1h, hours-only between 1h–24h, `Xd Yh`
  beyond that (the tray menu's detail lines add minutes to the 1h–24h
  case for a bit more precision)
- A trailing `⚠` means the last refresh failed and you're looking at
  the previous good values

If your plan doesn't include Fable 5 access, the `F` meter and its
Settings checkbox are simply absent/disabled — this isn't a bug.

## Settings

- Checkboxes for which of Session / Weekly / Weekly (Fable) to show
- Refresh interval: 30s / 1m / 2m / 5m (30s is the practical floor —
  the endpoint starts rate-limiting faster polling)
- **Refresh now** to force an immediate fetch
- Closing the Settings window hides it; it doesn't quit the app —
  quit from the tray menu

## Development

```bash
npm install
npm run tauri dev
```

The first Keychain read may show a macOS permission prompt — choose
**Always Allow**. This is expected behavior for any tool reading
another app's Keychain item.

## Build

```bash
npm run tauri build
```

Produces an unsigned `.app` / `.dmg` under `src-tauri/target/release/bundle/`.
Unsigned builds trigger Gatekeeper on first launch — right-click the
app → **Open** to bypass it once.
