// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { render, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { Awareness } from "y-protocols/awareness";
import * as Y from "yjs";

import { body, p } from "../content";
import { PageEditor } from "../page-editor";
import { memoryChannels, resolveGateway, settle } from "../test-editor";

import { createLocalConnector } from "./local-connector";
import { initials, inkOn, peersOf } from "./presence";
import type { LiveState } from "./provider";

/** An awareness with our state and two remote ones. */
function room() {
  const awareness = new Awareness(new Y.Doc());
  awareness.setLocalStateField("user", { name: "Me", color: "#1a73e8" });
  const remote = (clientId: number, user: unknown) => {
    awareness.states.set(clientId, { user });
    awareness.meta.set(clientId, { clock: 1, lastUpdated: Date.now() });
  };
  remote(11, { name: "Sam Okafor", color: "#386a20" });
  remote(12, { name: "Ana", color: "javascript:alert(1)" });
  remote(13, { name: "  " });
  remote(14, {});
  return awareness;
}

const live = (peers: [number, boolean][]): LiveState => ({
  status: "online",
  canWrite: true,
  peers: new Map(
    peers.map(([id, canWrite]) => [
      id,
      { user: `user-${String(id)}`, canWrite },
    ]),
  ),
});

describe("peers", () => {
  it("lists other named clients, sorted, with a safe colour", () => {
    const peers = peersOf(room());
    expect(peers.map((peer) => [peer.name, peer.color])).toEqual([
      ["Ana", "#4f5b92"],
      ["Sam Okafor", "#386a20"],
    ]);
  });

  it("shows only clients the server vouches for, with their access", () => {
    const peers = peersOf(room(), live([[11, false]]));
    expect(peers).toEqual([
      { clientId: 11, name: "Sam Okafor", color: "#386a20", canWrite: false },
    ]);
  });

  it("makes initials and picks a readable ink", () => {
    expect(initials("Sam Okafor")).toBe("SO");
    expect(initials("ana")).toBe("A");
    expect(initials("Priya van der Berg")).toBe("PB");
    expect(inkOn("#000000")).toBe("#ffffff");
    expect(inkOn("#ffeb3b")).toBe("#000000");
    expect(inkOn("#4f5b92")).toBe("#ffffff");
  });
});

describe("PresenceBar", () => {
  it("shows each editor the other one", async () => {
    const { factory } = memoryChannels();
    const connector = createLocalConnector({ channel: factory });
    const editor = (name: string) => (
      <div data-testid={name}>
        <PageEditor
          pageId="page-1"
          spaceId="space-1"
          seed={body(p("Hello"))}
          connector={connector}
          user={{ name, color: "#00658f" }}
          resolvePage={resolveGateway}
          onOpenPage={vi.fn<(id: string) => void>()}
        />
      </div>
    );
    render(
      <>
        {editor("Priya Shah")}
        {editor("Sam Okafor")}
      </>,
    );
    await settle();
    await waitFor(() => {
      expect(
        within(screen.getByTestId("Priya Shah")).getByRole("list", {
          name: "One other person on this page",
        }),
      ).toBeDefined();
    });
    const priyaSees = within(screen.getByTestId("Priya Shah")).getByRole(
      "listitem",
    );
    expect(priyaSees.getAttribute("aria-label")).toBe("Sam Okafor is editing");
    expect(priyaSees.textContent).toBe("SO");
    const samSees = within(screen.getByTestId("Sam Okafor")).getByRole(
      "listitem",
    );
    expect(samSees.getAttribute("aria-label")).toBe("Priya Shah is editing");
  });
});
