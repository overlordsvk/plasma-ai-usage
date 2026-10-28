//! ai-usage: show how much of your AI subscription limits (Claude, OpenCode Go) is used.

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
    --init            Write an example config to the default location
    -h, --help        Show this help

Without a config file, every provider whose credentials are found is shown.";

struct Window {
    label: &'static str,
    percent: f64,
    resets_at: Option<DateTime<Utc>>,
}

fn read_json(path: PathBuf) -> Result<Value, String> {
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn env_var(name: &str) -> Result<String, String> {
    env::var(name).map_err(|_| format!("environment variable {name} is not set"))
}

fn get(url: &str, headers: &[(&str, &str)]) -> Result<Value, String> {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(15)).build();
    let mut req = agent.get(url);
    for (k, v) in headers {
        req = req.set(k, v);
    }
    match req.call() {
        Ok(resp) => resp.into_json().map_err(|e| e.to_string()),
        Err(ureq::Error::Status(401, _)) => Err("unauthorized (token expired or invalid)".into()),
        Err(ureq::Error::Status(code, _)) => Err(format!("HTTP {code}")),
        Err(e) => Err(e.to_string()),
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
fn claude(cfg: &ProviderCfg) -> Result<Vec<Window>, String> {
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
fn opencode_go(cfg: &ProviderCfg) -> Result<Vec<Window>, String> {
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

fn fetch(cfg: &ProviderCfg) -> Result<Vec<Window>, String> {
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

type Results<'a> = Vec<(&'a ProviderCfg, Result<Vec<Window>, String>)>;

fn to_json(results: &Results) -> Value {
    let now = Utc::now();
    let list: Vec<Value> = results
        .iter()
        .map(|(cfg, res)| {
            let mut p = json!({ "name": cfg.name, "short": cfg.short, "windows": [] });
            match res {
                Ok(ws) => {
                    p["windows"] = ws
                        .iter()
                        .map(|w| {
                            let secs = w.resets_at.map(|t| (t - now).num_seconds());
                            json!({
                                "label": w.label,
                                "percent": w.percent,
                                "resets_in": secs.map(human_duration),
                                "resets_at": w.resets_at.map(|t| t.to_rfc3339()),
                            })
                        })
                        .collect();
                }
                Err(e) => p["error"] = json!(e),
            }
            p
        })
        .collect();
    json!({ "updated": now.to_rfc3339(), "providers": list })
}

fn print_text(results: &Results) {
    let now = Utc::now();
    for (cfg, res) in results {
        println!("{}", cfg.name);
        match res {
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
    let mut config_path = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--json" => json_out = true,
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

    let providers = match config::load(config_path) {
        Ok(p) => p,
        Err(e) => {
            if json_out {
                println!("{}", json!({ "error": e, "providers": [] }));
            } else {
                eprintln!("config error: {e}");
            }
            return ExitCode::FAILURE;
        }
    };
    if providers.is_empty() && !json_out {
        eprintln!("No providers found. Log in to Claude Code / opencode, or run `ai-usage --init`.");
    }

    let results: Results = std::thread::scope(|s| {
        let handles: Vec<_> = providers.iter().map(|p| (p, s.spawn(|| fetch(p)))).collect();
        handles
            .into_iter()
            .map(|(p, h)| (p, h.join().unwrap_or_else(|_| Err("internal error".into()))))
            .collect()
    });

    if json_out {
        println!("{}", to_json(&results));
    } else {
        print_text(&results);
    }
    ExitCode::SUCCESS
}
