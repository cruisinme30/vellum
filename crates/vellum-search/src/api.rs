// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The search service's operations on the gateway seam.
//!
//! | Operation        | Route                                         |
//! |------------------|-----------------------------------------------|
//! | `search.query`   | `GET /v1/search?q=…&kind=…&owner=…`           |
//! | `search.titles`  | `GET /v1/search/titles?q=…`                   |
//! | `search.context` | `GET /v1/search/context/{doc_id}/{block_id}`  |
//!
//! The gateway authenticates and checks the `read` scope; every result is
//! still filtered per object by authorization inside the service (D61).
//! List filters travel as comma-separated values so they fit a query string.

use std::str::FromStr;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use vellum_contracts::{Operation, RequestContext, Scope, Service};
use vellum_kernel::{
    BlockId, DocId, Error, ErrorKind, ObjectId, SpaceId, UserId,
};

use crate::access::Principal;
use crate::model::{DocKind, Filters, SearchPage, SearchQuery, TitleHit};
use crate::service::{Context, SearchService};

/// `search.query` input.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct QueryInput {
    /// What to search for.
    pub q: String,
    /// Comma-separated space IDs to search in.
    #[serde(default)]
    pub spaces: Option<String>,
    /// Comma-separated owner (user) IDs.
    #[serde(default)]
    pub owners: Option<String>,
    /// Comma-separated kinds: `page`, `book`, `collection`, `row`, `issue`,
    /// `task_list`, `folder`.
    #[serde(default)]
    pub kinds: Option<String>,
    /// A folder, drive, book or space ID: only documents under it.
    #[serde(default)]
    pub location: Option<ObjectId>,
    /// Only documents modified at or after this time (Unix milliseconds).
    #[serde(default)]
    pub modified_from: Option<i64>,
    /// Only documents modified at or before this time (Unix milliseconds).
    #[serde(default)]
    pub modified_to: Option<i64>,
    /// Only starred documents.
    #[serde(default)]
    pub starred: bool,
    /// Search the trash.
    #[serde(default)]
    pub trashed: bool,
    /// Hits to skip.
    #[serde(default)]
    pub offset: usize,
    /// Hits to return (default 20, at most 100).
    #[serde(default)]
    pub limit: Option<usize>,
}

/// `search.titles` input.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct TitlesInput {
    /// What the person has typed so far.
    pub q: String,
    /// Comma-separated space IDs to search in.
    #[serde(default)]
    pub spaces: Option<String>,
    /// Matches to return (default 10, at most 100).
    #[serde(default)]
    pub limit: Option<usize>,
}

/// `search.context` input.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ContextInput {
    /// The document.
    pub doc_id: DocId,
    /// The block, `blk-xxxxxxxx`.
    pub block_id: BlockId,
}

/// `search.titles` output.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct TitlesPage {
    /// The matching titles, best first.
    pub hits: Vec<TitleHit>,
}

/// The search service as the gateway sees it.
#[derive(Debug, Clone)]
pub struct SearchApi {
    service: SearchService,
}

impl SearchApi {
    /// Exposes `service` on the gateway.
    #[must_use]
    pub const fn new(service: SearchService) -> Self {
        Self { service }
    }
}

fn invalid(message: String) -> Error {
    Error::new(ErrorKind::InvalidInput, message)
}

fn list<T: FromStr>(raw: Option<&str>, what: &str) -> Result<Vec<T>, Error> {
    raw.unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(|item| {
            item.parse()
                .map_err(|_| invalid(format!("`{item}` is not a valid {what}")))
        })
        .collect()
}

/// The search principal for a gateway principal; anonymous callers have
/// nothing to search.
fn principal(ctx: &RequestContext) -> Result<Principal, Error> {
    if let Some(user) = ctx.principal.user_id() {
        return Ok(Principal::User(user));
    }
    if let Some(share) = ctx.principal.share_id() {
        return Ok(Principal::ShareLink(share));
    }
    Err(Error::new(ErrorKind::Unauthenticated, "sign in to search"))
}

