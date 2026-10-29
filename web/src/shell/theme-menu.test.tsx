// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { fireEvent, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { renderWithProviders } from "../test/render-app";
import { DEFAULT_SEED } from "../theme/themes";

import { ThemeSettings } from "./theme-menu";

const saved = (): unknown => JSON.parse(localStorage.getItem("vellum.appearance") ?? "{}");
const root = document.documentElement;

describe("ThemeSettings", () => {
  it("starts from the system theme, comfortable density and the default seed", () => {
    renderWithProviders(<ThemeSettings />);
    expect(screen.getByRole("radio", { name: "Match the system" })).toHaveProperty("checked", true);
    expect(screen.getByRole("radio", { name: "Comfortable" })).toHaveProperty("checked", true);
    expect(screen.getByLabelText("Accent colour")).toHaveProperty("value", DEFAULT_SEED);
    expect(screen.getByRole("button", { name: "Reset colour" })).toHaveProperty("disabled", true);
    expect(root.dataset["theme"]).toBe("light");
  });

  it("switches the theme, saves it and themes the document", async () => {
    const { user } = renderWithProviders(<ThemeSettings />);
    const before = root.style.getPropertyValue("--md-surface");
    await user.click(screen.getByRole("radio", { name: "Dark" }));
    expect(screen.getByRole("radio", { name: "Dark" })).toHaveProperty("checked", true);
    expect(saved()).toMatchObject({ theme: "dark" });
    expect(root.dataset["theme"]).toBe("dark");
    expect(root.style.colorScheme).toBe("dark");
    expect(root.style.getPropertyValue("--md-surface")).not.toBe(before);
  });

  it("switches the density", async () => {
    const { user } = renderWithProviders(<ThemeSettings />);
    await user.click(screen.getByRole("radio", { name: "Compact" }));
    expect(saved()).toMatchObject({ density: "compact" });
    expect(root.dataset["density"]).toBe("compact");
  });

  it("changes and resets the seed colour", async () => {
    const { user } = renderWithProviders(<ThemeSettings />);
    const primary = root.style.getPropertyValue("--md-primary");
    fireEvent.change(screen.getByLabelText("Accent colour"), { target: { value: "#b3261e" } });
    expect(saved()).toMatchObject({ seed: "#b3261e" });
    expect(root.style.getPropertyValue("--md-primary")).not.toBe(primary);
    const reset = screen.getByRole("button", { name: "Reset colour" });
    expect(reset).toHaveProperty("disabled", false);
    await user.click(reset);
    expect(saved()).not.toHaveProperty("seed");
    expect(screen.getByLabelText("Accent colour")).toHaveProperty("value", DEFAULT_SEED);
    expect(reset).toHaveProperty("disabled", true);
  });

  it("links to the full appearance settings", () => {
    renderWithProviders(<ThemeSettings />);
    expect(
      screen.getByRole("link", { name: "All appearance settings" }).getAttribute("href"),
    ).toBe("/settings/appearance");
  });
});
