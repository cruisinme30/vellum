// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useEffect, useRef } from "react";

import type { RenderApi } from "./api";
import { hydrateCalculators } from "./calculator";
import { adoptCss, mountRendered } from "./mount";

/** The class of every element that holds server-rendered HTML. */
const RENDER_SCOPE = "vl-render";

interface Loading {
  readonly api: RenderApi;
  done: Promise<void>;
  remove?: () => void;
}

let loading: Loading | undefined;

/**
 * Adds the renderer's stylesheet to the document once per server. Its
 * themes key off `<html data-theme>`, so a theme change restyles every
 * figure in place. A failure leaves figures unstyled but readable, and the
 * next rendered block tries again.
 */
function ensureRenderStylesheet(api: RenderApi, document_: Document = document): Promise<void> {
  if (loading?.api === api) return loading.done;
  const previous = loading;
  const entry: Loading = { api, done: Promise.resolve() };
  entry.done = api.stylesheet().then(
    ({ css }) => {
      previous?.remove?.();
      entry.remove = adoptCss(css, document_);
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
  useEffect(() => {
    void ensureRenderStylesheet(api);
  }, [api]);
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