impl QueryInput {
    /// The service query this input describes.
    ///
    /// # Errors
    /// [`ErrorKind::InvalidInput`] for an unparsable ID, kind or range.
    pub fn to_query(&self) -> Result<SearchQuery, Error> {
        let modified = match (self.modified_from, self.modified_to) {
            (None, None) => None,
            (from, to) => {
                let (from, to) =
                    (from.unwrap_or(i64::MIN), to.unwrap_or(i64::MAX));
                if from > to {
                    return Err(invalid(
                        "`modified_from` is after `modified_to`".into(),
                    ));
                }
                Some(from..=to)
            }
        };
        Ok(SearchQuery {
            text: self.q.clone(),
            filters: Filters {
                spaces: list::<SpaceId>(self.spaces.as_deref(), "space ID")?,
                owners: list::<UserId>(self.owners.as_deref(), "user ID")?,
                kinds: list::<DocKind>(self.kinds.as_deref(), "kind")?,
                location: self.location,
                modified,
                starred: self.starred,
                trashed: self.trashed,
            },
            offset: self.offset,
            limit: self.limit.unwrap_or(20),
        })
    }
}

impl Service for SearchApi {
    fn name(&self) -> &'static str {
        "search"
    }

    fn operations(self: Arc<Self>) -> Vec<Operation> {
        let query = Arc::clone(&self);
        let titles = Arc::clone(&self);
        let context = self;
        vec![
            Operation::query(
                "search.query",
                "/v1/search",
                Scope::Read,
                move |ctx, input: QueryInput| {
                    let api = Arc::clone(&query);
                    async move {
                        let principal = principal(&ctx)?;
                        let page: SearchPage = api
                            .service
                            .search(&principal, &input.to_query()?)
                            .await?;
                        Ok(page)
                    }
                },
            )
            .summary("Searches blocks across everything the caller can read")
            .description(
                "Full-text, block-level search with drive filters. Hits carry \
                 the block ID, its heading path, a highlighted snippet and a \
                 citation URI.",
            ),
            Operation::query(
                "search.titles",
                "/v1/search/titles",
                Scope::Read,
                move |ctx, input: TitlesInput| {
                    let api = Arc::clone(&titles);
                    async move {
                        let principal = principal(&ctx)?;
                        let filters = Filters {
                            spaces: list::<SpaceId>(
                                input.spaces.as_deref(),
                                "space ID",
                            )?,
                            ..Filters::default()
                        };
                        let hits: Vec<TitleHit> = api
                            .service
                            .titles(
                                &principal,
                                &input.q,
                                &filters,
                                input.limit.unwrap_or(10),
                            )
                            .await?;
                        Ok(TitlesPage { hits })
                    }
                },
            )
            .summary("Instant title search for the command menu")
            .budget(std::time::Duration::from_millis(250)),
            Operation::query(
                "search.context",
                "/v1/search/context/{doc_id}/{block_id}",
                Scope::Read,
                move |ctx, input: ContextInput| {
                    let api = Arc::clone(&context);
                    async move {
                        let principal = principal(&ctx)?;
                        let found: Option<Context> = api
                            .service
                            .context(&principal, input.doc_id, input.block_id)
                            .await?;
                        found.ok_or_else(|| {
                            Error::new(ErrorKind::NotFound, "no such block")
                        })
                    }
                },
            )
            .summary(
                "A block with the heading section around it, for quoting \
                and citations",
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use serde_json::json;
    use vellum_contracts::{CredentialKind, ScopeSet};
    use vellum_kernel::{RequestId, Timestamp};

    use super::*;
    use crate::conformance::{Corpus, bid};
    use crate::service::Update;
    use crate::tantivy_index::{Location, TantivyBackend, TantivyConfig};

    async fn api(corpus: &Corpus) -> Arc<SearchApi> {
        let (backend, _) = TantivyBackend::open(&TantivyConfig {
            location: Location::Memory,
            key: None,
            model: None,
        })
        .unwrap();
        let service = SearchService::new(
            Arc::new(backend),
            Arc::new(corpus.access()),
            Arc::new(corpus.stars()),
        );
        service
            .write(corpus.documents().into_iter().map(Update::Upsert).collect())
            .await
            .unwrap();
        Arc::new(SearchApi::new(service))
    }

    fn ctx(principal: vellum_contracts::Principal) -> RequestContext {
        RequestContext {
            request_id: RequestId::generate(),
            principal,
            deadline: Instant::now() + Duration::from_secs(5),
            traceparent: None,
        }
    }

    fn user(id: UserId) -> vellum_contracts::Principal {
        vellum_contracts::Principal::user(
            id,
            CredentialKind::Session,
            ScopeSet::from_scopes([Scope::Read]),
            Timestamp::now(),
        )
    }

    #[tokio::test]
    async fn operations_are_valid_and_serve_requests() {
        let corpus = Corpus::default();
        let ops = api(&corpus).await.operations();
        let ids: Vec<&str> = ops.iter().map(Operation::id).collect();
        assert_eq!(ids, ["search.query", "search.titles", "search.context"]);
        for op in &ops {
            op.validate("search").unwrap();
        }
        let ada = user(corpus.ada);

        let page = ops[0]
            .call(
                ctx(ada.clone()),
                json!({ "q": "canary deploy", "kinds": "page" }),
            )
            .await
            .unwrap();
        assert_eq!(page["hits"][0]["block"], json!(bid(3).to_string()));

        let titles = ops[1]
            .call(ctx(ada.clone()), json!({ "q": "gatew" }))
            .await
            .unwrap();
        assert!(titles["hits"].as_array().is_some_and(|t| !t.is_empty()));

        let context = ops[2]
            .call(
                ctx(ada.clone()),
                json!({
                    "doc_id": corpus.runbook.to_string(),
                    "block_id": bid(5).to_string()
                }),
            )
            .await
            .unwrap();
        assert_eq!(context["section"].as_array().map(Vec::len), Some(3));

        let missing = ops[2]
            .call(
                ctx(ada),
                json!({
                    "doc_id": corpus.runbook.to_string(),
                    "block_id": bid(77).to_string()
                }),
            )
            .await
            .unwrap_err();
        assert_eq!(missing.kind(), ErrorKind::NotFound);

        let anonymous = ops[0]
            .call(
                ctx(vellum_contracts::Principal::anonymous()),
                json!({ "q": "gateway" }),
            )
            .await
            .unwrap_err();
        assert_eq!(anonymous.kind(), ErrorKind::Unauthenticated);
    }

    #[test]
    fn query_input_parses_lists_and_ranges() {
        let space = SpaceId::from_u128(1);
        let input = QueryInput {
            q: "gateway".into(),
            spaces: Some(format!("{space}, ")),
            kinds: Some("issue,page".into()),
            modified_from: Some(5),
            ..QueryInput::default()
        };
        let query = input.to_query().unwrap();
        assert_eq!(query.filters.spaces, [space]);
        assert_eq!(query.filters.kinds, [DocKind::Issue, DocKind::Page]);
        assert_eq!(query.filters.modified, Some(5..=i64::MAX));
        assert_eq!(query.limit, 20);
    }

    #[test]
    fn bad_input_is_rejected() {
        let bad_kind = QueryInput {
            q: "x".into(),
            kinds: Some("spreadsheet".into()),
            ..QueryInput::default()
        };
        assert_eq!(
            bad_kind.to_query().unwrap_err().kind(),
            ErrorKind::InvalidInput
        );
        let backwards = QueryInput {
            q: "x".into(),
            modified_from: Some(10),
            modified_to: Some(1),
            ..QueryInput::default()
        };
        assert!(backwards.to_query().is_err());
    }
}
