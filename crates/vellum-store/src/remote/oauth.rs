// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Bearer tokens for the cloud-drive adapters.
//!
//! The user connects a drive once through the identity service's `OAuth`
//! flow; the store keeps only the refresh token and trades it for
//! short-lived access tokens here.

use std::{
    fmt,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use serde::Deserialize;
use tokio::sync::Mutex;

use super::{Http, Secret};
use crate::error::{StoreError, StoreResult};

/// Renew this long before the access token expires.
const EARLY: Duration = Duration::from_mins(1);

/// Supplies `OAuth` bearer tokens to an adapter.
#[async_trait]
pub trait TokenSource: Send + Sync + fmt::Debug {
    /// A currently valid access token.
    async fn token(&self) -> StoreResult<Secret>;

    /// Drops a cached token the server rejected, so the next
    /// [`TokenSource::token`] fetches a new one.
    async fn invalidate(&self) {}
}

/// A fixed token: for tests and for personal access tokens.
#[derive(Debug, Clone)]
pub struct StaticToken(Secret);

impl StaticToken {
    /// Wraps `token`.
    #[must_use]
    pub fn new(token: impl Into<String>) -> Self {
        Self(Secret::new(token))
    }
}

#[async_trait]
impl TokenSource for StaticToken {
    async fn token(&self) -> StoreResult<Secret> {
        Ok(self.0.clone())
    }
}

/// The `OAuth` 2.0 refresh-token grant (RFC 6749 §6), as Google, Dropbox and
/// Microsoft all implement it, with the access token cached until shortly
/// before it expires.
#[derive(Debug)]
pub struct RefreshToken {
    http: Http,
    token_url: String,
    client_id: String,
    client_secret: Option<Secret>,
    grant: Secret,
    cached: Mutex<Option<(Secret, Instant)>>,
}

#[derive(Deserialize)]
struct Grant {
    access_token: String,
    expires_in: Option<u64>,
}

impl RefreshToken {
    /// A source that posts to `token_url`.
    ///
    /// # Errors
    /// Returns [`StoreError::Backend`] if the HTTP client cannot start.
    pub fn new(
        token_url: impl Into<String>,
        client_id: impl Into<String>,
        client_secret: Option<Secret>,
        refresh_token: Secret,
    ) -> StoreResult<Self> {
        Ok(Self {
            http: Http::new("oauth")?,
            token_url: token_url.into(),
            client_id: client_id.into(),
            client_secret,
            grant: refresh_token,
            cached: Mutex::new(None),
        })
    }

    async fn refresh(&self) -> StoreResult<(Secret, Instant)> {
        let mut form = vec![
            ("grant_type", "refresh_token"),
            ("refresh_token", self.grant.expose()),
            ("client_id", self.client_id.as_str()),
        ];
        if let Some(secret) = &self.client_secret {
            form.push(("client_secret", secret.expose()));
        }
        let response = self
            .http
            .send(|| self.http.client().post(&self.token_url).form(&form))
            .await?;
        let status = response.status();
        if !status.is_success() {
            // The body may echo the grant; report the status only.
            return Err(StoreError::Backend(format!(
                "oauth: token refresh failed with {status}"
            )));
        }
        let grant: Grant = response.json().await.map_err(|e| {
            StoreError::Backend(format!(
                "oauth: bad token response: {}",
                e.without_url()
            ))
        })?;
        let life = Duration::from_secs(grant.expires_in.unwrap_or(3600));
        let until = Instant::now() + life.saturating_sub(EARLY);
        Ok((Secret::new(grant.access_token), until))
    }
}

#[async_trait]
impl TokenSource for RefreshToken {
    #[tracing::instrument(skip_all, err)]
    async fn token(&self) -> StoreResult<Secret> {
        let mut cached = self.cached.lock().await;
        if let Some((token, until)) = cached.as_ref()
            && Instant::now() < *until
        {
            return Ok(token.clone());
        }
        let (token, until) = self.refresh().await?;
        *cached = Some((token.clone(), until));
        drop(cached);
        Ok(token)
    }

    async fn invalidate(&self) {
        *self.cached.lock().await = None;
    }
}
