// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The `assist` API: test a model, summarise a page, ask a space, draft a
//! page from a template. Every call carries the user's model settings.

use std::sync::Arc;
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use vellum_contracts::{HttpMethod, Operation, Principal, RequestContext, Scope, Service};
use vellum_kernel::{Error, ErrorKind};

use crate::model::{ModelClient, ModelSettings, Prompt};
use crate::prompt;
use crate::sources::{DraftPage, Sources};

/// How long a summary or an answer may take.
pub const ANSWER_BUDGET: Duration = Duration::from_mins(2);
/// How long a draft may take.
pub const DRAFT_BUDGET: Duration = Duration::from_mins(3);
/// Search hits quoted to answer a question.
pub const ASK_HITS: u32 = 8;
/// Longest question or instructions accepted.
const MAX_TEXT: usize = 4_000;

/// The bring-your-own-model service.
pub struct AssistService {
    model: Arc<dyn ModelClient>,
    sources: Arc<dyn Sources>,
}

impl std::fmt::Debug for AssistService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssistService").finish_non_exhaustive()
    }
}

/// `POST /v1/assist/test`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TestInput {
    /// The model to try.
    pub model: ModelSettings,
}

/// Whether the model answered.
#[derive(Debug, Serialize, JsonSchema, PartialEq, Eq)]
pub struct TestResult {
    /// The model answered.
    pub ok: bool,
    /// What happened, in a sentence.
    pub detail: String,
}

/// `POST /v1/assist/summarize`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SummarizeInput {
    /// The model to use.
    pub model: ModelSettings,
    /// The page to summarise.
    pub page_id: String,
}

/// `POST /v1/assist/ask`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AskInput {
    /// The model to use.
    pub model: ModelSettings,
    /// The question.
    pub question: String,
    /// Limit the search to one space (slug).
    #[serde(default)]
    pub space: Option<String>,
}

/// One source an answer cites.
#[derive(Debug, Clone, Serialize, JsonSchema, PartialEq, Eq)]
pub struct Citation {
    /// The number the answer uses, `[n]`.
    pub n: usize,
    /// The page's id.
    pub doc_id: String,
    /// Page title and headings above the block.
    pub title: String,
    /// The quoted text.
    pub snippet: String,
    /// Opens the block in the app.
    pub url: String,
}

/// A summary or an answer.
#[derive(Debug, Serialize, JsonSchema, PartialEq, Eq)]
pub struct Answer {
    /// The text, as Markdown, with `[n]` citation marks.
    pub text: String,
    /// The sources cited, in order of first use.
    pub citations: Vec<Citation>,
    /// The input or the answer was cut short.
    pub truncated: bool,
    /// The model that answered.
    pub model: String,
}

/// `POST /v1/assist/draft`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DraftInput {
    /// The model to use.
    pub model: ModelSettings,
    /// The page whose structure the draft follows.
    pub template_page_id: String,
    /// The new page's title.
    pub title: String,
    /// What the page should say.
    pub instructions: String,
    /// Where the draft goes (a page id or `space:<slug>`); by default, the
    /// template's space.
    #[serde(default)]
    pub parent: Option<String>,
}

/// The draft page that was created for review.
#[derive(Debug, Serialize, JsonSchema, PartialEq, Eq)]
pub struct Draft {
    /// The new page's id.
    pub page_id: String,
    /// Opens it.
    pub url: String,
    /// Its body.
    pub markdown: String,
    /// The template or the answer was cut short.
    pub truncated: bool,
    /// The model that wrote it.
    pub model: String,
}

fn text(field: &str, value: &str) -> Result<String, Error> {
    let value = value.trim();
    if value.is_empty() {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!("{field}: required"),
        ));
    }
    if value.chars().count() > MAX_TEXT {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!("{field}: at most {MAX_TEXT} characters"),
        ));
    }
    Ok(value.to_owned())
}

impl AssistService {
    /// The service over a model client and the workspace.
    #[must_use]
    pub fn new(model: Arc<dyn ModelClient>, sources: Arc<dyn Sources>) -> Self {
        Self { model, sources }
    }

    async fn call(
        &self,
        settings: &ModelSettings,
        prompt: &Prompt,
    ) -> Result<crate::model::Completion, Error> {
        settings.validate()?;
        Ok(self.model.complete(settings, prompt).await?)
    }

    /// Sends a one-line prompt to check the settings.
    ///
    /// # Errors
    ///
    /// Invalid settings only; a failed call is reported in the result.
    pub async fn test(&self, who: &Principal, input: TestInput) -> Result<TestResult, Error> {
        let _ = who.require_user()?;
        input.model.validate()?;
        let prompt = Prompt {
            system: "You check that a connection works.".into(),
            user: "Reply with the single word: ready".into(),
            max_tokens: 1_000,
        };
        Ok(match self.model.complete(&input.model, &prompt).await {
            Ok(_) => TestResult {
                ok: true,
                detail: format!("{} answered.", input.model.model.trim()),
            },
            Err(e) => TestResult {
                ok: false,
                detail: capitalise(&e.to_string()),
            },
        })
    }

    /// Summarises a page the user can read.
    ///
    /// # Errors
    ///
    /// `not_found` for a page the user cannot read, and the model's errors.
    pub async fn summarize(&self, who: &Principal, input: SummarizeInput) -> Result<Answer, Error> {
        let _ = who.require_user()?;
        input.model.validate()?;
        let page = self.sources.page(who, input.page_id.trim()).await?;
        let (p, cut) = prompt::summarize(&page);
        let done = self.call(&input.model, &p).await?;
        Ok(Answer {
            text: done.text.trim().to_owned(),
            citations: Vec::new(),
            truncated: cut || done.truncated,
            model: input.model.model.trim().to_owned(),
        })
    }

