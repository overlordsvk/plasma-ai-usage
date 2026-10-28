//! ai-usage: show how much of your AI subscription limits (Claude, OpenCode Go) is used.

mod cache;
mod config;

use chrono::{DateTime, Utc};
use config::{Kind, ProviderCfg};
use serde_json::{json, Value};
use std::{env, fs, path::PathBuf, process::ExitCode, time::Duration};

const HELP: &str = "\
ai-usage - subscription usage for Claude and OpenCode Go

USAGE:
    ai-usage [OPTIONS]

OPTIONS:
    --json            Machine-readable output (used by the Plasma widget)
    --config <PATH>   Config file (default: ~/.config/ai-usage/config.toml)
    --fresh           Ignore the cache age (still at most once per 30 s, honours backoff)
    --init            Write an example config to the default location
    -h, --help        Show this help

Without a config file, every provider whose credentials are found is shown.";

struct Window {
    label: &'static str,
    percent: f64,
    resets_at: Option<DateTime<Utc>>,
}

struct FetchErr {
    msg: String,
    /// Some(seconds the server asked us to wait) on HTTP 429.
    rate_limited: Option<i64>,
}

impl From<String> for FetchErr {
    fn from(msg: String) -> Self {
        FetchErr { msg, rate_limited: None }
    }
}

impl From<&str> for FetchErr {
    fn from(msg: &str) -> Self {
        msg.to_string().into()
    }
}

fn read_json(path: PathBuf) -> Result<Value, String> {
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn env_var(name: &str) -> Result<String, String> {
    env::var(name).map_err(|_| format!("environment variable {name} is not set"))
}

fn get(url: &str, headers: &[(&str, &str)]) -> Result<Value, FetchErr> {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(15)).build();
    let mut req = agent.get(url);
    for (k, v) in headers {
        req = req.set(k, v);
    }
    match req.call() {
        Ok(resp) => resp.into_json().map_err(|e| e.to_string().into()),
        Err(ureq::Error::Status(401, _)) => Err("unauthorized (token expired or invalid)".into()),
        Err(ureq::Error::Status(429, resp)) => Err(FetchErr {
            msg: "rate limited (HTTP 429)".into(),
            rate_limited: Some(resp.header("retry-after").and_then(|v| v.parse().ok()).unwrap_or(0)),
        }),
        Err(ureq::Error::Status(code, _)) => Err(format!("HTTP {code}").into()),
        Err(e) => Err(e.to_string().into()),
    }
}

fn parse_time(v: &Value) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(v.as_str()?).ok().map(|t| t.with_timezone(&Utc))
}

