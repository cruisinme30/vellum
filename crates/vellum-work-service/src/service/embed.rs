// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Running Work commands from inside the server, without the gateway.
//!
//! Integrations (GitHub, GitLab, Slack, Sentry, email) act on issues as a
//! bot user. Service crates cannot depend on each other, so the server's
//! composition root adapts these two calls to the integrations' port.
//! A command passed to [`WorkService::transact`] runs under the same lock
//! as every operation, and its changes are written and announced the same
//! way: files first, then events, with memory put back if a write fails.

use vellum_kernel::Error;
use vellum_work::{Changes, CommentChanges, Comments, Ctx, UserRef, WorkGraph};

use super::WorkService;

/// What a command changed, and what it returns to its caller.
pub type Outcome<R> = (Changes, Option<CommentChanges>, R);

impl WorkService {
    /// Reads the issue graph and comments under the service lock.
    pub async fn read<R>(&self, f: impl FnOnce(&WorkGraph, &Comments) -> R) -> R {
        let st = self.state.lock().await;
        f(&st.graph, &st.comments)
    }

    /// Runs one command as `actor` and commits what it changed.
    ///
    /// `f` gets the graph, the comments and a context with the service's
    /// clock and `actor`. If `f` fails, or the files cannot be written,
    /// memory goes back to how it was and nothing is announced.
    ///
    /// # Errors
    ///
    /// Returns the command's error, or `unavailable` if the files cannot
    /// be written.
    pub async fn transact<R>(
        &self,
        actor: UserRef,
        f: impl FnOnce(&mut WorkGraph, &mut Comments, &Ctx) -> Result<Outcome<R>, Error>,
    ) -> Result<R, Error> {
        let now = self.clock.now();
        let ctx = Ctx {
            now,
            today: now.date(),
            actor: Some(actor),
        };
        let mut st = self.state.lock().await;
        let backup = st.clone();
        let state = &mut *st;
        let (changes, comments, out) = match f(&mut state.graph, &mut state.comments, &ctx) {
            Ok(done) => done,
            Err(e) => {
                *st = backup;
                return Err(e);
            }
        };
        self.commit(&mut st, backup, changes, comments, Vec::new())
            .await?;
        Ok(out)
    }
}
