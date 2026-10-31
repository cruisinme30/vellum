// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { Channel, type ChannelName } from "../shared/channels.js";

import { APP_URL, isAppUrl, isSafeExternal } from "./security.js";

/** What the bridge can do, implemented by the main process. */
export interface BridgeServices {
  serverUrl(): string;
  signIn(): Promise<{ account: string }>;
  signOut(): Promise<void>;
  accessToken(): Promise<string | undefined>;
  openExternal(url: string): Promise<void>;
  notify(notice: Notice): void;
  /** Private spaces; the inputs are checked by the implementation. */
  privateDeviceKey(): Promise<unknown>;
  privateEnrol(spaceId: unknown, label: unknown): Promise<unknown>;
  privateReseal(spaceId: unknown, trust: unknown): Promise<unknown>;
}

/** A notification; `url` is an app path opened on click. */
export interface Notice {
  readonly title: string;
  readonly body: string;
  readonly url: string;
}

type Handler = (input: unknown) => Promise<unknown>;

/**
 * Builds the IPC handlers. Every call is checked: it must come from an app
 * page, and its arguments are validated here rather than trusted.
 */
export function bridgeHandlers(
  services: BridgeServices,
): ReadonlyMap<ChannelName, Handler> {
  return new Map<ChannelName, Handler>([
    [Channel.serverUrl, () => Promise.resolve(services.serverUrl())],
    [Channel.signIn, () => services.signIn()],
    [Channel.signOut, () => services.signOut()],
    [Channel.accessToken, () => services.accessToken()],
    [
      Channel.openExternal,
      async (input) => {
        if (typeof input !== "string" || !isSafeExternal(input))
          throw new TypeError("Only web and mail links open outside Vellum");
        await services.openExternal(input);
      },
    ],
    [Channel.privateDeviceKey, () => services.privateDeviceKey()],
    [
      Channel.privateEnrol,
      (input) => {
        const { spaceId, label } = (
          typeof input === "object" && input !== null ? input : {}
        ) as {
          spaceId?: unknown;
          label?: unknown;
        };
        return services.privateEnrol(spaceId, label);
      },
    ],
    [
      Channel.privateReseal,
      (input) => {
        const { spaceId, trust } = (
          typeof input === "object" && input !== null ? input : {}
        ) as {
          spaceId?: unknown;
          trust?: unknown;
        };
        return services.privateReseal(spaceId, trust);
      },
    ],
    [
      Channel.notify,
      (input) => {
        services.notify(parseNotice(input));
        return Promise.resolve();
      },
    ],
  ]);
}

/** Runs a handler only for calls from app pages. */
export async function dispatch(
  handlers: ReadonlyMap<ChannelName, Handler>,
  channel: ChannelName,
  senderUrl: string,
  input?: unknown,
): Promise<unknown> {
  if (!isAppUrl(senderUrl))
    throw new Error("Vellum desktop calls are only allowed from app pages");
  const handler = handlers.get(channel);
  if (handler === undefined) throw new Error(`Unknown channel ${channel}`);
  return handler(input);
}

/** Validates a notification from the page; links must stay inside the app. */
export function parseNotice(input: unknown): Notice {
  if (typeof input !== "object" || input === null)
    throw new TypeError("A notification needs a title, body and url");
  const record = input as Record<string, unknown>;
  const { title, body, url } = record;
  if (
    typeof title !== "string" ||
    typeof body !== "string" ||
    typeof url !== "string"
  ) {
    throw new TypeError("A notification needs a title, body and url");
  }
  const target = new URL(url, APP_URL);
  const path = isAppUrl(target.href)
    ? `${target.pathname}${target.search}`
    : "/inbox";
  return { title: title.slice(0, 200), body: body.slice(0, 1000), url: path };
}
