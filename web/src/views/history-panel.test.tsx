// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { render, screen, waitFor } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { formatSize } from "../data/format";
import type { PageHistory, PageVersion } from "../data/history";
import { I18nProvider } from "../i18n/i18n";

import { HistoryPanel, type HistoryPanelProperties } from "./history-panel";

const OLD: PageVersion = {
  id: "v1",
  savedAt: new Date("2026-10-29T09:00:00Z"),
  size: 120,
  author: { type: "external" },
};
const NEW: PageVersion = {
  id: "v2",
  savedAt: new Date("2026-10-29T10:00:00Z"),
  size: 2048,
  author: { type: "user", user: "me" },
};

function fakeHistory() {
  let versions = [NEW, OLD];
  const restore = vi.fn<PageHistory["restore"]>(() => {
    versions = [
      { ...OLD, id: "v3", savedAt: new Date("2026-10-29T11:00:00Z") },
      ...versions,
    ];
    return Promise.resolve();
  });
  const history: PageHistory = {
    list: () => Promise.resolve(versions),
    read: (_page, id) =>
      Promise.resolve(id === "v1" ? "# Plan\n\nThe first draft\n" : undefined),
    restore,
  };
  return { history, restore };
}

function renderPanel(overrides: Partial<HistoryPanelProperties> = {}) {
  const { history, restore } = fakeHistory();
  const onClose = vi.fn<() => void>();
  const user = userEvent.setup();
  render(
    <I18nProvider>
      <HistoryPanel
        pageId="page-1"
        history={history}
        meId="me"
        canRestore
        onClose={onClose}
        {...overrides}
      />
    </I18nProvider>,
  );
  return { restore, onClose, user };
}

describe("version history", () => {
  it("lists versions newest first, naming who wrote them", async () => {
    renderPanel();
    const items = await screen.findAllByRole("button", { pressed: false });
    expect(items[0]?.textContent).toContain("You · 2.0 KB · current");
    expect(items[1]?.textContent).toContain("Edited outside Vellum · 120 B");
  });

  it(
    "shows a version's text and restores it after saving the editor " + "first",
    async () => {
      const saveFirst = vi.fn(() => Promise.resolve(true));
      const { restore, user } = renderPanel({ saveFirst });
      await user.click(
        await screen.findByRole("button", { name: /Edited outside Vellum/ }),
      );
      expect(await screen.findByText(/The first draft/)).toBeTruthy();

      await user.click(
        screen.getByRole("button", { name: "Restore this version" }),
      );
      await user.click(screen.getByRole("button", { name: "Restore" }));
      await waitFor(() => {
        expect(restore).toHaveBeenCalledWith("page-1", "v1");
      });
      expect(saveFirst).toHaveBeenCalledBefore(restore);
      expect(await screen.findByText(/Restored the version from/)).toBeTruthy();
      // The list reloads with the restored copy on top.
      expect(await screen.findAllByRole("listitem")).toHaveLength(3);
    },
  );

  it(
    "restores nothing when the editor's changes could not be " + "saved",
    async () => {
      const { restore, user } = renderPanel({
        saveFirst: () => Promise.resolve(false),
      });
      await user.click(
        await screen.findByRole("button", { name: /Edited outside Vellum/ }),
      );
      await user.click(
        await screen.findByRole("button", { name: "Restore this version" }),
      );
      await user.click(screen.getByRole("button", { name: "Restore" }));
      const alert = await screen.findByRole("alert");
      expect(alert.textContent).toContain("could not be saved");
      expect(restore).not.toHaveBeenCalled();
    },
  );

  it("offers no restore for the current version or to viewers", async () => {
    const { user } = renderPanel({ canRestore: false });
    await user.click(
      await screen.findByRole("button", { name: /Edited outside Vellum/ }),
    );
    expect(await screen.findByText(/The first draft/)).toBeTruthy();
    expect(
      screen.queryByRole("button", { name: "Restore this version" }),
    ).toBeNull();

    await user.click(screen.getByRole("button", { name: /current/ }));
    expect(await screen.findByText(/not text/)).toBeTruthy();
  });

  it("closes on Escape", async () => {
    const { onClose, user } = renderPanel();
    await screen.findAllByRole("listitem");
    await user.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalled();
  });
});

describe("formatSize", () => {
  it("picks the unit", () => {
    expect(formatSize(5)).toBe("5 B");
    expect(formatSize(1536)).toBe("1.5 KB");
    expect(formatSize(3 * 1024 * 1024)).toBe("3.0 MB");
  });
});
