// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/** The slice of electron-updater's `autoUpdater` the app uses. */
export interface Updater {
  autoDownload: boolean;
  autoInstallOnAppQuit: boolean;
  checkForUpdates(): Promise<unknown>;
  on(event: "update-downloaded", listener: (info: { version: string }) => void): unknown;
  on(event: "error", listener: (error: Error) => void): unknown;
}

export interface UpdateOptions {
  readonly intervalMs?: number;
  readonly onReady: (version: string) => void;
  readonly onError: (error: Error) => void;
  readonly setInterval?: (callback: () => void, ms: number) => unknown;
}

const FOUR_HOURS = 4 * 60 * 60_000;

/**
 * Checks GitHub Releases for signed updates at start-up and every four
 * hours, downloads in the background and installs on quit. The person is
 * told an update is ready; nothing restarts under them.
 */
export function startUpdates(updater: Updater, options: UpdateOptions): void {
  updater.autoDownload = true;
  updater.autoInstallOnAppQuit = true;
  updater.on("update-downloaded", (info) => {
    options.onReady(info.version);
  });
  updater.on("error", options.onError);
  const check = (): void => {
    updater.checkForUpdates().catch(options.onError);
  };
  check();
  (options.setInterval ?? setInterval)(check, options.intervalMs ?? FOUR_HOURS);
}
