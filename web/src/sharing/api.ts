// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Sharing (#95): who has access to an item, and share links. The server's
 * drive owns it (`/api/v1/drive/items/{id}/sharing`, ADR 0054 D276).
 */

import type { CreatedLink, RoleView, SharingView } from "../api/api.gen";
import type { Http } from "../api/http";

/** A role that can be given in the share dialog. */
export type GrantRole = Exclude<RoleView, "owner">;

/** A share link to make. */
interface NewLink {
  /** RFC 3339; leave out for no end. */
  readonly expiresAt?: string;
  /** Eight characters or more; leave out for an open link. */
  readonly password?: string;
}

/** The sharing calls the share dialog makes. */
export interface SharingApi {
  state(id: string): Promise<SharingView>;
  share(
    id: string,
    principal: string,
    role: GrantRole,
    expiresAt?: string,
  ): Promise<SharingView>;
  unshare(id: string, principal: string): Promise<SharingView>;
  /** The answer holds the only copy of the link's URL. */
  createLink(id: string, link: NewLink): Promise<CreatedLink>;
  revokeLink(linkId: string): Promise<void>;
}

const segment = encodeURIComponent;
const item = (id: string) => `/api/v1/drive/items/${segment(id)}`;

/** Sharing over a Vellum server. */
export function createHttpSharingApi(http: Http): SharingApi {
  return {
    state: (id) => http.get<SharingView>(`${item(id)}/sharing`),
    share: (id, principal, role, expiresAt) =>
      http.send<SharingView>("PUT", `${item(id)}/grants`, {
        principal,
        role,
        expires_at: expiresAt,
      }),
    unshare: (id, principal) =>
      http.send<SharingView>(
        "DELETE",
        `${item(id)}/grants/${segment(principal)}`,
      ),
    createLink: (id, link) =>
      http.send<CreatedLink>("POST", `${item(id)}/links`, {
        expires_at: link.expiresAt,
        password: link.password,
      }),
    async revokeLink(linkId) {
      await http.send("DELETE", `/api/v1/drive/links/${segment(linkId)}`);
    },
  };
}

export type {
  CreatedLink,
  GrantView,
  LinkView,
  SharingView,
} from "../api/api.gen";
