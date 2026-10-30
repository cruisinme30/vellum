// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { fireEvent, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { renderApp } from "../test/render-app";

import { exportTheme, THEME_FORMAT } from "./theme-file";

const root = () => document.documentElement;

const noop = () => {
  // Stands in for the browser's download.
};

afterEach(() => {
  localStorage.clear();
  vi.unstubAllGlobals();
  for (const key of ["theme", "density", "motion", "font", "customTheme"]) {
    Reflect.deleteProperty(root().dataset, key);
  }
  root().removeAttribute("style");
});

function editor() {
  return within(screen.getByRole("region", { name: "Custom theme" }));
}

describe("Settings → Appearance", () => {
  it("changes text size, font, density and motion at once", async () => {
    const { user } = renderApp("/settings/appearance");
    expect(
      screen.getByRole("heading", { level: 1, name: "Appearance" }),
    ).toBeDefined();
    await user.click(screen.getByRole("radio", { name: "125 %" }));
    await user.click(screen.getByRole("radio", { name: "OpenDyslexic" }));
    await user.click(screen.getByRole("radio", { name: "Spacious" }));
    await user.click(screen.getByRole("radio", { name: "Reduce motion" }));
    expect(root().style.fontSize).toBe("125%");
    expect(root().dataset["font"]).toBe("dyslexic");
    expect(root().dataset["density"]).toBe("spacious");
    expect(root().dataset["motion"]).toBe("reduce");
    expect(
      JSON.parse(localStorage.getItem("vellum.appearance") ?? "{}"),
    ).toMatchObject({
      textSize: 125,
      font: "dyslexic",
    });
  });

  it("checks contrast live and only lets a passing theme be used", async () => {
    const { user } = renderApp("/settings/appearance");
    const custom = editor();
    expect(custom.getAllByRole("status")[0]?.textContent).toMatch(
      /^All \d+ colour pairs pass\.$/,
    );
    // Make body text almost the colour of the page.
    const input = custom.getByLabelText("on-surface", { selector: "input" });
    fireEvent.change(input, { target: { value: "#f0f0f0" } });
    expect(custom.getAllByRole("status")[0]?.textContent).toMatch(
      /too little contrast/,
    );
    expect(
      custom.getByRole("button", { name: "Use this theme" }),
    ).toHaveProperty("disabled", true);
    await user.click(custom.getByRole("button", { name: "Reset colours" }));
    await user.click(custom.getByRole("button", { name: "Use this theme" }));
    expect(root().dataset["customTheme"]).toBe("My theme");
    expect(
      screen.getAllByRole("radio", { name: "Custom: My theme" })[0],
    ).toHaveProperty("checked", true);
  });

  it("imports a theme file and explains a refused one", async () => {
    const { user } = renderApp("/settings/appearance");
    const custom = editor();
    const input = custom.getByLabelText("Import JSON", { selector: "input" });
    const good = exportTheme({
      name: "Ink",
      base: "sepia",
      seed: "#225588",
      tokens: {},
    });
    await user.upload(
      input,
      new File([good], "ink.json", { type: "application/json" }),
    );
    expect(await custom.findByText(/Imported "Ink"/)).toBeDefined();
    expect(custom.getByLabelText("Theme name")).toHaveProperty("value", "Ink");
    const bad = JSON.stringify({
      format: THEME_FORMAT,
      name: "Fog",
      base: "light",
      seed: "#225588",
      tokens: { "on-surface": "#eeeeee" },
    });
    await user.upload(
      input,
      new File([bad], "fog.json", { type: "application/json" }),
    );
    const alert = await custom.findByRole("alert");
    expect(alert.textContent).toMatch(/not imported/);
  });

  it("exports the draft as a JSON file", async () => {
    const created: Blob[] = [];
    vi.stubGlobal("URL", {
      ...URL,
      createObjectURL: (blob: Blob) => {
        created.push(blob);
        return "blob:theme";
      },
      revokeObjectURL: vi.fn(),
    });
    const click = vi
      .spyOn(HTMLAnchorElement.prototype, "click")
      .mockImplementation(noop);
    const { user } = renderApp("/settings/appearance");
    await user.click(editor().getByRole("button", { name: "Export JSON" }));
    expect(click).toHaveBeenCalled();
    const text = await created[0]?.text();
    expect(JSON.parse(text ?? "{}")).toMatchObject({
      format: THEME_FORMAT,
      name: "My theme",
    });
    click.mockRestore();
  });
});
