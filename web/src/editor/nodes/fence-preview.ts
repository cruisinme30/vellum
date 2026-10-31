// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Light client-side previews for data fences, shown while editing. The
 * renderer (vellum-render) draws the canonical SVG on save; these parsers
 * only need to be forgiving, never authoritative.
 */

/** A parsed `chart` fence with one series. */
export interface ChartPreview {
  readonly title: string;
  readonly type: string;
  readonly labels: readonly string[];
  readonly values: readonly number[];
}

const OPTIONS = new Set([
  "title",
  "type",
  "labels",
  "values",
  "unit",
  "ymin",
  "ymax",
  "legend",
  "colors",
]);
const NUMBER = /^-?\d+(?:\.\d+)?$/;

const list = (raw: string) =>
  raw
    .replace(/^\[/, "")
    .replace(/\]$/, "")
    .split(",")
    .map((s) => s.trim())
    .filter((s) => s !== "");

/** Splits `key: value` lines; lines without a colon are skipped. */
export function keyValues(body: string): [key: string, value: string][] {
  return body.split("\n").flatMap((line): [string, string][] => {
    const colon = line.indexOf(":");
    return colon <= 0
      ? []
      : [[line.slice(0, colon).trim(), line.slice(colon + 1).trim()]];
  });
}

/** Reads a chart fence. Pie-style `Label: n` lines become the series. */
export function parseChart(body: string): ChartPreview | undefined {
  const pairs = keyValues(body);
  const fields = new Map(pairs.filter(([key]) => OPTIONS.has(key)));
  const slices = pairs.filter(
    ([key, value]) => !OPTIONS.has(key) && NUMBER.test(value),
  );
  const labels = fields.has("labels")
    ? list(fields.get("labels") ?? "")
    : slices.map(([l]) => l);
  const values = fields.has("values")
    ? list(fields.get("values") ?? "").map(Number)
    : slices.map(([, v]) => Number(v));
  if (values.length === 0 || values.some((v) => !Number.isFinite(v))) {
    return undefined;
  }
  return {
    title: fields.get("title") ?? "",
    type: fields.get("type") ?? "bar",
    labels,
    values,
  };
}

/**
 * Parses a CSV body: the first row is the header. Quotes are not special in v1.
 */
export function parseCsv(body: string): string[][] {
  return body
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line !== "")
    .map((line) => line.split(",").map((cell) => cell.trim()));
}

/**
 * Kroki-rendered diagram languages: drawn by the sidecar when it is enabled.
 */
export const KROKI = new Set([
  "d2",
  "plantuml",
  "graphviz",
  "excalidraw",
  "drawio",
]);