    /// Answers a question from the user's permission-filtered search
    /// results, with citations.
    ///
    /// # Errors
    ///
    /// Invalid input, search failures and the model's errors.
    pub async fn ask(&self, who: &Principal, input: AskInput) -> Result<Answer, Error> {
        let _ = who.require_user()?;
        input.model.validate()?;
        let question = text("question", &input.question)?;
        let space = input
            .space
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let hits = self.sources.search(who, &question, space, ASK_HITS).await?;
        if hits.is_empty() {
            return Ok(Answer {
                text: "Nothing you can see in this workspace matches that question, so there is nothing to answer from.".into(),
                citations: Vec::new(),
                truncated: false,
                model: input.model.model.trim().to_owned(),
            });
        }
        let done = self
            .call(&input.model, &prompt::ask(&question, &hits))
            .await?;
        let citations = prompt::cited(&done.text, hits.len())
            .into_iter()
            .filter_map(|n| {
                hits.get(n - 1).map(|h| Citation {
                    n,
                    doc_id: h.doc_id.clone(),
                    title: std::iter::once(h.title.as_str())
                        .chain(h.heading_path.iter().map(String::as_str))
                        .collect::<Vec<_>>()
                        .join(" › "),
                    snippet: prompt::clean(&h.snippet),
                    url: h.url.clone(),
                })
            })
            .collect();
        Ok(Answer {
            text: done.text.trim().to_owned(),
            citations,
            truncated: done.truncated,
            model: input.model.model.trim().to_owned(),
        })
    }

    /// Writes a page from a template and saves it as a draft for review.
    ///
    /// # Errors
    ///
    /// Invalid input, an unreadable template, the model's errors, and
    /// `permission_denied` where the user cannot create pages.
    pub async fn draft(&self, who: &Principal, input: DraftInput) -> Result<Draft, Error> {
        let _ = who.require_user()?;
        who.require_scope(Scope::WriteDraft)?;
        input.model.validate()?;
        let title = text("title", &input.title)?;
        let instructions = text("instructions", &input.instructions)?;
        let template = self
            .sources
            .page(who, input.template_page_id.trim())
            .await?;
        let (p, cut) = prompt::draft(&template, &title, &instructions);
        let done = self.call(&input.model, &p).await?;
        let markdown = prompt::unfence(&done.text).to_owned();
        let parent = input
            .parent
            .map(|p| p.trim().to_owned())
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| format!("space:{}", template.space));
        let created = self
            .sources
            .create_draft(
                who,
                DraftPage {
                    parent,
                    title,
                    markdown: markdown.clone(),
                    template: Some(template.id),
                },
            )
            .await?;
        Ok(Draft {
            page_id: created.id,
            url: created.url,
            markdown,
            truncated: cut || done.truncated,
            model: input.model.model.trim().to_owned(),
        })
    }
}

fn capitalise(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map_or_else(String::new, |first| {
        let mut out: String = first.to_uppercase().collect();
        out.push_str(chars.as_str());
        if !out.ends_with('.') {
            out.push('.');
        }
        out
    })
}

impl Service for AssistService {
    fn name(&self) -> &'static str {
        "assist"
    }

    fn operations(self: Arc<Self>) -> Vec<Operation> {
        let test = {
            let s = Arc::clone(&self);
            move |ctx: RequestContext, input: TestInput| {
                let s = Arc::clone(&s);
                async move { s.test(&ctx.principal, input).await }
            }
        };
        let summarize = {
            let s = Arc::clone(&self);
            move |ctx: RequestContext, input: SummarizeInput| {
                let s = Arc::clone(&s);
                async move { s.summarize(&ctx.principal, input).await }
            }
        };
        let ask = {
            let s = Arc::clone(&self);
            move |ctx: RequestContext, input: AskInput| {
                let s = Arc::clone(&s);
                async move { s.ask(&ctx.principal, input).await }
            }
        };
        let draft = {
            let s = Arc::clone(&self);
            move |ctx: RequestContext, input: DraftInput| {
                let s = Arc::clone(&s);
                async move { s.draft(&ctx.principal, input).await }
            }
        };
        vec![
            Operation::mutation("assist.test", HttpMethod::Post, "/v1/assist/test", Scope::Read, test)
                .summary("Check that your own model answers")
                .description("Sends a one-line prompt to the model in the settings. The API key is used for this call only and never stored.")
                .budget(ANSWER_BUDGET),
            Operation::mutation(
                "assist.summarize",
                HttpMethod::Post,
                "/v1/assist/summarize",
                Scope::Read,
                summarize,
            )
            .summary("Summarise a page with your own model")
            .description("Reads the page as you and sends it to the model in the settings, with your key. Vellum runs no model and keeps no key.")
            .budget(ANSWER_BUDGET),
            Operation::mutation("assist.ask", HttpMethod::Post, "/v1/assist/ask", Scope::Read, ask)
                .summary("Ask this space, answered with citations by your own model")
                .description("Searches what you can read, quotes the best blocks to the model, and returns its answer with links to the blocks it cites.")
                .budget(ANSWER_BUDGET),
            Operation::mutation(
                "assist.draft",
                HttpMethod::Post,
                "/v1/assist/draft",
                Scope::WriteDraft,
                draft,
            )
            .summary("Draft a page from a template with your own model")
            .description("Writes a page that follows the template's structure and saves it as a draft for you to review.")
            .budget(DRAFT_BUDGET),
        ]
    }
}
