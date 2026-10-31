// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * My Work's connections (#525, D298): the secret calendar feed, CalDAV
 * with an app password, the capture e-mail address, Slack capture and
 * imports from other task apps.
 */

import type {
  ConnectionsView,
  CreatedToken,
  ImportFrom,
  ImportView,
  SecretView,
} from "../api/api.gen";
import type { Http } from "../api/http";

/** The connection operations the settings page uses. */
export interface ConnectionsApi {
  /** The CalDAV URL and whether e-mail and Slack capture are available. */
  readonly connections: () => Promise<ConnectionsView>;
  /** Issues a new feed URL; the old one stops working. Shown once. */
  readonly rotateFeed: () => Promise<string>;
  /** Turns the calendar feed off. */
  readonly revokeFeed: () => Promise<void>;
  /** Issues a new capture address; the old one stops working. Shown once. */
  readonly rotateCapture: () => Promise<string>;
  /** Turns capture by e-mail off. */
  readonly revokeCapture: () => Promise<void>;
  /** Removes the person's Slack links. */
  readonly unlinkSlack: () => Promise<void>;
  /**
   * Makes an API token for a CalDAV app, which it uses as its password. Shown
   * once.
   */
  readonly appPassword: (name: string) => Promise<string>;
  /** Imports an export from another app; `project` names a Todoist list. */
  readonly importTasks: (
    from: ImportFrom,
    content: string,
    project?: string,
  ) => Promise<ImportView>;
}

const MY_WORK = "/api/v1/my-work";

/** Connections over a Vellum server. */
export function createHttpConnectionsApi(http: Http): ConnectionsApi {
  const secret = async (path: string) => {
    const issued = await http.send<SecretView>("POST", path, {});
    return issued.value;
  };
  return {
    connections: () => http.get<ConnectionsView>(`${MY_WORK}/connections`),
    rotateFeed: () => secret(`${MY_WORK}/feed`),
    async revokeFeed() {
      await http.send("DELETE", `${MY_WORK}/feed`);
    },
    rotateCapture: () => secret(`${MY_WORK}/capture-address`),
    async revokeCapture() {
      await http.send("DELETE", `${MY_WORK}/capture-address`);
    },
    async unlinkSlack() {
      await http.send("DELETE", `${MY_WORK}/slack-link`);
    },
    async appPassword(name) {
      // CalDAV clients read and write tasks, nothing else.
      const created = await http.send<CreatedToken>(
        "POST",
        "/api/v1/me/tokens",
        {
          name,
          scopes: ["read", "write:direct"],
        },
      );
      return created.token;
    },
    importTasks: (from, content, project) =>
      http.send<ImportView>(
        "POST",
        "/api/v1/tasks/import",
        project === undefined || project.trim() === ""
          ? { from, content }
          : { from, content, project: project.trim() },
      ),
  };
}

export type { ConnectionsView, ImportFrom, ImportView } from "../api/api.gen";
