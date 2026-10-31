// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useSearchParams } from "react-router";

import { failureText } from "../data/failure";
import { useServerApis } from "../data/server-apis";
import { useLoaded } from "../data/use-loaded";
import { useT } from "../i18n/i18n";
import type { RenderApi } from "../render/api";
import { RenderedHtml } from "../render/rendered-html";
import { CopyDiagnostic } from "../shell/copy-diagnostic";
import { Icon } from "../shell/icon";

/**
 * Whether the page shows its reading view (`?view=read`), and a switch
 * between it and the editor. Only a server can render, so in demo mode
 * there is no switch and the editor shows (ADR 0068, D289).
 */
export function usePageViewMode() {
  const t = useT();
  const { render } = useServerApis();
  const [search, setSearch] = useSearchParams();
  const reading = render !== undefined && search.get("view") === "read";
  const choose = (read: boolean) => {
    setSearch(
      (current) => {
        const next = new URLSearchParams(current);
        if (read) next.set("view", "read");
        else next.delete("view");
        return next;
      },
      { replace: true },
    );
  };
  const toggle = render !== undefined && (
    <div
      className="vl-view-switch"
      role="group"
      aria-label={t("page-view-switch")}
    >
      <button
        type="button"
        className="vl-text-button"
        aria-pressed={reading}
        onClick={() => {
          choose(true);
        }}
      >
        <Icon name="description" />
        {t("page-view-read")}
      </button>
      <button
        type="button"
        className="vl-text-button"
        aria-pressed={!reading}
        onClick={() => {
          choose(false);
        }}
      >
        <Icon name="edit" />
        {t("page-view-edit")}
      </button>
    </div>
  );
  return { reading, render, toggle };
}

/**
 * The page as the server renders it for this reader: every diagram, chart
 * and plan drawn, in the app's theme. Loaded fresh each time it opens; the
 * browser revalidates with the page's `ETag`, so an unchanged page costs a
 * `304` (ADR 0061, D270).
 */
export function ReadingView({
  pageId,
  api,
}: {
  readonly pageId: string;
  readonly api: RenderApi;
}) {
  const t = useT();
  const loaded = useLoaded(() => api.page(pageId), [api, pageId], failureText);
  if (loaded.state === "loading") {
    return (
      <p className="vl-muted" role="status">
        {t("page-rendering")}
      </p>
    );
  }
  if (loaded.state === "failed") {
    return (
      <p className="vl-field-error" role="alert">
        {loaded.message}
        <CopyDiagnostic message={loaded.message} />
      </p>
    );
  }
  return (
    <RenderedHtml
      api={api}
      html={loaded.value.html}
      className="vl-reading"
      label={t("page-reading")}
    />
  );
}
