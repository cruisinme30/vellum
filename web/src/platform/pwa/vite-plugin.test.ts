// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import { describe, expect, it, vi } from "vitest";

import { shellManifest, vellumServiceWorker } from "./vite-plugin";

type Bundle = Record<
  string,
  | { type: "chunk"; fileName: string; code: string }
  | { type: "asset"; fileName: string; source: string }
>;

/** Calls a plugin hook the way Vite would. */
function call(hook: unknown, self: unknown, ...input: unknown[]): unknown {
  return (hook as (...parameters: unknown[]) => unknown).apply(self, input);
}

interface BuildConfig {
  build: {
    rolldownOptions: {
      input: Record<string, string>;
      output: { entryFileNames: (chunk: { name: string }) => string };
    };
  };
}

function hooks() {
  return { plugin: vellumServiceWorker(), call };
}

describe("shellManifest", () => {
  it("lists outputs and public files with content revisions, sorted", () => {
    const manifest = shellManifest(
      [
        { fileName: "index.html", content: "<html>" },
        { fileName: "assets/app.js.map", content: "{}" },
        { fileName: "sw.js", content: "x" },
        { fileName: "assets/app.js", content: "code" },
      ],
      [{ fileName: "icons/a.png", content: new Uint8Array([1]) }],
    );
    expect(manifest.entries.map((entry) => entry.url)).toEqual([
      "/assets/app.js",
      "/icons/a.png",
      "/index.html",
    ]);
    expect(
      manifest.entries.every((entry) =>
        /^[\da-f]{16}$/.test(entry.revision ?? ""),
      ),
    ).toBe(true);
    const changed = shellManifest(
      [{ fileName: "index.html", content: "<html lang>" }],
      [],
    );
    expect(changed.version).not.toBe(manifest.version);
  });
});

describe("vellumServiceWorker", () => {
  it("adds the worker as a second entry named sw.js", () => {
    const { plugin, call } = hooks();
    const config = call(plugin.config, undefined, {
      root: "/web",
    }) as BuildConfig;
    const { input, output } = config.build.rolldownOptions;
    expect(input).toEqual({
      index: "/web/index.html",
      sw: "/web/src/platform/pwa/sw.ts",
    });
    expect(output.entryFileNames({ name: "sw" })).toBe("sw.js");
    expect(output.entryFileNames({ name: "index" })).toBe(
      "assets/[name]-[hash].js",
    );
    const fallback = call(plugin.config, undefined, {}) as BuildConfig;
    expect(fallback.build.rolldownOptions.input["index"]).toBe(
      path.resolve(process.cwd(), "index.html"),
    );
  });

  it("injects the manifest, including public files, into the worker", () => {
    const publicDirectory = mkdtempSync(path.join(tmpdir(), "vellum-public-"));
    mkdirSync(path.join(publicDirectory, "icons"));
    writeFileSync(path.join(publicDirectory, "icons", "icon.png"), "png");
    writeFileSync(path.join(publicDirectory, "manifest.webmanifest"), "{}");
    const { plugin, call } = hooks();
    call(plugin.configResolved, undefined, {
      root: "/web",
      publicDir: publicDirectory,
    });
    const bundle: Bundle = {
      "sw.js": {
        type: "chunk",
        fileName: "sw.js",
        code: "const m = __VELLUM_SHELL__;",
      },
      "index.html": { type: "asset", fileName: "index.html", source: "<html>" },
    };
    call(plugin.generateBundle, { error: vi.fn() }, {}, bundle);
    const worker = bundle["sw.js"];
    const code = worker?.type === "chunk" ? worker.code : "";
    const injected = JSON.parse(
      code.replace("const m = ", "").replace(/;$/, ""),
    ) as {
      entries: { url: string }[];
    };
    expect(injected.entries.map((entry) => entry.url)).toEqual([
      "/icons/icon.png",
      "/index.html",
      "/manifest.webmanifest",
    ]);
  });

  it("fails the build when the worker did not build", () => {
    const { plugin, call } = hooks();
    call(plugin.configResolved, undefined, {
      root: "/web",
      publicDir: "/does/not/exist",
    });
    const error = vi.fn(() => {
      throw new Error("build failed");
    });
    expect(() => call(plugin.generateBundle, { error }, {}, {})).toThrow(
      "build failed",
    );
    expect(error).toHaveBeenCalledWith(
      expect.stringContaining("did not build to sw.js"),
    );
  });
});
