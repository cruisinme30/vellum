// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * End-to-end encrypted private spaces on the server (`/api/v1/spaces/{id}/e2e`,
 * ADR 0043): which devices each shared drive is sealed to. Enrolling and
 * resealing run on the desktop app, which holds the device key.
 */

import type { Revoked, SpaceKeys, SpaceList } from "../../api/api.gen";
import type { Http } from "../../api/http";

/** A shared drive, with the devices its private pages are sealed to. */
export interface PrivateSpace {
  readonly id: string;
  readonly name: string;
  readonly keys: SpaceKeys;
}

/** The private-space operations the web app uses. */
export interface PrivateSpacesApi {
  /** The caller's shared drives, by name, each with its key list. */
  spaces(): Promise<PrivateSpace[]>;
  /** Removes one device key: one's own, or anyone's for a manager. */
  revoke(space: string, fingerprint: string): Promise<Revoked>;
}

const segment = encodeURIComponent;

/** Private spaces over a Vellum server. */
export function createHttpPrivateSpacesApi(http: Http): PrivateSpacesApi {
  return {
    async spaces() {
      const { spaces } = await http.get<SpaceList>(
        "/api/v1/drive/shared-drives",
      );
      const withKeys = await Promise.all(
        spaces.map(async (space) => ({
          id: space.id,
          name: space.name,
          keys: await http.get<SpaceKeys>(
            `/api/v1/spaces/${segment(space.id)}/e2e/keys`,
          ),
        })),
      );
      return withKeys.toSorted((a, b) => a.name.localeCompare(b.name));
    },
    revoke: (space, fingerprint) =>
      http.send<Revoked>(
        "DELETE",
        `/api/v1/spaces/${segment(space)}/e2e/keys/${segment(fingerprint)}`,
      ),
  };
}
