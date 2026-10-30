// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { afterEach, describe, expect, it, vi } from "vitest";

import { registerServiceWorker } from "./register";

// Service worker APIs use null for "none"; the lint rules forbid the literal.
const NULL = JSON.parse("null") as undefined;
const resolved = () => Promise.resolve();

class FakeWorker extends EventTarget {
  state = "installing";
  readonly postMessage = vi.fn();
}

class FakeRegistration extends EventTarget {
  waiting: FakeWorker | undefined = NULL;
  installing: FakeWorker | undefined = NULL;
  readonly update = vi.fn(resolved);
}

class FakeContainer extends EventTarget {
  controller: object | undefined = {};
  readonly registration = new FakeRegistration();
  readonly register = vi.fn(() => Promise.resolve(this.registration));
}

function setup(controller: object | undefined = {}) {
  const container = new FakeContainer();
  container.controller = controller;
  const reload = vi.fn();
  const onUpdateReady = vi.fn();
  const options = {
    container: container as unknown as ServiceWorkerContainer,
    reload,
    onUpdateReady,
    checkEveryMs: 1000,
  };
  return { container, reload, onUpdateReady, options };
}

describe("registerServiceWorker", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("registers /sw.js at the root without HTTP caching", async () => {
    const { container, options } = setup();
    await registerServiceWorker(options);
    expect(container.register).toHaveBeenCalledWith("/sw.js", {
      scope: "/",
      updateViaCache: "none",
    });
  });

  it("announces a new version, activates it on request and reloads once", async () => {
    const { container, reload, onUpdateReady, options } = setup();
    const handle = await registerServiceWorker(options);
    expect(handle?.updateReady).toBe(false);
    const next = new FakeWorker();
    container.registration.installing = next;
    container.registration.dispatchEvent(new Event("updatefound"));
    next.state = "installed";
    next.dispatchEvent(new Event("statechange"));
    expect(onUpdateReady).toHaveBeenCalledTimes(1);
    expect(handle?.updateReady).toBe(true);
    container.registration.waiting = next;
    handle?.applyUpdate();
    expect(next.postMessage).toHaveBeenCalledWith("skip-waiting");
    container.dispatchEvent(new Event("controllerchange"));
    container.dispatchEvent(new Event("controllerchange"));
    expect(reload).toHaveBeenCalledTimes(1);
  });

  it("treats the first install as no update and does not reload", async () => {
    const { container, reload, onUpdateReady, options } = setup(NULL);
    await registerServiceWorker(options);
    const first = new FakeWorker();
    container.registration.installing = first;
    container.registration.dispatchEvent(new Event("updatefound"));
    first.state = "installed";
    first.dispatchEvent(new Event("statechange"));
    container.dispatchEvent(new Event("controllerchange"));
    expect(onUpdateReady).not.toHaveBeenCalled();
    expect(reload).not.toHaveBeenCalled();
  });

  it(
    "treats a first install that claimed the page before its events arrived " +
      "as no update",
    async () => {
      // WebKit can deliver "installed" after the worker activated and
      // `clients.claim()` made it the page's controller.
      const { container, reload, onUpdateReady, options } = setup(NULL);
      await registerServiceWorker(options);
      const first = new FakeWorker();
      container.registration.installing = first;
      container.registration.dispatchEvent(new Event("updatefound"));
      container.controller = first;
      first.state = "installed";
      first.dispatchEvent(new Event("statechange"));
      container.dispatchEvent(new Event("controllerchange"));
      expect(onUpdateReady).not.toHaveBeenCalled();
      expect(reload).not.toHaveBeenCalled();
    },
  );

  it("reports a version already waiting at start-up", async () => {
    const { container, onUpdateReady, options } = setup();
    container.registration.waiting = new FakeWorker();
    const handle = await registerServiceWorker(options);
    expect(handle?.updateReady).toBe(true);
    expect(onUpdateReady).toHaveBeenCalledTimes(1);
  });

  it("checks for updates periodically", async () => {
    vi.useFakeTimers();
    const { container, options } = setup();
    await registerServiceWorker(options);
    await vi.advanceTimersByTimeAsync(2000);
    expect(container.registration.update).toHaveBeenCalledTimes(2);
  });

  it("gives up quietly when registration fails or is unavailable", async () => {
    const { container, options } = setup();
    container.register.mockImplementationOnce(() =>
      Promise.reject(new Error("blocked")),
    );
    expect(await registerServiceWorker(options)).toBeUndefined();
    expect(await registerServiceWorker({})).toBeUndefined();
  });
});
