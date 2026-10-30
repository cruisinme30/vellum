// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The `e2e` service on the gateway.
//!
//! | Operation | Route | Scope | Space access |
//! |---|---|---|---|
//! | `e2e.keys` | `GET /v1/spaces/{space_id}/e2e/keys` | `read` | view |
//! | `e2e.enrol` | `POST /v1/spaces/{space_id}/e2e/keys` | `write:direct`, fresh | view (manage for the first key) |
//! | `e2e.revoke` | `DELETE /v1/spaces/{space_id}/e2e/keys/{fingerprint}` | `write:direct`, fresh | view (manage for others' keys) |
//! | `e2e.objects` | `GET /v1/spaces/{space_id}/e2e/objects` | `read` | view |
//! | `e2e.get` | `GET /v1/spaces/{space_id}/e2e/object` | `read` | view |
//! | `e2e.put` | `PUT /v1/spaces/{space_id}/e2e/object` | `write:direct` | edit |
//! | `e2e.delete` | `DELETE /v1/spaces/{space_id}/e2e/object` | `write:direct` | edit |

use std::sync::Arc;
use std::time::Duration;

use vellum_contracts::{HttpMethod, Operation, Scope, Service};

use crate::api::{
    DeleteObject, EnrolDevice, GetObject, ListObjects, PutObject, RevokeDevice,
    SpaceRef,
};
use crate::app::E2e;

/// How recent a sign-in must be to change a space's keys.
const FRESH: Duration = Duration::from_mins(10);

impl Service for E2e {
    fn name(&self) -> &'static str {
        "e2e"
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one block per operation reads best"
    )]
    fn operations(self: Arc<Self>) -> Vec<Operation> {
        let svc = Arc::clone(&self);
        let keys = Operation::query(
            "e2e.keys",
            "/v1/spaces/{space_id}/e2e/keys",
            Scope::Read,
            move |ctx, input: SpaceRef| {
                let svc = Arc::clone(&svc);
                async move {
                    svc.keys(ctx.principal.require_user()?, input.space_id)
                        .await
                }
            },
        )
        .summary("List the devices a private space is sealed to")
        .description(
            "Every device key enrolled in the end-to-end encrypted space. \
             Devices seal new pages to exactly these keys. An empty list means \
             the space is not private.",
        );
        let svc = Arc::clone(&self);
        let enrol =
            Operation::mutation(
                "e2e.enrol",
                HttpMethod::Post,
                "/v1/spaces/{space_id}/e2e/keys",
                Scope::WriteDirect,
                move |ctx, input: EnrolDevice| {
                    let svc = Arc::clone(&svc);
                    async move {
                        svc.enrol(ctx.principal.require_user()?, input).await
                    }
                },
            )
            .fresh_auth(FRESH)
            .summary("Enrol this device in a private space")
            .description(
                "Adds the caller's device public key. The first key makes the \
             space private and needs a manager. Existing members' devices then \
             add the new key to each page.",
            );
        let svc = Arc::clone(&self);
        let revoke = Operation::mutation(
            "e2e.revoke",
            HttpMethod::Delete,
            "/v1/spaces/{space_id}/e2e/keys/{fingerprint}",
            Scope::WriteDirect,
            move |ctx, input: RevokeDevice| {
                let svc = Arc::clone(&svc);
                async move {
                    svc.revoke(ctx.principal.require_user()?, input).await
                }
            },
        )
        .fresh_auth(FRESH)
        .summary("Revoke a device from a private space")
        .description(
            "Removes a lost or retired device's key: your own, or anyone's if \
             you manage the space. Members' devices strip it from each page. A \
             device that kept old ciphertext can still read what it could read \
             before.",
        );
        let svc = Arc::clone(&self);
        let objects = Operation::query(
            "e2e.objects",
            "/v1/spaces/{space_id}/e2e/objects",
            Scope::Read,
            move |ctx, input: ListObjects| {
                let svc = Arc::clone(&svc);
                async move {
                    svc.objects(ctx.principal.require_user()?, input).await
                }
            },
        )
        .summary("List a private space's objects")
        .description(
            "Paths, sizes and change times, a page at a time in path order.",
        );
        let svc = Arc::clone(&self);
        let get = Operation::query(
            "e2e.get",
            "/v1/spaces/{space_id}/e2e/object",
            Scope::Read,
            move |ctx, input: GetObject| {
                let svc = Arc::clone(&svc);
                async move {
                    svc.get(ctx.principal.require_user()?, input).await
                }
            },
        )
        .summary("Download a sealed page")
        .description(
            "The recipient envelope, base64, as stored. Only members' devices \
             can open it. Objects that are not end-to-end encrypted are \
             refused.",
        );
        let svc = Arc::clone(&self);
        let put = Operation::mutation(
            "e2e.put",
            HttpMethod::Put,
            "/v1/spaces/{space_id}/e2e/object",
            Scope::WriteDirect,
            move |ctx, input: PutObject| {
                let svc = Arc::clone(&svc);
                async move {
                    svc.put(ctx.principal.require_user()?, input).await
                }
            },
        )
        .summary("Upload a page sealed on this device")
        .description(
            "Stores a recipient envelope sealed to exactly the space's current \
             keys. Pass the version you replace as `if_match`, or nothing to \
             create.",
        );
        let svc = self;
        let delete =
            Operation::mutation(
                "e2e.delete",
                HttpMethod::Delete,
                "/v1/spaces/{space_id}/e2e/object",
                Scope::WriteDirect,
                move |ctx, input: DeleteObject| {
                    let svc = Arc::clone(&svc);
                    async move {
                        svc.delete(ctx.principal.require_user()?, input).await
                    }
                },
            )
            .summary("Delete a sealed page")
            .description("Deletes the object if it is still at `if_match`.");
        vec![keys, enrol, revoke, objects, get, put, delete]
    }
}
