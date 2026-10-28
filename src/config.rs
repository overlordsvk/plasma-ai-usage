//! Provider configuration: ~/.config/ai-usage/config.toml, or auto-detection when absent.

use std::{env, fs, path::PathBuf};
use toml::{Table, Value};

pub const EXAMPLE: &str = include_str!("../config.example.toml");

#[derive(Clone, Copy)]
pub enum Kind {
    Claude,
    OpenCodeGo,
}

impl Kind {
    pub fn id(self) -> &'static str {
        match self {
            Kind::Claude => "claude",
            Kind::OpenCodeGo => "opencode-go",
        }
    }

    fn parse(s: &str) -> Result<Kind, String> {
        match s {
            "claude" => Ok(Kind::Claude),
            "opencode-go" => Ok(Kind::OpenCodeGo),
            other => Err(format!("unknown provider type \"{other}\" (expected claude or opencode-go)")),
        }
    }

    fn defaults(self) -> (&'static str, &'static str) {
        match self {
            Kind::Claude => ("Claude", "C"),
            Kind::OpenCodeGo => ("OpenCode Go", "OC"),
        }
    }
}

pub struct ProviderCfg {
    pub kind: Kind,
    pub name: String,
    pub short: String,
    pub windows: Option<Vec<String>>,
    opts: Table,
}

impl ProviderCfg {
    fn new(kind: Kind, opts: Table) -> Self {
        let (name, short) = kind.defaults();
        let s = |k: &str, d: &str| opts.get(k).and_then(Value::as_str).unwrap_or(d).to_string();
        ProviderCfg {
            kind,
            name: s("name", name),
            short: s("short", short),
            windows: opts.get("windows").and_then(Value::as_array).map(|a| {
                a.iter().filter_map(Value::as_str).map(String::from).collect()
            }),
            opts,
        }
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        self.opts.get(key).and_then(Value::as_str)
    }

    pub fn path(&self, key: &str) -> Option<PathBuf> {
        self.str(key).map(expand)
    }
}

pub fn home() -> PathBuf {
    PathBuf::from(env::var("HOME").unwrap_or_default())
}

fn xdg(var: &str, fallback: &str) -> PathBuf {
    env::var_os(var).map(PathBuf::from).unwrap_or_else(|| home().join(fallback))
}

pub fn expand(p: &str) -> PathBuf {
    match p.strip_prefix("~/") {
        Some(rest) => home().join(rest),
        None => PathBuf::from(p),
    }
}

pub fn default_path() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config").join("ai-usage/config.toml")
}

pub fn claude_credentials() -> PathBuf {
    env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|| home().join(".claude")).join(".credentials.json")
}

pub fn opencode_auth() -> PathBuf {
    xdg("XDG_DATA_HOME", ".local/share").join("opencode/auth.json")
}

pub struct Config {
    /// Minimum seconds between API calls per provider, shared by all widgets.
    pub cache_seconds: i64,
    pub providers: Vec<ProviderCfg>,
}

const DEFAULT_CACHE_SECONDS: i64 = 240;

/// Load providers from `path`. A missing file at the default location means
/// auto-detect: every provider whose standard credential file exists.
pub fn load(path: Option<PathBuf>) -> Result<Config, String> {
    let explicit = path.is_some();
    let path = path.unwrap_or_else(default_path);
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if !explicit && e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Config { cache_seconds: DEFAULT_CACHE_SECONDS, providers: autodetect() })
        }
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let table: Table = text.parse().map_err(|e| format!("{}: {e}", path.display()))?;
    let cache_seconds = table.get("cache_seconds").and_then(Value::as_integer).unwrap_or(DEFAULT_CACHE_SECONDS);
    let Some(list) = table.get("provider").and_then(Value::as_array) else {
        return Err(format!("{}: no [[provider]] entries", path.display()));
    };
    let mut out = Vec::new();
    for v in list {
        let opts = v.as_table().ok_or("[[provider]] must be a table")?.clone();
        if opts.get("enabled").and_then(Value::as_bool) == Some(false) {
            continue;
        }
        let kind = Kind::parse(opts.get("type").and_then(Value::as_str).ok_or("provider without type")?)?;
        out.push(ProviderCfg::new(kind, opts));
    }
    Ok(Config { cache_seconds, providers: out })
}

fn autodetect() -> Vec<ProviderCfg> {
    let mut out = Vec::new();
    if claude_credentials().exists() {
        out.push(ProviderCfg::new(Kind::Claude, Table::new()));
    }
    if opencode_auth().exists() {
        out.push(ProviderCfg::new(Kind::OpenCodeGo, Table::new()));
    }
    out
}
