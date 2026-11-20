//! Asset-generation secrets: masked inventory, explicit secret reads and revision-checked writes.

use autocraft_core::{AssetKeyService, AssetKeySummary, Error, Result, Secret};

use crate::{
    db::{self, Conn},
    mask::mask,
    store::SqliteSettingsStore,
};

fn current(conn: &Conn, provider: &str) -> Result<u64> {
    conn.row(
        "SELECT revision FROM asset_keys WHERE provider = ?1",
        (provider,),
    )?
    .map(|row| db::revision(&row, 0))
    .transpose()
    .map(|revision| revision.unwrap_or(0))
}

fn expect(conn: &Conn, provider: &str, revision: u64) -> Result<()> {
    if current(conn, provider)? != revision {
        return Err(Error::conflict("素材密钥已被修改，请重新读取后再保存。"));
    }
    Ok(())
}

impl AssetKeyService for SqliteSettingsStore {
    fn asset_key_summaries(&self) -> Result<Vec<AssetKeySummary>> {
        self.read(|conn| {
            conn.rows(
                "SELECT provider, key, revision FROM asset_keys ORDER BY provider",
                (),
            )?
            .iter()
            .map(|row| {
                Ok(AssetKeySummary {
                    provider: db::text(row, 0)?,
                    masked_key: mask(&db::text(row, 1)?),
                    revision: db::revision(row, 2)?,
                })
            })
            .collect()
        })
    }

    fn asset_key(&self, provider: &str) -> Result<Secret> {
        self.read(|conn| {
            let row = conn
                .row(
                    "SELECT key FROM asset_keys WHERE provider = ?1",
                    (provider,),
                )?
                .ok_or_else(|| Error::not_found("素材密钥尚未设置。"))?;
            Ok(Secret::new(db::text(&row, 0)?))
        })
    }

    fn asset_keys(&self) -> Result<Vec<(String, Secret)>> {
        self.read(|conn| {
            conn.rows("SELECT provider, key FROM asset_keys ORDER BY provider", ())?
                .iter()
                .map(|row| Ok((db::text(row, 0)?, Secret::new(db::text(row, 1)?))))
                .collect()
        })
    }

    fn set_asset_key(
        &self,
        provider: &str,
        key: Secret,
        expect_revision: u64,
    ) -> Result<AssetKeySummary> {
        if provider.trim().is_empty()
            || provider != provider.trim()
            || key.expose().trim().is_empty()
        {
            return Err(Error::invalid(
                "素材提供方与密钥不能为空，提供方不能包含首尾空白。",
            ));
        }
        self.write(|conn| {
            expect(conn, provider, expect_revision)?;
            let revision = expect_revision.checked_add(1)
                .and_then(|revision| i64::try_from(revision).ok())
                .ok_or_else(|| Error::conflict("素材密钥版本已超出范围。"))?;
            conn.execute(
                "INSERT INTO asset_keys (provider, key, revision) VALUES (?1, ?2, ?3) \
                 ON CONFLICT (provider) DO UPDATE SET key = excluded.key, revision = excluded.revision",
                (provider, key.expose(), revision),
            )?;
            Ok(AssetKeySummary {
                provider: provider.to_owned(), masked_key: mask(key.expose()), revision: revision as u64,
            })
        })
    }

    fn remove_asset_key(&self, provider: &str, expect_revision: u64) -> Result<()> {
        self.write(|conn| {
            expect(conn, provider, expect_revision)?;
            conn.execute("DELETE FROM asset_keys WHERE provider = ?1", (provider,))?;
            Ok(())
        })
    }
}
