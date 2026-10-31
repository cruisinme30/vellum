// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { render, screen } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";

import { Popover } from "./popover";

function Harness({ onClose }: { readonly onClose: () => void }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button
        type="button"
        onClick={() => {
          setOpen(true);
        }}
      >
        Open
      </button>
      <button type="button">Elsewhere</button>
      {open && (
        <Popover
          label="Settings"
          onClose={() => {
            onClose();
            setOpen(false);
          }}
        >
          <p>Text</p>
          <button type="button">Inside</button>
        </Popover>
      )}
    </>
  );
}

function renderPopover() {
  const onClose = vi.fn<() => void>();
  const user = userEvent.setup();
  render(<Harness onClose={onClose} />);
  return { onClose, user };
}

describe("Popover", () => {
  it("is a labelled dialog that focuses its first control", async () => {
    const { user } = renderPopover();
    await user.click(screen.getByRole("button", { name: "Open" }));
    const dialog = screen.getByRole("dialog", { name: "Settings" });
    expect(dialog.className).toBe("vl-popover ");
    expect(document.activeElement).toBe(
      screen.getByRole("button", { name: "Inside" }),
    );
  });

  it("stays open for clicks and keys inside it", async () => {
    const { user, onClose } = renderPopover();
    await user.click(screen.getByRole("button", { name: "Open" }));
    await user.click(screen.getByRole("button", { name: "Inside" }));
    await user.keyboard("a");
    expect(onClose).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog")).toBeDefined();
  });

  it("closes on Escape and returns focus to its opener", async () => {
    const { user, onClose } = renderPopover();
    const opener = screen.getByRole("button", { name: "Open" });
    await user.click(opener);
    await user.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalledOnce();
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(document.activeElement).toBe(opener);
  });

  it("closes on a click outside", async () => {
    const { user, onClose } = renderPopover();
    await user.click(screen.getByRole("button", { name: "Open" }));
    await user.click(screen.getByRole("button", { name: "Elsewhere" }));
    expect(onClose).toHaveBeenCalledOnce();
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});
