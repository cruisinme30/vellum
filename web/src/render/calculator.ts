// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Makes the server's calculator blocks interactive. The renderer draws
 * each `vl-calculator` with range inputs (`data-key`) and results
 * (`data-formula`, `data-unit`) computed at their starting values; moving a
 * slider recomputes the results here with the same arithmetic as
 * `vellum_render::model::evaluate` and the same number format.
 */

type Variables = ReadonlyMap<string, number>;

const isDigit = (c: string | undefined) =>
  c !== undefined && c >= "0" && c <= "9";
const isNumberChar = (c: string | undefined) => isDigit(c) || c === ".";
const isNameStart = (c: string | undefined) =>
  c !== undefined && /^[\p{L}_]$/u.test(c);
const isNameChar = (c: string | undefined) =>
  c !== undefined && /^[\p{L}\p{N}_]$/u.test(c);

/** One pass over a formula, mirroring the Rust parser's recursive descent. */
class Formula {
  private readonly tokens: string[] = [];
  private pos = 0;
  private depth = 0;

  constructor(
    text: string,
    private readonly variables: Variables,
  ) {
    for (const c of text) {
      if (c.trim() !== "") this.tokens.push(c);
    }
  }

  value(): number {
    const v = this.expr();
    if (this.pos !== this.tokens.length) {
      throw new Error(`unexpected \`${this.peek() ?? " "}\``);
    }
    if (!Number.isFinite(v)) throw new Error("result is not a number");
    return v;
  }

  private peek(): string | undefined {
    return this.tokens[this.pos];
  }

  /** Reads characters while `test` holds. */
  private take(test: (c: string | undefined) => boolean): string {
    const start = this.pos;
    while (test(this.peek())) this.pos += 1;
    return this.tokens.slice(start, this.pos).join("");
  }

  private expr(): number {
    let v = this.term();
    for (let op = this.peek(); op === "+" || op === "-"; op = this.peek()) {
      this.pos += 1;
      const r = this.term();
      v = op === "+" ? v + r : v - r;
    }
    return v;
  }

  private term(): number {
    let v = this.factor();
    for (
      let op = this.peek();
      op !== undefined && "*/%×÷".includes(op);
      op = this.peek()
    ) {
      this.pos += 1;
      v = combine(v, op, this.factor());
    }
    return v;
  }

  private factor(): number {
    this.depth += 1;
    if (this.depth > 64) throw new Error("formula is nested too deeply");
    const v = this.primary();
    this.depth -= 1;
    return v;
  }

  private primary(): number {
    const c = this.peek();
    if (c === "-") {
      this.pos += 1;
      return -this.factor();
    }
    if (c === "(") {
      this.pos += 1;
      const v = this.expr();
      if (this.peek() !== ")") throw new Error("missing `)`");
      this.pos += 1;
      return v;
    }
    if (isNumberChar(c)) return parseNumber(this.take(isNumberChar));
    if (isNameStart(c)) {
      const name = this.take(isNameChar);
      const found = this.variables.get(name);
      if (found === undefined) throw new Error(`unknown input \`${name}\``);
      return found;
    }
    throw new Error("formula ends early");
  }
}

function combine(left: number, op: string, right: number): number {
  if (op === "*" || op === "×") return left * right;
  if (right === 0) throw new Error("division by zero");
  return op === "%" ? left % right : left / right;
}

/** Digits with at most one point, as Rust's `f64` parser takes them. */
function parseNumber(text: string): number {
  const points = text.split(".").length - 1;
  if (points > 1 || text === ".") throw new Error(`bad number \`${text}\``);
  return Number(text);
}

/**
 * Evaluates `+ - * / % ( )` over numbers and named inputs; throws on bad input.
 */
export function evaluate(formula: string, variables: Variables): number {
  return new Formula(formula, variables).value();
}

/** `vellum_render::html::num`: at most two decimals, no trailing zeros. */
function plain(v: number): string {
  if (!Number.isFinite(v)) return "0";
  let s = v.toFixed(2);
  if (s.includes(".")) {
    while (s.endsWith("0")) s = s.slice(0, -1);
    if (s.endsWith(".")) s = s.slice(0, -1);
  }
  return s === "-0" ? "0" : s;
}

/** Puts a comma between each group of three digits. */
function group(digits: string): string {
  let out = "";
  for (let index = 0; index < digits.length; index += 1) {
    if (index > 0 && (digits.length - index) % 3 === 0) out += ",";
    out += digits.charAt(index);
  }
  return out;
}

/**
 * `vellum_render::figure::chart::format_value`: thousands separators and a
 * unit.
 */
export function formatValue(v: number, unit: string): string {
  const s = plain(Math.round(v * 100) / 100);
  const point = s.indexOf(".");
  const int = point === -1 ? s : s.slice(0, point);
  const frac = point === -1 ? "" : s.slice(point);
  const sign = int.startsWith("-") ? "-" : "";
  const text = sign + group(int.slice(sign.length)) + frac;
  if (unit === "") return text;
  return unit === "%" ? `${text}%` : `${text} ${unit}`;
}

/** Recomputes one calculator's results from its sliders. */
function recompute(calculator: Element) {
  const variables = new Map<string, number>();
  for (const input of calculator.querySelectorAll<HTMLInputElement>(
    "input[data-key]",
  )) {
    const key = input.dataset["key"] ?? "";
    variables.set(key, Number(input.value));
    const shown = input.parentElement?.querySelector("output");
    if (shown) shown.textContent = plain(Number(input.value));
  }
  for (const result of calculator.querySelectorAll<HTMLElement>(
    ".vl-calc-result",
  )) {
    const output = result.querySelector("output");
    if (!output) continue;
    try {
      const v = evaluate(result.dataset["formula"] ?? "", variables);
      output.textContent = formatValue(v, result.dataset["unit"] ?? "");
      // Later results may use earlier ones.
      variables.set(result.dataset["key"] ?? "", v);
    } catch {
      output.textContent = "–";
    }
  }
}

/**
 * Wires every calculator under `root`; returns a function that unwires them.
 */
export function hydrateCalculators(root: HTMLElement): () => void {
  const onInput = (event: Event) => {
    const target = event.target;
    if (!(target instanceof HTMLInputElement)) return;
    const calculator = target.closest(".vl-calculator");
    if (calculator && root.contains(calculator)) recompute(calculator);
  };
  root.addEventListener("input", onInput);
  return () => {
    root.removeEventListener("input", onInput);
  };
}
