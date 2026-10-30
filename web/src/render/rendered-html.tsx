// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useEffect, useRef } from "react";

import { useFigureSeed } from "../theme/appearance-context";

import type { RenderApi } from "./api";
import { hydrateCalculators } from "./calculator";
import { adoptCss, mountRendered } from "./mount";

/** The class of every element that holds server-rendered HTML. */
const RENDER_SCOPE = "vl-render";

interface Loading {
  readonly api: RenderApi;
  readonly seed: string | undefined;
  done: Promise<void>;
}

let loading: Loading | undefined;
/** Removes the stylesheet in use, whichever request added it. */
let removeAdopted: (() => void) | undefined;

/**
 * Adds the renderer's stylesheet to the document once per server and
 * accent colour. Its themes key off `<html data-theme>`, so a theme change
 * restyles every figure in place; a new accent colour swaps the sheet for
 * one recoloured from it (D302). A failure leaves figures unstyled but
 * readable, and the next rendered block tries again.
 */
export function ensureRenderStylesheet(
  api: RenderApi,
  seed?: string,
  document_: Document = document,
): Promise<void> {
  if (loading?.api === api && loading.seed === seed) return loading.done;
  const entry: Loading = { api, seed, done: Promise.resolve() };
  entry.done = api.stylesheet(seed).then(
    ({ css }) => {
      // A newer accent may have been asked for while this one loaded.
      if (loading !== entry) return;
      removeAdopted?.();
      removeAdopted = adoptCss(css, document_);
    },
    () => {
      if (loading === entry) loading = undefined;
    },
  );
  loading = entry;
  return entry.done;
}

/**
 * HTML from the server's renderer. The server sanitises it (no script, no
 * event handlers, only safe URLs; ADR 0007), so it is inserted as is,
 * with its styles moved where the app's Content Security Policy allows
 * them; calculators get their sliders wired here.
 */
export function RenderedHtml({
  html,
  api,
  className,
  label,
  busy = false,
}: {
  readonly html: string;
  readonly api: RenderApi;
  readonly className?: string;
  /** A newer drawing is on its way. */
  readonly busy?: boolean;
  /** An accessible name for the region, when it stands alone. */
  readonly label?: string;
}) {
  const reference = useRef<HTMLDivElement>(null);
  const seed = useFigureSeed();
  useEffect(() => {
    void ensureRenderStylesheet(api, seed);
  }, [api, seed]);
  useEffect(() => {
    const root = reference.current;
    if (root === null) return;
    const unmount = mountRendered(root, html);
    const unwire = hydrateCalculators(root);
    return () => {
      unwire();
      unmount();
    };
  }, [html]);
  return (
    <div
      ref={reference}
      className={className === undefined ? RENDER_SCOPE : `${RENDER_SCOPE} ${className}`}
      aria-busy={busy || undefined}
      {...(label === undefined ? {} : { role: "region", "aria-label": label })}
    />
  );
}
