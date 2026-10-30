// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * The server's renderer, for the web app (issue #522, ADR 0068): whole
 * pages for the reading view, single blocks for the editor, and the theme
 * stylesheet that styles both. Types come from the generated client.
 *
 * Page and stylesheet reads carry an `ETag`; the browser's own cache sends
 * `If-None-Match` and turns the server's `304` back into the cached body,
 * so this client needs no conditional logic of its own.
 */

import type {
  BlockInput,
  RenderedBlockView,
  RenderedPageView,
  RenderStylesheet,
} from "../api/api.gen";
import type { Http } from "../api/http";

/** The render operations the web app uses. */
export interface RenderApi {
  /** A page as sanitised HTML, for the reader. */
  page(id: string): Promise<RenderedPageView>;
  /** One fenced block, drawn the way the page will show it. */
  block(input: BlockInput): Promise<RenderedBlockView>;
  /** The stylesheet for rendered HTML, scoped to the app's render containers. */
  stylesheet(): Promise<RenderStylesheet>;
}

/** Most blocks kept per tab; an edit that returns to an old text is instant. */
const BLOCK_CACHE = 200;

/** The render API over the app's HTTP client, with a small cache of drawn blocks. */
export function createHttpRenderApi(http: Http): RenderApi {
  const blocks = new Map<string, Promise<RenderedBlockView>>();
  return {
    page: (id) => http.get<RenderedPageView>(`/api/v1/pages/${encodeURIComponent(id)}/render`),
    block(input) {
      const key = JSON.stringify([input.lang, input.args ?? "", input.source]);
      const known = blocks.get(key);
      if (known !== undefined) {
        // Most recently used goes last.
        blocks.delete(key);
        blocks.set(key, known);
        return known;
      }
      const drawn = http.send<RenderedBlockView>("POST", "/api/v1/render/block", input);
      blocks.set(key, drawn);
      // A failure is not kept: the next edit, or the diagram server coming back, retries.
      void drawn.catch(() => {
        if (blocks.get(key) === drawn) blocks.delete(key);
      });
      while (blocks.size > BLOCK_CACHE) {
        const oldest = blocks.keys().next().value;
        if (oldest === undefined) break;
        blocks.delete(oldest);
      }
      return drawn;
    },
    stylesheet: () => http.get<RenderStylesheet>("/api/v1/render/stylesheet"),
  };
}

export { type BlockInput, type RenderedBlockView } from "../api/api.gen";
