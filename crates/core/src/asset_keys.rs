//! Revision-checked asset credentials; only summaries are suitable for display.
use crate::{Result, Secret};
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetKeySummary {
    pub provider: String,
    pub masked_key: String,
    pub revision: u64,
}
pub trait AssetKeyService: Send + Sync {
    fn asset_key_summaries(&self) -> Result<Vec<AssetKeySummary>>;
    fn asset_key(&self, provider: &str) -> Result<Secret>;
    fn asset_keys(&self) -> Result<Vec<(String, Secret)>>;
    /// Revision zero creates; stale revisions return CONFLICT.
    fn set_asset_key(
        &self,
        provider: &str,
        key: Secret,
        expect_revision: u64,
    ) -> Result<AssetKeySummary>;
    fn remove_asset_key(&self, provider: &str, expect_revision: u64) -> Result<()>;
}
