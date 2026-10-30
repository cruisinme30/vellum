// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { telemetry } from "../telemetry";

/** Update checks fail while offline; the next one retries. */
function ignore(): void {
  // Nothing to do.
}

/** What the app learns about the service worker. */
export interface WorkerHandle {
  /** True when a new version is installed and waiting for the person. */
  readonly updateReady: boolean;
  /** Activates the waiting version and reloads once it takes control. */
  applyUpdate(): void;
}

/** Options for {@link registerServiceWorker}. */
export interface RegisterOptions {
  /** Called when a new version finishes installing. */
  readonly onUpdateReady?: () => void;
  /** Defaults to `navigator.serviceWorker`. */
  readonly container?: ServiceWorkerContainer;
  /** Defaults to `location.reload`. */
  readonly reload?: () => void;
  /** How often to look for a new version; defaults to hourly. */
  readonly checkEveryMs?: number;
}

/**
 * Registers `/sw.js` so the app starts offline and can receive pushes.
 * Resolves to `undefined` where service workers are unavailable (old
 * browsers, private modes, the desktop shell's `app:` scheme).
 */
export async function registerServiceWorker(
  options: RegisterOptions = {},
): Promise<WorkerHandle | undefined> {
  const container =
    options.container ?? (globalThis.navigator as Navigator | undefined)?.serviceWorker;
  if (container === undefined) return undefined;
  const reload =
    options.reload ??
    (() => {
      location.reload();
    });
  // Typed as always a registration, but a browser that blocks service
  // workers by policy (and Playwright's `serviceWorkers: "block"`)
  // resolves to nothing; without this check the app crashed (D285).
  let registered: unknown;
  try {
    registered = await container.register("/sw.js", { scope: "/", updateViaCache: "none" });
  } catch {
    registered = undefined;
  }
  if (registered === undefined || registered === null) {
    telemetry().count("vellum.client.sw.register_failed");
    return undefined;
  }
  const registration = registered as ServiceWorkerRegistration;
  let updateReady = registration.waiting !== null && container.controller !== null;
  const announce = () => {
    updateReady = true;
    options.onUpdateReady?.();
  };
  if (updateReady) announce();
  registration.addEventListener("updatefound", () => {
    const installing = registration.installing;
    installing?.addEventListener("statechange", () => {
      // An update is a new worker next to the one that controls the page.
      // The first install has no other controller: nothing to update. Its
      // own controller does not count either: WebKit can deliver the
      // "installed" event only after the worker activated and its
      // `clients.claim()` made it the controller, and treating that as an
      // update reloaded the page during its first load.
      const controller = container.controller;
      if (installing.state === "installed" && controller !== null && controller !== installing) {
        announce();
      }
    });
  });
  setInterval(() => {
    registration.update().catch(ignore);
  }, options.checkEveryMs ?? 3_600_000);
  let reloading = false;
  container.addEventListener("controllerchange", () => {
    if (reloading || !updateReady) return;
    reloading = true;
    reload();
  });
  return {
    get updateReady() {
      return updateReady;
    },
    applyUpdate() {
      registration.waiting?.postMessage("skip-waiting");
    },
  };
}
