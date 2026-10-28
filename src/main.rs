//! ai-usage: show how much of the Claude and OpenCode Go subscription limits is used.
//!
//! `ai-usage`        human-readable bars for the terminal
//! `ai-usage --json` machine-readable output for the Plasma widget

use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use std::{env, fs, path::PathBuf, time::Duration};

struct Window {
    label: &'static str,
    percent: f64,
    resets_at: Option<DateTime<Utc>>,
}

struct Provider {
    name: &'static str,
    result: Result<Vec<Window>, String>,
}

fn home() -> PathBuf {
    PathBuf::from(env::var("HOME").unwrap_or_default())
}

fn read_json(path: PathBuf) -> Result<Value, String> {
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout(Duration::from_secs(15)).build()
}

fn get(url: &str, headers: &[(&str, &str)]) -> Result<Value, String> {
    let mut req = agent().get(url);
    for (k, v) in headers {
        req = req.set(k, v);
    }
    match req.call() {
        Ok(resp) => resp.into_json().map_err(|e| e.to_string()),
        Err(ureq::Error::Status(401, _)) => Err("unauthorized (token expired?)".into()),
        Err(ureq::Error::Status(code, _)) => Err(format!("HTTP {code}")),
        Err(e) => Err(e.to_string()),
    }
}

fn parse_time(v: &Value) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(v.as_str()?).ok().map(|t| t.with_timezone(&Utc))
}

fn window(label: &'static str, v: &Value, pct_key: &str, reset_key: &str) -> Option<Window> {
    Some(Window {
        label,
        percent: v.get(pct_key)?.as_f64()?,
        resets_at: v.get(reset_key).and_then(parse_time),
    })
}

/// Claude: uses the OAuth token Claude Code keeps in ~/.claude/.credentials.json.
/// We never refresh it ourselves (that would rotate Claude Code's refresh token);
/// Claude Code refreshes it whenever it runs.
fn claude() -> Result<Vec<Window>, String> {
    let creds = read_json(home().join(".claude/.credentials.json"))?;
    let oauth = &creds["claudeAiOauth"];
    let token = oauth["accessToken"].as_str().ok_or("no accessToken")?;
    if let Some(exp) = oauth["expiresAt"].as_i64() {
        if exp < Utc::now().timestamp_millis() {
            return Err("token expired, run `claude` to refresh".into());
        }
    }
    let auth = format!("Bearer {token}");
    let v = get(
        "https://api.anthropic.com/api/oauth/usage",
        &[("Authorization", &auth), ("anthropic-beta", "oauth-2025-04-20")],
    )?;
    let mut out = Vec::new();
    for (key, label) in [("five_hour", "5h"), ("seven_day", "week"), ("seven_day_opus", "opus")] {
        if let Some(w) = window(label, &v[key], "utilization", "resets_at") {
            out.push(w);
        }
    }
    Ok(out)
}

/// OpenCode Go: uses the API key opencode stores in ~/.local/share/opencode/auth.json.
fn opencode_go() -> Result<Vec<Window>, String> {
    let auth = read_json(home().join(".local/share/opencode/auth.json"))?;
    let key = auth["opencode-go"]["key"].as_str().ok_or("no opencode-go key")?;
    let bearer = format!("Bearer {key}");
    let v = get("https://opencode.ai/zen/go/v1/usage", &[("Authorization", &bearer)])?;
    let u = &v["usage"];
    let mut out = Vec::new();
    for (key, label) in [("rolling", "5h"), ("weekly", "week"), ("monthly", "month")] {
        if let Some(w) = window(label, &u[key], "percent", "resetsAt") {
            out.push(w);
        }
    }
    Ok(out)
}

fn human_duration(secs: i64) -> String {
    let secs = secs.max(0);
    let (d, h, m) = (secs / 86400, secs % 86400 / 3600, secs % 3600 / 60);
    if d > 0 {
        format!("{d}d {h}h")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else {
        format!("{m}m")
    }
}

fn to_json(providers: &[Provider]) -> Value {
    let now = Utc::now();
    let list: Vec<Value> = providers
        .iter()
        .map(|p| match &p.result {
            Ok(ws) => json!({
                "name": p.name,
                "windows": ws.iter().map(|w| {
                    let secs = w.resets_at.map(|t| (t - now).num_seconds());
                    json!({
                        "label": w.label,
                        "percent": w.percent,
                        "resets_in": secs.map(human_duration),
                        "resets_at": w.resets_at.map(|t| t.to_rfc3339()),
                    })
                }).collect::<Vec<_>>(),
            }),
            Err(e) => json!({ "name": p.name, "error": e, "windows": [] }),
        })
        .collect();
    json!({ "updated": now.to_rfc3339(), "providers": list })
}

fn print_text(providers: &[Provider]) {
    let now = Utc::now();
    for p in providers {
        println!("{}", p.name);
        match &p.result {
            Err(e) => println!("  error: {e}"),
            Ok(ws) => {
                for w in ws {
                    let filled = ((w.percent / 5.0).round() as usize).min(20);
                    let reset = w
                        .resets_at
                        .map(|t| format!("  resets in {}", human_duration((t - now).num_seconds())))
                        .unwrap_or_default();
                    println!(
                        "  {:<6}{}{} {:>3.0}%{}",
                        w.label,
                        "█".repeat(filled),
                        "░".repeat(20 - filled),
                        w.percent,
                        reset
                    );
                }
            }
        }
    }
}

fn main() {
    let (c, o) = std::thread::scope(|s| {
        let c = s.spawn(claude);
        let o = s.spawn(opencode_go);
        (c.join(), o.join())
    });
    let providers = [
        Provider { name: "Claude", result: c.unwrap_or_else(|_| Err("panic".into())) },
        Provider { name: "OpenCode Go", result: o.unwrap_or_else(|_| Err("panic".into())) },
    ];
    if env::args().any(|a| a == "--json") {
        println!("{}", to_json(&providers));
    } else {
        print_text(&providers);
    }
}
