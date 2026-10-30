// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { render, screen } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { ApiError } from "../api/http";
import { failureText } from "../data/failure";
import { I18nProvider } from "../i18n/i18n";

import { CopyDiagnostic } from "./copy-diagnostic";

describe("CopyDiagnostic", () => {
  it("copies the request ID, route and time of the failure shown", async () => {
    const user = userEvent.setup();
    const writeText = vi
      .spyOn(navigator.clipboard, "writeText")
      .mockResolvedValue();
    const message = failureText(
      new ApiError("internal", "Saving failed.", {
        requestId: "req-7",
        route: "POST /api/v1/x",
      }),
    );
    render(
      <I18nProvider>
        <p role="alert">
          {message}
          <CopyDiagnostic message={message} />
        </p>
      </I18nProvider>,
    );
    await user.click(screen.getByRole("button", { name: "Copy diagnostic" }));
    const copied = writeText.mock.calls[0]?.[0] ?? "";
    expect(copied).toContain("Request ID: req-7");
    expect(copied).toContain("Route: POST /api/v1/x");
    expect(copied).toMatch(/Time: \d{4}-\d\d-\d\dT/);
    expect(screen.getByRole("status").textContent).toBe("Diagnostic copied");
  });

  it(
    "shows the text to copy by hand when the clipboard is " + "blocked",
    async () => {
      const user = userEvent.setup();
      vi.spyOn(navigator.clipboard, "writeText").mockRejectedValue(
        new Error("denied"),
      );
      render(
        <I18nProvider>
          <CopyDiagnostic message="Something broke" />
        </I18nProvider>,
      );
      await user.click(screen.getByRole("button", { name: "Copy diagnostic" }));
      const manual = await screen.findByRole("textbox", {
        name: "Diagnostic to copy",
      });
      expect((manual as HTMLTextAreaElement).value).toContain(
        "Message: Something broke",
      );
    },
  );
});
