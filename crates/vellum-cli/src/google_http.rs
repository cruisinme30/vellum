// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Google Calendar over HTTPS, for My Work's time blocks.
//!
//! [`GoogleHttp`] implements `vellum_tasks::google::GoogleCalendar` with
//! the server's `OAuth` client (`VELLUM_GOOGLE_CLIENT_ID` and
//! `VELLUM_GOOGLE_CLIENT_SECRET`). Access tokens are cached until a
//! minute before they expire. Tokens never reach the logs.
//!
//! [`FileLinks`] keeps each person's connection in
//! `<data>/.vellum/tasks/google/<user>.json`, readable only by the
//! server's own account.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Mutex;
use url::Url;
use vellum_contracts::BoxFuture;
use vellum_kernel::{Error, ErrorKind, UserId};
use vellum_tasks::google::{Changes, GoogleCalendar, GoogleLink, GoogleLinks};

/// The client ID variable.
pub const CLIENT_ID_VAR: &str = "VELLUM_GOOGLE_CLIENT_ID";
/// The client secret variable.
pub const CLIENT_SECRET_VAR: &str = "VELLUM_GOOGLE_CLIENT_SECRET";

/// Renew an access token this long before it expires.
const EARLY: Duration = Duration::from_mins(1);

/// Events per page when listing changes.
const PAGE_SIZE: &str = "250";

/// The most pages one sync reads.
const MAX_PAGES: usize = 40;

fn unavailable(what: &str, e: impl std::fmt::Display) -> Error {
    Error::new(ErrorKind::Unavailable, format!("{what}: {e}"))
}

/// Where Google's endpoints are; tests point these at a fake.
#[derive(Debug, Clone)]
pub struct Endpoints {
    /// The token endpoint.
    pub token: Url,
    /// The revocation endpoint.
    pub revoke: Url,
    /// The Calendar API base, ending in `/`.
    pub api: Url,
}

impl Endpoints {
    /// Google's own endpoints.
    ///
    /// # Errors
    ///
    /// Never in practice; the URLs are constants.
    pub fn google() -> Result<Self, Error> {
        let parse = |u: &str| {
            Url::parse(u).map_err(|e| unavailable("bad Google URL", e))
        };
        Ok(Self {
            token: parse("https://oauth2.googleapis.com/token")?,
            revoke: parse("https://oauth2.googleapis.com/revoke")?,
            api: parse("https://www.googleapis.com/calendar/v3/")?,
        })
    }
}

#[derive(Deserialize)]
struct TokenReply {
    access_token: String,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    refresh_token: Option<String>,
}

/// Google Calendar over HTTPS.
pub struct GoogleHttp {
    http: reqwest::Client,
    endpoints: Endpoints,
    client_id: String,
    client_secret: String,
    tokens: Mutex<HashMap<String, (String, Instant)>>,
}

impl std::fmt::Debug for GoogleHttp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GoogleHttp").finish_non_exhaustive()
    }
}

