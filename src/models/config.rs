use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Application configuration, loaded from TOML file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// How often to scan for processes (milliseconds)
    #[serde(default = "default_scan_interval_ms")]
    pub scan_interval_ms: u64,

    /// How long to keep memory history (seconds)
    #[serde(default = "default_history_retention_secs")]
    pub history_retention_secs: u64,

    /// Threshold before a process is considered idle (seconds)
    #[serde(default = "default_idle_threshold_secs")]
    pub idle_threshold_secs: u64,

    /// Grace period before downgrading stale processes (seconds)
    #[serde(default = "default_stale_grace_period_secs")]
    pub stale_grace_period_secs: u64,

    /// Grace period after main process exit before orphan cleanup (seconds)
    #[serde(default = "default_orphan_grace_period_secs")]
    pub orphan_grace_period_secs: u64,

    /// How often to check for memory leaks (seconds)
    #[serde(default = "default_leak_check_interval_secs")]
    pub leak_check_interval_secs: u64,

    /// Memory growth rate threshold to flag as a leak (bytes/sec)
    #[serde(default = "default_leak_growth_threshold_bytes_per_sec")]
    pub leak_growth_threshold_bytes_per_sec: f64,

    /// Whether to automatically clean up orphan processes
    #[serde(default)]
    pub auto_cleanup: bool,

    /// Log level filter (trace, debug, info, warn, error)
    #[serde(default = "default_log_level")]
    pub log_level: String,

    /// Custom IPC path (overrides platform default)
    #[serde(default)]
    pub ipc_path: Option<PathBuf>,
}

fn default_scan_interval_ms() -> u64 {
    1000
}
fn default_history_retention_secs() -> u64 {
    3600
}
fn default_idle_threshold_secs() -> u64 {
    300
}
fn default_stale_grace_period_secs() -> u64 {
    900
}
fn default_orphan_grace_period_secs() -> u64 {
    30
}
fn default_leak_check_interval_secs() -> u64 {
    10
}
fn default_leak_growth_threshold_bytes_per_sec() -> f64 {
    1_048_576.0 // 1 MB/s
}
fn default_log_level() -> String {
    "info".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            scan_interval_ms: default_scan_interval_ms(),
            history_retention_secs: default_history_retention_secs(),
            idle_threshold_secs: default_idle_threshold_secs(),
            stale_grace_period_secs: default_stale_grace_period_secs(),
            orphan_grace_period_secs: default_orphan_grace_period_secs(),
            leak_check_interval_secs: default_leak_check_interval_secs(),
            leak_growth_threshold_bytes_per_sec: default_leak_growth_threshold_bytes_per_sec(),
            log_level: default_log_level(),
            auto_cleanup: false,
            ipc_path: None,
        }
    }
}

impl Config {
    /// Load configuration from the platform-appropriate config directory.
    /// Falls back to defaults if the config file does not exist.
    pub fn load() -> anyhow::Result<Self> {
        let path = Self::config_path()?;
        if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            let config: Config = toml::from_str(&content)?;
            Ok(config)
        } else {
            Ok(Config::default())
        }
    }

    /// Return the platform-appropriate path for the config file.
    pub fn config_path() -> anyhow::Result<PathBuf> {
        let dirs = directories::ProjectDirs::from("dev", "clmem", "clmem")
            .ok_or_else(|| anyhow::anyhow!("Cannot determine config directory"))?;
        Ok(dirs.config_dir().join("clmem.toml"))
    }

    /// Save the current configuration to disk.
    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::config_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)?;
        std::fs::write(&path, content)?;
        Ok(())
    }
}
