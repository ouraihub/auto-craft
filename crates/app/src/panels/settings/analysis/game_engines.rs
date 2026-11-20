//! Game-builder inventory shared by Home and Analysis settings.
use autocraft_core::{AnalysisSettings, Error, Result};
use ca_engines::game::{self, Inventory, Places};

pub(super) fn discover(settings: &AnalysisSettings) -> Result<Inventory> {
    let store = settings
        .engines
        .store_dir
        .clone()
        .ok_or_else(|| Error::corrupt("分析引擎存储位置未配置"))?;
    Ok(game::inventory(&Places::here(store)))
}
