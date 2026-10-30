// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import "fake-indexeddb/auto";
import { act, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { LocalStore } from "./local-store";
import { openLocalDatabase } from "./persistence";
import { FakeServer, freshDatabaseName, issueMutators } from "./test-support";
import type { ReadTx } from "./types";
import { useQuery } from "./use-query";

const titles = (tx: ReadTx) => tx.scan("issues").map(([key]) => key);

function IssueList({ store }: { readonly store: LocalStore }) {
  const keys = useQuery(store, ["issues"], titles);
  return (
    <ul>
      {keys.map((key) => (
        <li key={key}>{key}</li>
      ))}
    </ul>
  );
}

describe("useQuery", () => {
  it("renders local data at once and re-renders on a local edit", async () => {
    const database = await openLocalDatabase(freshDatabaseName());
    const store = await LocalStore.open({
      database,
      mutators: issueMutators,
      transport: new FakeServer(),
    });
    store.mutate("createIssue", { key: "ENG-1", title: "a" });
    render(<IssueList store={store} />);
    expect(
      screen.getAllByRole("listitem").map((item) => item.textContent),
    ).toEqual(["ENG-1"]);
    act(() => {
      store.mutate("createIssue", { key: "ENG-2", title: "b" });
    });
    expect(screen.getAllByRole("listitem")).toHaveLength(2);
  });
});