impl GoogleHttp {
    /// A client for `endpoints` with the server's `OAuth` client.
    ///
    /// # Errors
    ///
    /// The HTTP client cannot start.
    pub fn new(
        endpoints: Endpoints,
        client_id: String,
        client_secret: String,
    ) -> Result<Self, Error> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| unavailable("cannot start the HTTP client", e))?;
        Ok(Self {
            http,
            endpoints,
            client_id,
            client_secret,
            tokens: Mutex::new(HashMap::new()),
        })
    }

    async fn token_request(
        &self,
        form: &[(&str, &str)],
    ) -> Result<TokenReply, Error> {
        let response = self
            .http
            .post(self.endpoints.token.clone())
            .form(form)
            .send()
            .await
            .map_err(|e| {
                unavailable("Google sign-in failed", e.without_url())
            })?;
        let status = response.status();
        if status.as_u16() == 400 || status.as_u16() == 401 {
            return Err(Error::new(
                ErrorKind::Unauthenticated,
                "Google refused the grant; connect Google Calendar again",
            ));
        }
        if !status.is_success() {
            return Err(unavailable("Google sign-in failed", status));
        }
        response
            .json()
            .await
            .map_err(|e| unavailable("Google sign-in reply", e.without_url()))
    }

    /// A valid access token for a refresh token.
    async fn access(&self, grant: &str) -> Result<String, Error> {
        if let Some((token, until)) = self.tokens.lock().await.get(grant)
            && *until > Instant::now()
        {
            return Ok(token.clone());
        }
        let reply = self
            .token_request(&[
                ("client_id", &self.client_id),
                ("client_secret", &self.client_secret),
                ("refresh_token", grant),
                ("grant_type", "refresh_token"),
            ])
            .await?;
        let life = Duration::from_secs(reply.expires_in.unwrap_or(3600))
            .saturating_sub(EARLY);
        let _ = self.tokens.lock().await.insert(
            grant.to_owned(),
            (reply.access_token.clone(), Instant::now() + life),
        );
        Ok(reply.access_token)
    }

    fn url(&self, segments: &[&str]) -> Result<Url, Error> {
        let mut url = self.endpoints.api.clone();
        {
            let mut path = url.path_segments_mut().map_err(|()| {
                unavailable("bad Calendar URL", "cannot be a base")
            })?;
            let _ = path.pop_if_empty();
            for segment in segments {
                let _ = path.push(segment);
            }
        }
        Ok(url)
    }

    async fn call(
        &self,
        grant: &str,
        method: reqwest::Method,
        url: Url,
        body: Option<&Value>,
    ) -> Result<reqwest::Response, Error> {
        let token = self.access(grant).await?;
        let mut request = self.http.request(method, url).bearer_auth(token);
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().await.map_err(|e| {
            unavailable("Google Calendar failed", e.without_url())
        })?;
        if response.status().as_u16() == 401 {
            let _ = self.tokens.lock().await.remove(grant);
        }
        Ok(response)
    }

    async fn json(response: reqwest::Response) -> Result<Value, Error> {
        let status = response.status();
        match status.as_u16() {
            200..=299 => response.json().await.map_err(|e| {
                unavailable("Google Calendar reply", e.without_url())
            }),
            401 | 403 => Err(Error::new(
                ErrorKind::Unauthenticated,
                "Google Calendar refused the request; connect it again",
            )),
            404 | 410 => Err(Error::new(ErrorKind::NotFound, "no such event")),
            429 => Err(Error::new(
                ErrorKind::RateLimited,
                "Google Calendar is busy",
            )),
            _ => Err(unavailable("Google Calendar failed", status)),
        }
    }

    fn id_of(value: &Value) -> Result<String, Error> {
        value
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| unavailable("Google Calendar reply", "no id"))
    }

    async fn list(
        &self,
        grant: &str,
        calendar: &str,
        sync_token: Option<&str>,
    ) -> Result<Changes, Error> {
        let mut events = Vec::new();
        let mut page_token: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let mut url = self.url(&["calendars", calendar, "events"])?;
            {
                let mut query = url.query_pairs_mut();
                let _ = query
                    .append_pair("maxResults", PAGE_SIZE)
                    .append_pair("showDeleted", "true")
                    .append_pair("singleEvents", "true");
                if let Some(token) = sync_token {
                    let _ = query.append_pair("syncToken", token);
                }
                if let Some(token) = &page_token {
                    let _ = query.append_pair("pageToken", token);
                }
            }
            let response =
                self.call(grant, reqwest::Method::GET, url, None).await?;
            if response.status().as_u16() == 410 {
                return Ok(Changes::Expired);
            }
            let page = Self::json(response).await?;
            if let Some(items) = page.get("items").and_then(Value::as_array) {
                events.extend(items.iter().cloned());
            }
            if let Some(next) =
                page.get("nextPageToken").and_then(Value::as_str)
            {
                page_token = Some(next.to_owned());
                continue;
            }
            return Ok(Changes::Page {
                events,
                next_sync_token: page
                    .get("nextSyncToken")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            });
        }
        Err(unavailable(
            "Google Calendar",
            "too many changes in one sync",
        ))
    }
}

impl GoogleCalendar for GoogleHttp {
    fn exchange(
        &self,
        code: String,
        verifier: String,
        redirect: String,
    ) -> BoxFuture<'_, Result<String, Error>> {
        Box::pin(async move {
            let reply = self
                .token_request(&[
                    ("client_id", &self.client_id),
                    ("client_secret", &self.client_secret),
                    ("code", &code),
                    ("code_verifier", &verifier),
                    ("redirect_uri", &redirect),
                    ("grant_type", "authorization_code"),
                ])
                .await?;
            let grant = reply.refresh_token.ok_or_else(|| {
                Error::new(
                    ErrorKind::Unauthenticated,
                    "Google sent no refresh token; connect again",
                )
            })?;
            let life = Duration::from_secs(reply.expires_in.unwrap_or(3600))
                .saturating_sub(EARLY);
            let _ = self.tokens.lock().await.insert(
                grant.clone(),
                (reply.access_token, Instant::now() + life),
            );
            Ok(grant)
        })
    }

    fn create_calendar(
        &self,
        grant: String,
        name: String,
        zone: String,
    ) -> BoxFuture<'_, Result<String, Error>> {
        Box::pin(async move {
            let url = self.url(&["calendars"])?;
            let body = json!({"summary": name, "timeZone": zone});
            let reply = self
                .call(&grant, reqwest::Method::POST, url, Some(&body))
                .await?;
            Self::id_of(&Self::json(reply).await?)
        })
    }

    fn changes(
        &self,
        grant: String,
        calendar: String,
        sync_token: Option<String>,
    ) -> BoxFuture<'_, Result<Changes, Error>> {
        Box::pin(async move {
            self.list(&grant, &calendar, sync_token.as_deref()).await
        })
    }

    fn insert(
        &self,
        grant: String,
        calendar: String,
        body: Value,
    ) -> BoxFuture<'_, Result<String, Error>> {
        Box::pin(async move {
            let url = self.url(&["calendars", &calendar, "events"])?;
            let reply = self
                .call(&grant, reqwest::Method::POST, url, Some(&body))
                .await?;
            Self::id_of(&Self::json(reply).await?)
        })
    }

    fn patch(
        &self,
        grant: String,
        calendar: String,
        event: String,
        body: Value,
    ) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(async move {
            let url = self.url(&["calendars", &calendar, "events", &event])?;
            let reply = self
                .call(&grant, reqwest::Method::PATCH, url, Some(&body))
                .await?;
            Self::json(reply).await.map(|_| ())
        })
    }

    fn delete(
        &self,
        grant: String,
        calendar: String,
        event: String,
    ) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(async move {
            let url = self.url(&["calendars", &calendar, "events", &event])?;
            let reply = self
                .call(&grant, reqwest::Method::DELETE, url, None)
                .await?;
            match reply.status().as_u16() {
                200..=299 | 404 | 410 => Ok(()),
                _ => Self::json(reply).await.map(|_| ()),
            }
        })
    }

    fn revoke(&self, grant: String) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(async move {
            let _ = self.tokens.lock().await.remove(&grant);
            let response = self
                .http
                .post(self.endpoints.revoke.clone())
                .form(&[("token", grant.as_str())])
                .send()
                .await
                .map_err(|e| {
                    unavailable("Google revoke failed", e.without_url())
                })?;
            if response.status().is_success()
                || response.status().as_u16() == 400
            {
                Ok(())
            } else {
                Err(unavailable("Google revoke failed", response.status()))
            }
        })
    }
}

