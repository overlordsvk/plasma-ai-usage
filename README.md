# ai-usage

Shows used % of subscription limits for **Claude** (5h / weekly) and **OpenCode Go** (5h / weekly / monthly),
as a terminal command and as a KDE Plasma 6 panel widget.

```
ai-usage          # bars in the terminal
ai-usage --json   # what the widget reads
```

## How it gets the data

| Provider    | Credentials (read only)                         | Endpoint                                      |
|-------------|-------------------------------------------------|-----------------------------------------------|
| Claude      | `~/.claude/.credentials.json` (Claude Code OAuth) | `GET api.anthropic.com/api/oauth/usage`        |
| OpenCode Go | `~/.local/share/opencode/auth.json` (`opencode-go`) | `GET opencode.ai/zen/go/v1/usage`              |

Both endpoints are undocumented and may change. The Claude token is never refreshed by this tool
(that would rotate Claude Code's refresh token); if it has expired, run `claude` once.

Each check is two small GET requests and costs no tokens. The widget polls every 3 minutes.

## Install

```
./install.sh
```

Then right-click the panel → Add Widgets → "AI Usage". Panel view: `C` / `OC` with one thin bar per
window and the highest % (green < 60, orange < 85, red ≥ 85). Click for details and reset times.