fn windows(v: &Value, keys: &[(&str, &'static str)], pct_key: &str, reset_key: &str) -> Vec<Window> {
    keys.iter()
        .filter_map(|(key, label)| {
            let w = &v[key];
            Some(Window {
                label,
                percent: w.get(pct_key)?.as_f64()?,
                resets_at: w.get(reset_key).and_then(parse_time),
            })
        })
        .collect()
}

/// Claude: OAuth token from Claude Code's credentials file. Never refreshed here,
/// since that would rotate Claude Code's refresh token; running `claude` refreshes it.
fn claude(cfg: &ProviderCfg) -> Result<Vec<Window>, FetchErr> {
    let token = match cfg.str("token_env") {
        Some(var) => env_var(var)?,
        None => {
            let creds = read_json(cfg.path("credentials_file").unwrap_or_else(config::claude_credentials))?;
            let oauth = &creds["claudeAiOauth"];
            if oauth["expiresAt"].as_i64().is_some_and(|exp| exp < Utc::now().timestamp_millis()) {
                return Err("token expired, run `claude` to refresh".into());
            }
            oauth["accessToken"].as_str().ok_or("no accessToken in credentials file")?.to_string()
        }
    };
    let auth = format!("Bearer {token}");
    let v = get(
        "https://api.anthropic.com/api/oauth/usage",
        &[("Authorization", &auth), ("anthropic-beta", "oauth-2025-04-20")],
    )?;
    Ok(windows(
        &v,
        &[("five_hour", "5h"), ("seven_day", "week"), ("seven_day_opus", "opus")],
        "utilization",
        "resets_at",
    ))
}

/// OpenCode Go: API key from config, env, or opencode's auth.json.
fn opencode_go(cfg: &ProviderCfg) -> Result<Vec<Window>, FetchErr> {
    let key = if let Some(k) = cfg.str("api_key") {
        k.to_string()
    } else if let Some(var) = cfg.str("api_key_env") {
        env_var(var)?
    } else {
        let path = cfg.path("auth_file").unwrap_or_else(config::opencode_auth);
        let entry = cfg.str("auth_entry").unwrap_or("opencode-go");
        let auth = read_json(path.clone())?;
        auth[entry]["key"]
            .as_str()
            .ok_or_else(|| format!("no \"{entry}\" key in {}, run `opencode auth login`", path.display()))?
            .to_string()
    };
    let bearer = format!("Bearer {key}");
    let v = get("https://opencode.ai/zen/go/v1/usage", &[("Authorization", &bearer)])?;
    Ok(windows(
        &v["usage"],
        &[("rolling", "5h"), ("weekly", "week"), ("monthly", "month")],
        "percent",
        "resetsAt",
    ))
}

fn fetch(cfg: &ProviderCfg) -> Result<Vec<Window>, FetchErr> {
    let mut ws = match cfg.kind {
        Kind::Claude => claude(cfg)?,
        Kind::OpenCodeGo => opencode_go(cfg)?,
    };
    if let Some(keep) = &cfg.windows {
        ws.retain(|w| keep.iter().any(|k| k == w.label));
    }
    Ok(ws)
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

const MIN_FRESH_SECONDS: i64 = 30;
const BACKOFF_BASE: i64 = 300;
const BACKOFF_MAX: i64 = 1800;

fn window_json(w: &Window) -> Value {
    json!({ "label": w.label, "percent": w.percent, "resets_at": w.resets_at.map(|t| t.to_rfc3339()) })
}

/// Decide from the cached entry whether to call the API, and return the new entry.
/// Entry fields: checked (last attempt), fetched (last success), windows,
/// error, backoff_until, strikes (consecutive 429s).
fn refresh(cfg: &ProviderCfg, old: &Value, ttl: i64, fresh: bool) -> Option<Value> {
    let now = Utc::now().timestamp();
    let checked = old["checked"].as_i64().unwrap_or(0);
    let min_age = if fresh { MIN_FRESH_SECONDS.min(ttl) } else { ttl };
    if now - checked < min_age || now < old["backoff_until"].as_i64().unwrap_or(0) {
        return None;
    }
    let mut e = if old.is_object() { old.clone() } else { json!({}) };
    e["checked"] = json!(now);
    match fetch(cfg) {
        Ok(ws) => {
            e["fetched"] = json!(now);
            e["windows"] = ws.iter().map(window_json).collect();
            e["error"] = Value::Null;
            e["backoff_until"] = Value::Null;
            e["strikes"] = json!(0);
        }
        Err(err) => {
            e["error"] = json!(err.msg);
            if let Some(retry_after) = err.rate_limited {
                let strikes = e["strikes"].as_i64().unwrap_or(0) + 1;
                let backoff = (BACKOFF_BASE << (strikes - 1).min(4)).min(BACKOFF_MAX).max(retry_after);
                e["strikes"] = json!(strikes);
                e["backoff_until"] = json!(now + backoff);
            }
        }
    }
    Some(e)
}

/// Provider entries as shown: cached windows plus any current error.
fn view(cfg: &ProviderCfg, e: &Value, now: DateTime<Utc>) -> Value {
    let windows: Vec<Value> = e["windows"]
        .as_array()
        .map(|ws| {
            ws.iter()
                .map(|w| {
                    let mut w = w.clone();
                    let secs = parse_time(&w["resets_at"]).map(|t| (t - now).num_seconds());
                    w["resets_in"] = json!(secs.map(human_duration));
                    w
                })
                .collect()
        })
        .unwrap_or_default();
    let mut p = json!({ "name": cfg.name, "short": cfg.short, "windows": windows });
    if let Some(err) = e["error"].as_str() {
        let mut msg = err.to_string();
        if let Some(until) = e["backoff_until"].as_i64() {
            msg += &format!(", retrying in {}", human_duration(until - now.timestamp()));
        }
        if let Some(t) = e["fetched"].as_i64() {
            msg += &format!(" (showing data from {} ago)", human_duration(now.timestamp() - t));
        }
        p["error"] = json!(msg);
        p["stale"] = json!(e["fetched"].is_i64());
    }
    p
}

fn print_text(providers: &[Value]) {
    for p in providers {
        println!("{}", p["name"].as_str().unwrap_or(""));
        if let Some(e) = p["error"].as_str() {
            println!("  error: {e}");
        }
        for w in p["windows"].as_array().into_iter().flatten() {
            let pct = w["percent"].as_f64().unwrap_or(0.0);
            let filled = ((pct / 5.0).round() as usize).min(20);
            let reset = w["resets_in"].as_str().map(|r| format!("  resets in {r}")).unwrap_or_default();
            println!(
                "  {:<6}{}{} {:>3.0}%{}",
                w["label"].as_str().unwrap_or(""),
                "█".repeat(filled),
                "░".repeat(20 - filled),
                pct,
                reset
            );
        }
    }
}

fn init() -> Result<PathBuf, String> {
    let path = config::default_path();
    if path.exists() {
        return Err(format!("{} already exists", path.display()));
    }
    fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    fs::write(&path, config::EXAMPLE).map_err(|e| e.to_string())?;
    Ok(path)
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut json_out = false;
    let mut fresh = false;
    let mut config_path = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--json" => json_out = true,
            "--fresh" => fresh = true,
            "--config" => match it.next() {
                Some(p) => config_path = Some(config::expand(p)),
                None => {
                    eprintln!("--config needs a path");
                    return ExitCode::FAILURE;
                }
            },
            "--init" => {
                return match init() {
                    Ok(p) => {
                        println!("wrote {}", p.display());
                        ExitCode::SUCCESS
                    }
                    Err(e) => {
                        eprintln!("{e}");
                        ExitCode::FAILURE
                    }
                }
            }
            "-h" | "--help" => {
                println!("{HELP}");
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("unknown argument: {other}\n\n{HELP}");
                return ExitCode::FAILURE;
            }
        }
    }

    let cfg = match config::load(config_path) {
        Ok(c) => c,
        Err(e) => {
            if json_out {
                println!("{}", json!({ "error": e, "providers": [] }));
            } else {
                eprintln!("config error: {e}");
            }
            return ExitCode::FAILURE;
        }
    };
    if cfg.providers.is_empty() && !json_out {
        eprintln!("No providers found. Log in to Claude Code / opencode, or run `ai-usage --init`.");
    }

    let mut cache = match cache::Cache::open() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("cache: {e}");
            return ExitCode::FAILURE;
        }
    };
    let keys: Vec<String> = cfg.providers.iter().map(|p| format!("{}:{}", p.kind.id(), p.name)).collect();
    let updates: Vec<Option<Value>> = std::thread::scope(|s| {
        let handles: Vec<_> = cfg
            .providers
            .iter()
            .zip(&keys)
            .map(|(p, k)| {
                let old = cache.entry(k);
                s.spawn(move || refresh(p, old, cfg.cache_seconds, fresh))
            })
            .collect();
        handles.into_iter().map(|h| h.join().ok().flatten()).collect()
    });
    let changed = updates.iter().any(Option::is_some);
    for (k, u) in keys.iter().zip(updates) {
        if let Some(u) = u {
            cache.set(k, u);
        }
    }
    if changed {
        cache.save();
    }

    let now = Utc::now();
    let providers: Vec<Value> = cfg.providers.iter().zip(&keys).map(|(p, k)| view(p, cache.entry(k), now)).collect();
    if json_out {
        println!("{}", json!({ "updated": now.to_rfc3339(), "providers": providers }));
    } else {
        print_text(&providers);
    }
    ExitCode::SUCCESS
}
