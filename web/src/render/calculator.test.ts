// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";

import { evaluate, formatValue, hydrateCalculators } from "./calculator";

const variables = new Map([
  ["units", 100],
  ["price", 12],
  ["größe", 3],
]);

describe("evaluate", () => {
  it.each([
    ["units * price", 1200],
    ["1 + 2 * 3", 7],
    ["(1 + 2) * 3", 9],
    ["-units + 1", -99],
    ["10 % 4", 2],
    ["6 ÷ 4", 1.5],
    ["2 × größe", 6],
    [".5 + 1", 1.5],
    // Spaces are dropped first, as the server does.
    ["1 2", 12],
  ])("%s is %d, as the server computes it", (formula, value) => {
    expect(evaluate(formula, variables)).toBe(value);
  });

  it.each([
    ["units / 0", "division by zero"],
    ["nope + 1", "unknown input `nope`"],
    ["(1 + 2", "missing `)`"],
    ["1 +", "formula ends early"],
    ["1.2.3", "bad number `1.2.3`"],
    ["1 (2)", "unexpected `(`"],
    ["(".repeat(70) + "1" + ")".repeat(70), "formula is nested too deeply"],
  ])("refuses %s", (formula, message) => {
    expect(() => evaluate(formula, variables)).toThrow(message);
  });
});

describe("formatValue", () => {
  it.each([
    [1200, "", "1,200"],
    [1_234_567.891, "USD", "1,234,567.89 USD"],
    [-4321.5, "", "-4,321.5"],
    [12.5, "%", "12.5%"],
    [0, "", "0"],
    [-0.001, "", "0"],
  ])("shows %d %s as %s", (value, unit, text) => {
    expect(formatValue(value, unit)).toBe(text);
  });
});

describe("hydrateCalculators", () => {
  it("recomputes results as a slider moves, and stops when unwired", () => {
    const root = document.createElement("div");
    root.innerHTML = `<div class="vl-calculator">
      <label for="u"><span>Units</span><input type="range" id="u" data-key="units" min="0" max="1000" step="10" value="100"><output for="u">100</output></label>
      <label for="p"><span>Price</span><input type="range" id="p" data-key="price" min="0" max="24" value="12"><output for="p">12</output></label>
      <div class="vl-calc-result" data-key="total" data-formula="units * price" data-unit="USD"><span>Total</span> <output>1,200 USD</output></div>
      <div class="vl-calc-result" data-key="half" data-formula="total / 2" data-unit=""><span>Half</span> <output>600</output></div>
      <div class="vl-calc-result" data-key="bad" data-formula="units / 0" data-unit=""><span>Bad</span> <output>–</output></div>
    </div>`;
    document.body.append(root);
    const unwire = hydrateCalculators(root);
    const units = root.querySelector<HTMLInputElement>("#u");
    if (!units) throw new Error("no slider");
    units.value = "250";
    units.dispatchEvent(new Event("input", { bubbles: true }));
    const outputs = [...root.querySelectorAll("output")].map(
      (o) => o.textContent,
    );
    expect(outputs).toEqual(["250", "12", "3,000 USD", "1,500", "–"]);

    unwire();
    units.value = "10";
    units.dispatchEvent(new Event("input", { bubbles: true }));
    expect(root.querySelector(".vl-calc-result output")?.textContent).toBe(
      "3,000 USD",
    );
    root.remove();
  });
});
