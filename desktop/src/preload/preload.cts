// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// Exposes `window.vellumDesktop`, the `DesktopBridge` in
// web/src/platform/platform.ts. The preload runs sandboxed, so it may only
// require "electron"; channel names repeat src/shared/channels.ts.

import { contextBridge, ipcRenderer, type IpcRendererEvent } from "electron";

// The main process passes its version as `--vellum-version=<semver>`.
const versionArgument = process.argv.find((argument) =>
  argument.startsWith("--vellum-version="),
);
const version = versionArgument?.slice("--vellum-version=".length) ?? "0.0.0";
const os =
  process.platform === "darwin" || process.platform === "win32"
    ? process.platform
    : "linux";

contextBridge.exposeInMainWorld("vellumDesktop", {
  version,
  os,
  serverUrl: (): Promise<string> =>
    ipcRenderer.invoke("vellum:server-url") as Promise<string>,
  signIn: (): Promise<{ account: string }> =>
    ipcRenderer.invoke("vellum:sign-in") as Promise<{ account: string }>,
  signOut: (): Promise<void> =>
    ipcRenderer.invoke("vellum:sign-out") as Promise<void>,
  accessToken: (): Promise<string | undefined> =>
    ipcRenderer.invoke("vellum:access-token") as Promise<string | undefined>,
  openExternal: (url: string): Promise<void> =>
    ipcRenderer.invoke("vellum:open-external", url) as Promise<void>,
  notify: (notice: {
    title: string;
    body: string;
    url: string;
  }): Promise<void> =>
    ipcRenderer.invoke("vellum:notify", notice) as Promise<void>,
  privateSpaces: {
    deviceKey: (): Promise<{ publicKey: string; fingerprint: string }> =>
      ipcRenderer.invoke("vellum:private-device-key") as Promise<{
        publicKey: string;
        fingerprint: string;
      }>,
    enrol: (spaceId: string, label?: string): Promise<{ lines: string[] }> =>
      ipcRenderer.invoke("vellum:private-enrol", {
        spaceId,
        label,
      }) as Promise<{
        lines: string[];
      }>,
    reseal: (spaceId: string, trust?: string[]): Promise<{ lines: string[] }> =>
      ipcRenderer.invoke("vellum:private-reseal", {
        spaceId,
        trust,
      }) as Promise<{
        lines: string[];
      }>,
  },
  onOpenUrl: (listener: (path: string) => void): (() => void) => {
    const forward = (_event: IpcRendererEvent, path: unknown): void => {
      if (typeof path === "string") listener(path);
    };
    ipcRenderer.on("vellum:open-url", forward);
    return () => {
      ipcRenderer.removeListener("vellum:open-url", forward);
    };
  },
});
