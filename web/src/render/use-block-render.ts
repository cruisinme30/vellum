// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useEffect, useRef, useState } from "react";

import { diagnosticOf, failureText, type Diagnostic } from "../data/failure";

import type { BlockInput, RenderApi, RenderedBlockView } from "./api";

/** How long typing pauses before a block is drawn again. */
const DRAW_DELAY_MS = 400;

/** Why a block could not be drawn: one line, and what to copy for support. */
interface Failure {
  readonly text: string;
  readonly diagnostic: Diagnostic;
}

/**
 * A block's drawing: the latest answer, and whether a newer one is on its way.
 */
export interface BlockDrawing {
  /** The last drawing that arrived; kept while the next one loads. */
  readonly drawn?: RenderedBlockView | undefined;
  /** Why the last request failed. */
  readonly failure?: Failure | undefined;
  readonly busy: boolean;
}

interface Answer {
  /** The block text it answers. */
  readonly key: string;
  readonly drawn?: RenderedBlockView;
  readonly failure?: Failure;
}

/**
 * Draws a fence on the server as its text settles (ADR 0068, D288). The
 * first drawing is immediate; each change after it waits
 * {@link DRAW_DELAY_MS}, and an answer for older text is dropped. With no
 * `api` (demo mode) nothing is drawn.
 */
export function useBlockRender(
  api: RenderApi | undefined,
  block: BlockInput,
): BlockDrawing {
  const [answer, setAnswer] = useState<Answer>();
  const key = JSON.stringify([block.lang, block.args ?? "", block.source]);
  // The first drawing goes at once; later ones wait for typing to pause.
  const first = useRef(true);
  useEffect(() => {
    if (api === undefined) return;
    let current = true;
    const delay = first.current ? 0 : DRAW_DELAY_MS;
    first.current = false;
    const [lang, options, source] = JSON.parse(key) as [string, string, string];
    const timer = setTimeout(() => {
      api.block({ lang, args: options, source }).then(
        (drawn) => {
          if (current) setAnswer({ key, drawn });
        },
        (error: unknown) => {
          if (!current) return;
          const text = failureText(error);
          setAnswer({
            key,
            failure: { text, diagnostic: diagnosticOf(error) },
          });
        },
      );
    }, delay);
    return () => {
      current = false;
      clearTimeout(timer);
    };
  }, [api, key]);
  return {
    drawn: answer?.drawn,
    failure: answer?.failure,
    busy: api !== undefined && answer?.key !== key,
  };
}
