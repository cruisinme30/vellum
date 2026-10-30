// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The sync engine as a gateway [`Service`].
//!
//! | Operation | Route | Scope |
//! |---|---|---|
//! | `sync.push` | `POST /v1/sync/push` | `write:direct` |
//! | `sync.pull` | `GET /v1/sync/pull` | `read` |
//!
//! Both need a signed-in person: a client ID is bound to the first account
//! that pushes with it, and a pull only returns what that account can read.

use std::sync::Arc;

use vellum_contracts::{
    BoxFuture, HttpMethod, Operation, Readiness, Scope, Service,
};

use crate::app::Sync as Engine;
use crate::domain::{PullRequest, PushRequest};
use crate::ports::{ReadAccess, SyncStore};

impl<S: SyncStore, A: ReadAccess> Service for Engine<S, A> {
    fn name(&self) -> &'static str {
        "sync"
    }

    fn operations(self: Arc<Self>) -> Vec<Operation> {
        let engine = Arc::clone(&self);
        let push = Operation::mutation(
            "sync.push",
            HttpMethod::Post,
            "/v1/sync/push",
            Scope::WriteDirect,
            move |ctx, request: PushRequest| {
                let engine = Arc::clone(&engine);
                async move {
                    ctx.principal.require_scope(Scope::WriteDirect)?;
                    let user = ctx.principal.require_user()?;
                    engine.push(user, ctx.request_id, request).await
                }
            },
        )
        .summary("Send a device's pending changes")
        .description(
            "Applies the device's queued mutations in order, each at most \
             once. The response says which ones are processed and which were \
             refused.",
        );
        let engine = self;
        let pull = Operation::query(
            "sync.pull",
            "/v1/sync/pull",
            Scope::Read,
            move |ctx, request: PullRequest| {
                let engine = Arc::clone(&engine);
                async move {
                    ctx.principal.require_scope(Scope::Read)?;
                    let user = ctx.principal.require_user()?;
                    engine.pull(user, request).await
                }
            },
        )
        .summary("Fetch changes since the device's last pull")
        .description(
            "Returns the records the signed-in person can read that changed \
             since the cursor. When `reset` is true the device drops its copy \
             first; when `more` is true it pulls again at once.",
        );
        vec![push, pull]
    }

    fn readiness(&self) -> BoxFuture<'_, Readiness> {
        Box::pin(async move {
            match self.store().info().await {
                Ok(_) => Readiness::Ready,
                Err(err) => Readiness::NotReady(format!(
                    "the sync log is unreachable: {err}"
                )),
            }
        })
    }
}
