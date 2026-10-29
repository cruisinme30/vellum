// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { act, render, renderHook, screen } from "@testing-library/react";
import type { ReactNode } from "react";
import { describe, expect, it, vi } from "vitest";

import type { CollabConnector } from "../editor/collab/provider";

import { createNorthstarStore } from "./mock/northstar";
import { useServices, useWorkspace, WorkspaceProvider } from "./workspace-context";

/** Swallows React's logged error for an expected throw. */
const quiet = () => {
  // Expected error output.
};

const collab: CollabConnector = {
  open: () => {
    throw new Error("not used in these tests");
  },
};

function PageCount() {
  const { pages } = useWorkspace();
  return <p>{pages.length} pages</p>;
}

describe("WorkspaceProvider", () => {
  it("hands the store and connector to useServices", () => {
    const store = createNorthstarStore();
    const wrapper = ({ children }: { readonly children: ReactNode }) => (
      <WorkspaceProvider store={store} collab={collab}>
        {children}
      </WorkspaceProvider>
    );
    const { result } = renderHook(() => useServices(), { wrapper });
    expect(result.current.store).toBe(store);
    expect(result.current.collab).toBe(collab);
  });

  it("re-renders useWorkspace consumers when the store changes", () => {
    const store = createNorthstarStore();
    const count = store.getSnapshot().pages.length;
    render(
      <WorkspaceProvider store={store} collab={collab}>
        <PageCount />
      </WorkspaceProvider>,
    );
    expect(screen.getByText(`${String(count)} pages`)).toBeDefined();
    act(() => {
      store.createPage({ spaceId: store.getSnapshot().spaces[0]?.id ?? "", parentId: undefined });
    });
    expect(screen.getByText(`${String(count + 1)} pages`)).toBeDefined();
  });

  it("throws a clear error outside a provider", () => {
    const silence = vi.spyOn(console, "error").mockImplementation(quiet);
    expect(() => renderHook(() => useServices())).toThrow(/needs a WorkspaceProvider/);
    silence.mockRestore();
  });
});
