// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { act, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { useOnline, watchConnectivity } from "./use-online";

function Banner() {
  return <p>{useOnline() ? "online" : "offline"}</p>;
}

function goOffline(offline: boolean) {
  vi.spyOn(navigator, "onLine", "get").mockReturnValue(!offline);
  globalThis.dispatchEvent(new Event(offline ? "offline" : "online"));
}

describe("connectivity", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("re-renders when the connection drops and returns", () => {
    render(<Banner />);
    expect(screen.getByText("online")).toBeTruthy();
    act(() => {
      goOffline(true);
    });
    expect(screen.getByText("offline")).toBeTruthy();
    act(() => {
      goOffline(false);
    });
    expect(screen.getByText("online")).toBeTruthy();
  });

  it("tells watchers and stops after unsubscribing", () => {
    const onChange = vi.fn();
    const stop = watchConnectivity(onChange);
    goOffline(true);
    expect(onChange).toHaveBeenLastCalledWith(false);
    stop();
    goOffline(false);
    expect(onChange).toHaveBeenCalledTimes(1);
  });
});
