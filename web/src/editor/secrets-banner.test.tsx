// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { act, fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { SecretScanResult } from "../api/api.gen";
import { I18nProvider } from "../i18n/i18n";

import { pageText, SecretsBanner, type ScanSecrets } from "./secrets-banner";
import { createTestEditor } from "./test-editor";

/** A GitHub token shaped string, built so no source file holds one. */
const TOKEN = ["ghp", "Zq9Xw8Vu7Ts6Rq5Po4Nm3Lk2Ji1Hg0FeDcBa"].join("_");

const content = {
  type: "doc",
  content: [
    { type: "paragraph", content: [{ type: "text", text: "Deploys" }] },
    {
      type: "paragraph",
      content: [{ type: "text", text: `The bot uses ${TOKEN} to push.` }],
    },
  ],
};

/** What the server answers for the page as written. */
const answer = (policy: SecretScanResult["policy"]): SecretScanResult => ({
  policy,
  findings: [
    {
      rule: "github-token",
      line: 2,
      column: 14,
      length: TOKEN.length,
      redacted: "ghp_… (40 chars)",
    },
  ],
});

function show(scan: ScanSecrets) {
  const editor = createTestEditor({ content });
  render(
    <I18nProvider>
      <SecretsBanner editor={editor} scan={scan} />
    </I18nProvider>,
  );
  return editor;
}

describe("SecretsBanner", () => {
  it("sends one line per block", () => {
    const editor = createTestEditor({ content });
    expect(pageText(editor.state.doc).text).toBe(
      `Deploys\nThe bot uses ${TOKEN} to push.`,
    );
  });

  it("names the kind and line, never the secret, and removes it", async () => {
    const scan = vi.fn<ScanSecrets>().mockResolvedValue(answer("warn"));
    const editor = show(scan);
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("GitHub token on line 2");
    expect(alert.textContent).not.toContain(TOKEN);
    expect(scan).toHaveBeenCalledWith(
      `Deploys\nThe bot uses ${TOKEN} to push.`,
    );
    act(() => {
      fireEvent.click(screen.getByRole("button", { name: "Remove" }));
    });
    expect(editor.getText()).not.toContain(TOKEN);
    expect(editor.getText()).toContain("The bot uses  to push.");
  });

  it(
    "lets the writer say it is not a secret, unless saves are " + "blocked",
    async () => {
      show(vi.fn<ScanSecrets>().mockResolvedValue(answer("warn")));
      await screen.findByRole("alert");
      act(() => {
        fireEvent.click(
          screen.getByRole("button", { name: "It's not a secret" }),
        );
      });
      expect(screen.queryByRole("alert")).toBeNull();
    },
  );

  it("says a blocked page will not save", async () => {
    show(vi.fn<ScanSecrets>().mockResolvedValue(answer("block")));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("can't be saved");
    expect(
      screen.queryByRole("button", { name: "It's not a secret" }),
    ).toBeNull();
  });

  it("shows nothing when the page is clean or the check fails", async () => {
    const clean = vi
      .fn<ScanSecrets>()
      .mockResolvedValue({ policy: "warn", findings: [] });
    show(clean);
    await vi.waitFor(() => {
      expect(clean).toHaveBeenCalled();
    });
    expect(screen.queryByRole("alert")).toBeNull();
  });
});
