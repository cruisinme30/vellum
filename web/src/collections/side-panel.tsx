// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useEffect, useId, useRef, type ReactNode } from "react";

import { useT } from "../i18n/i18n";
import { Icon } from "../shell/icon";

/**
 * A side panel for collection settings, shaped like the row panel: a
 * non-modal dialog that Escape or the close button closes, giving focus
 * back to what opened it.
 */
export function SidePanel({
  title,
  onClose,
  children,
}: {
  readonly title: string;
  readonly onClose: () => void;
  readonly children: ReactNode;
}) {
  const t = useT();
  const headingId = useId();
  const panel = useRef<HTMLDivElement>(null);
  const close = useRef(onClose);
  useEffect(() => {
    close.current = onClose;
  });
  useEffect(() => {
    const opener = document.activeElement as HTMLElement | null;
    panel.current?.querySelector<HTMLElement>("input, select, textarea")?.focus();
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") close.current();
    };
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("keydown", onKey);
      if (opener?.isConnected) opener.focus();
    };
  }, []);
  return (
    <div
      ref={panel}
      className="vl-row-panel vl-side-panel"
      role="dialog"
      aria-modal="false"
      aria-labelledby={headingId}
    >
      <div className="vl-row-panel-head">
        <h2 id={headingId}>{title}</h2>
        <button
          type="button"
          className="vl-icon-button"
          aria-label={t("collection-close-panel")}
          onClick={onClose}
        >
          <Icon name="close" />
        </button>
      </div>
      <div className="vl-row-fields">{children}</div>
    </div>
  );
}

/** A form's error line, announced when it appears. */
export function FormError({ error }: { readonly error: string | undefined }) {
  return (
    error !== undefined && (
      <p className="vl-field-error" role="alert">
        {error}
      </p>
    )
  );
}

/** Saves a text file the browser offers to the user. */
export function download(fileName: string, text: string, type = "text/csv"): void {
  const url = URL.createObjectURL(new Blob([text], { type: `${type};charset=utf-8` }));
  const link = document.createElement("a");
  link.href = url;
  link.download = fileName;
  link.rel = "noopener";
  document.body.append(link);
  link.click();
  link.remove();
  setTimeout(() => {
    URL.revokeObjectURL(url);
  }, 0);
}
