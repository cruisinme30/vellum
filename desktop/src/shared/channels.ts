// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * IPC channel names shared by the main process and the preload script.
 * The sandboxed preload cannot import this file, so it repeats the
 * strings; `channels.test.ts` keeps the two in step.
 */
export const Channel = {
  serverUrl: "vellum:server-url",
  signIn: "vellum:sign-in",
  signOut: "vellum:sign-out",
  accessToken: "vellum:access-token",
  openExternal: "vellum:open-external",
  notify: "vellum:notify",
  /** Main to renderer: a `vellum://` link was opened. */
  openUrl: "vellum:open-url",
} as const;

export type ChannelName = (typeof Channel)[keyof typeof Channel];
