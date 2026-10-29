// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { createHash } from "node:crypto";
import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";

import type { Plugin } from "vite";

import type { PrecacheEntry, ShellManifest } from "./sw-core";

/** The worker's source, relative to the web root. */
const WORKER_SOURCE = "src/platform/pwa/sw.ts";
const PLACEHOLDER = "__VELLUM_SHELL__";

function hash(content: string | Uint8Array): string {
  return createHash("sha256").update(content).digest("hex").slice(0, 16);
}

/** Every file under `directory`, as `/relative/path` with forward slashes. */
function listFiles(directory: string, prefix = ""): string[] {
  let names: string[];
  try {
    names = readdirSync(path.join(directory, prefix));
  } catch {
    return [];
  }
  return names.flatMap((name) => {
    const relative = prefix === "" ? name : `${prefix}/${name}`;
    const nested = listFiles(directory, relative);
    return nested.length > 0 ? nested : [relative];
  });
}

/** The emitted worker chunk, still holding the placeholder. */
function isWorkerChunk(value: unknown): value is { type: "chunk"; code: string } {
  if (typeof value !== "object" || value === null) return false;
  const { type, code } = value as { type?: unknown; code?: unknown };
  return type === "chunk" && typeof code === "string" && code.includes(PLACEHOLDER);
}

/** A built output file, as the bundle reports it. */
export interface OutputFile {
  readonly fileName: string;
  readonly content: string | Uint8Array;
}

/**
 * Builds the shell manifest from the build output and the public folder.
 * Source maps and the worker itself are left out.
 */
export function shellManifest(
  outputs: readonly OutputFile[],
  publicFiles: readonly OutputFile[],
): ShellManifest {
  const entries: PrecacheEntry[] = [...outputs, ...publicFiles]
    .filter(({ fileName }) => !fileName.endsWith(".map") && fileName !== "sw.js")
    .map(({ fileName, content }) => ({ url: `/${fileName}`, revision: hash(content) }))
    .toSorted((a, b) => a.url.localeCompare(b.url));
  return { version: hash(JSON.stringify(entries)), entries };
}

/**
 * Builds the service worker next to the app: adds `sw.ts` as a second
 * entry emitted at `/sw.js`, then injects the shell manifest into it.
 * Dev mode serves no worker, so nothing goes stale while developing.
 */
export function vellumServiceWorker(): Plugin {
  let root = "";
  let publicDirectory = "";
  return {
    name: "vellum-service-worker",
    apply: "build",
    enforce: "post",
    config(config) {
      const base = config.root ?? process.cwd();
      return {
        build: {
          rolldownOptions: {
            input: {
              index: path.resolve(base, "index.html"),
              sw: path.resolve(base, WORKER_SOURCE),
            },
            output: {
              entryFileNames: (chunk: { name: string }) =>
                chunk.name === "sw" ? "sw.js" : "assets/[name]-[hash].js",
            },
          },
        },
      };
    },
    configResolved(resolved) {
      root = resolved.root;
      publicDirectory = resolved.publicDir;
    },
    generateBundle(_options, bundle) {
      const outputs: OutputFile[] = Object.values(bundle).map((file) => ({
        fileName: file.fileName,
        content: file.type === "chunk" ? file.code : file.source,
      }));
      const publicFiles = listFiles(publicDirectory).map((fileName) => ({
        fileName,
        content: readFileSync(path.join(publicDirectory, fileName)),
      }));
      const manifest = shellManifest(outputs, publicFiles);
      const worker: unknown = bundle["sw.js"];
      if (!isWorkerChunk(worker)) {
        this.error(`vellum: ${WORKER_SOURCE} under ${root} did not build to sw.js`);
        return;
      }
      worker.code = worker.code.replaceAll(PLACEHOLDER, JSON.stringify(manifest));
    },
  };
}
