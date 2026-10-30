// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! `vellum admin move-space`: moves a space to another shard (ADR 0040,
//! D175). Every replica must be stopped first, since running replicas keep
//! the old placement in memory.

use vellum_db::{ShardedIndex, ShardsConfig};
use vellum_kernel::{Error, ErrorKind, SpaceId};

/// Moves space `space` (its ID) to shard `to`, reading the shards from the
/// environment, and reports what moved.
///
/// # Errors
/// No shards configured, a malformed ID, an unknown shard, or a database
/// error.
pub async fn move_space(space: &str, to: &str) -> Result<Vec<String>, Error> {
    let config = ShardsConfig::from_env(|key| std::env::var(key).ok())
        .map_err(|e| Error::new(ErrorKind::InvalidInput, e.to_string()))?
        .ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidInput,
                "VELLUM_SHARDS is not set: there is only one database",
            )
        })?;
    let id: SpaceId = space.parse().map_err(|e| {
        Error::new(ErrorKind::InvalidInput, format!("bad space ID: {e}"))
    })?;
    let unavailable = |e: vellum_db::Error| {
        let kind = if matches!(e, vellum_db::Error::UnknownShard(_)) {
            ErrorKind::InvalidInput
        } else {
            ErrorKind::Unavailable
        };
        Error::new(kind, e.to_string())
    };
    let index = ShardedIndex::open(&config).await.map_err(unavailable)?;
    let object = id.into();
    let from = index
        .shard_for(object)
        .await
        .map_err(unavailable)?
        .name()
        .to_owned();
    let moved = index.move_space(object, to).await;
    index.close().await;
    let rows = moved.map_err(unavailable)?;
    Ok(vec![if rows == 0 && from.eq_ignore_ascii_case(to) {
        format!("space {space} is already on shard {from}")
    } else {
        format!("moved space {space} from shard {from} to {to} ({rows} rows)")
    }])
}
