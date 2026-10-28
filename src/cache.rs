//! Shared on-disk cache, so several widgets (panels on each screen, desktop)
//! hit the APIs once per period instead of once each. A file lock serialises
//! concurrent runs: the first refreshes, the rest wait and read its result.

use serde_json::{json, Value};
use std::{
    fs::{self, File},
    path::PathBuf,
};

pub struct Cache {
    path: PathBuf,
    pub data: Value,
    _lock: File,
}

fn dir() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::config::home().join(".cache"))
        .join("ai-usage")
}

impl Cache {
    /// Open the cache, holding an exclusive lock until dropped.
    pub fn open() -> Result<Cache, String> {
        let dir = dir();
        fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let lock = File::create(dir.join("lock")).map_err(|e| e.to_string())?;
        lock.lock().map_err(|e| e.to_string())?;
        let path = dir.join("state.json");
        let data = fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_else(|| json!({}));
        Ok(Cache { path, data, _lock: lock })
    }

    pub fn entry(&self, key: &str) -> &Value {
        &self.data[key]
    }

    pub fn set(&mut self, key: &str, v: Value) {
        self.data[key] = v;
    }

    pub fn save(&self) {
        let tmp = self.path.with_extension("tmp");
        if fs::write(&tmp, self.data.to_string()).is_ok() {
            let _ = fs::rename(&tmp, &self.path);
        }
    }
}
