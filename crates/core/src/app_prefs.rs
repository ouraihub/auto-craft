//! Host-side preferences that are not domain settings: the log
//! page's switch, the log level and the appearance. Stored in `autocraft.db` `app_prefs`.

use serde::{Deserialize, Serialize};

use crate::{AppearancePrefs, Result};

/// The levels the log page offers; `debug` shows the full call chain.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    #[default]
    Info,
    Debug,
}

impl LogLevel {
    /// The `EnvFilter` directive.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Debug => "debug",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPrefs {
    /// 显示后台日志.
    pub logs_visible: bool,
    pub log_level: LogLevel,
    pub appearance: AppearancePrefs,
    /// UI language preference; fresh installations use Simplified Chinese.
    pub locale: String,
    /// Whether the first-run data notice has been acknowledged.
    pub onboarding_done: bool,
}

impl Default for AppPrefs {
    fn default() -> Self {
        Self {
            logs_visible: false,
            log_level: LogLevel::default(),
            appearance: AppearancePrefs::default(),
            locale: "zh-CN".to_owned(),
            onboarding_done: false,
        }
    }
}

pub trait AppPrefService: Send + Sync {
    fn app_prefs(&self) -> Result<AppPrefs>;
    fn set_logs_visible(&self, visible: bool) -> Result<AppPrefs>;
    fn set_log_level(&self, level: LogLevel) -> Result<AppPrefs>;
    /// Replaces the whole appearance value; the stored value is what `app_prefs` returns next.
    fn set_appearance(&self, appearance: &AppearancePrefs) -> Result<AppPrefs>;
    /// Stores the requested UI locale without changing unrelated preferences.
    fn set_locale(&self, locale: &str) -> Result<AppPrefs>;
    /// Stores acknowledgement of the first-run data notice.
    fn set_onboarding_done(&self, done: bool) -> Result<AppPrefs>;
}