/// Connections kept one file per person (or in memory).
#[derive(Debug)]
pub struct FileLinks {
    dir: Option<PathBuf>,
    memory: Mutex<HashMap<UserId, GoogleLink>>,
}

impl FileLinks {
    /// Connections under `dir`, or in memory when `None`.
    #[must_use]
    pub fn new(dir: Option<PathBuf>) -> Self {
        Self {
            dir,
            memory: Mutex::new(HashMap::new()),
        }
    }

    fn path(&self, user: UserId) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join(format!("{user}.json")))
    }
}

fn write_private(path: &std::path::Path, bytes: &[u8]) -> Result<(), Error> {
    use std::io::Write as _;
    let fail =
        |e: std::io::Error| unavailable("cannot save a Google connection", e);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(fail)?;
    }
    let temp = path.with_extension("json.tmp");
    let mut options = std::fs::OpenOptions::new();
    let _ = options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        let _ = options.mode(0o600);
    }
    let mut file = options.open(&temp).map_err(fail)?;
    file.write_all(bytes).map_err(fail)?;
    file.sync_all().map_err(fail)?;
    std::fs::rename(&temp, path).map_err(fail)
}

impl GoogleLinks for FileLinks {
    fn link(
        &self,
        user: UserId,
    ) -> BoxFuture<'_, Result<Option<GoogleLink>, Error>> {
        Box::pin(async move {
            let Some(path) = self.path(user) else {
                return Ok(self.memory.lock().await.get(&user).cloned());
            };
            match std::fs::read(&path) {
                Ok(bytes) => {
                    serde_json::from_slice(&bytes).map(Some).map_err(|e| {
                        unavailable("a Google connection is unreadable", e)
                    })
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => {
                    Err(unavailable("cannot read a Google connection", e))
                }
            }
        })
    }

    fn set_link(
        &self,
        user: UserId,
        link: Option<GoogleLink>,
    ) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(async move {
            let Some(path) = self.path(user) else {
                match link {
                    Some(link) => {
                        let _ = self.memory.lock().await.insert(user, link);
                    }
                    None => {
                        let _ = self.memory.lock().await.remove(&user);
                    }
                }
                return Ok(());
            };
            match link {
                Some(link) => {
                    let bytes = serde_json::to_vec(&link).map_err(|e| {
                        Error::new(ErrorKind::Internal, e.to_string())
                    })?;
                    write_private(&path, &bytes)
                }
                None => match std::fs::remove_file(&path) {
                    Ok(()) => Ok(()),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        Ok(())
                    }
                    Err(e) => {
                        Err(unavailable("cannot remove a Google connection", e))
                    }
                },
            }
        })
    }

    fn linked(&self) -> BoxFuture<'_, Result<Vec<UserId>, Error>> {
        Box::pin(async move {
            let Some(dir) = &self.dir else {
                return Ok(self.memory.lock().await.keys().copied().collect());
            };
            let entries = match std::fs::read_dir(dir) {
                Ok(entries) => entries,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(Vec::new());
                }
                Err(e) => {
                    return Err(unavailable(
                        "cannot list Google connections",
                        e,
                    ));
                }
            };
            Ok(entries
                .flatten()
                .filter_map(|e| {
                    e.file_name()
                        .to_str()?
                        .strip_suffix(".json")?
                        .parse::<UserId>()
                        .ok()
                })
                .collect())
        })
    }
}
