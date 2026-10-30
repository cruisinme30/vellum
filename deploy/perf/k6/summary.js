// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// Turns k6's end-of-test data into the shape `cargo xtask perf-gate` reads:
// { "scenarios": { "<name>": { "p95_ms": <number> } } }.

const SOURCES = {
  read_cached: 'http_req_duration{scenario:read_cached}',
  read_uncached: 'http_req_duration{scenario:read_uncached}',
  write: 'http_req_duration{scenario:write}',
  render_cold: 'render_cold',
  collab_sync: 'collab_sync',
  keystroke: 'keystroke',
  pdf_export: 'pdf_export',
};

export function summarise(data, path) {
  const scenarios = {};
  for (const [name, metric] of Object.entries(SOURCES)) {
    const values = data.metrics[metric] && data.metrics[metric].values;
    // A metric with no samples reports zeros; leave it out so the gate
    // calls it missing instead of comparing a zero.
    const sampled = values && values['p(95)'] !== undefined && values.max > 0;
    if (sampled) {
      scenarios[name] = { p95_ms: Math.round(values['p(95)'] * 10) / 10 };
    }
  }
  return {
    [path]: `${JSON.stringify({ scenarios }, null, 2)}\n`,
    stdout: `${JSON.stringify({ scenarios })}\n`,
  };
}
