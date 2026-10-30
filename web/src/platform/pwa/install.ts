// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/** The non-standard event Chromium fires when the app can be installed. */
interface InstallPromptEvent extends Event {
  prompt(): Promise<void>;
  readonly userChoice: Promise<{ outcome: "accepted" | "dismissed" }>;
}

/** Lets the app offer "Install Vellum" from its own menu. */
export interface Installer {
  /** True once the browser has offered installation. */
  readonly available: boolean;
  /**
   * Shows the browser's install dialog; resolves to whether it was accepted.
   */
  prompt(): Promise<boolean>;
}

/**
 * Captures the browser's install offer so the app can show it later from
 * a button instead of the browser's own banner. Safari has no such event;
 * there, the help page explains "Add to Home Screen".
 *
 * @param onAvailable - called when the offer arrives
 */
export function captureInstallPrompt(
  target: EventTarget = globalThis,
  onAvailable?: () => void,
): Installer {
  let pending: InstallPromptEvent | undefined;
  target.addEventListener("beforeinstallprompt", (event) => {
    event.preventDefault();
    pending = event as InstallPromptEvent;
    onAvailable?.();
  });
  target.addEventListener("appinstalled", () => {
    pending = undefined;
  });
  return {
    get available() {
      return pending !== undefined;
    },
    async prompt() {
      const offer = pending;
      if (offer === undefined) return false;
      pending = undefined;
      await offer.prompt();
      const choice = await offer.userChoice;
      return choice.outcome === "accepted";
    },
  };
}

/** Whether the app runs installed (standalone) rather than in a tab. */
export function isStandalone(
  scope: { matchMedia?: (query: string) => MediaQueryList } = globalThis,
): boolean {
  return scope.matchMedia?.("(display-mode: standalone)").matches === true;
}
