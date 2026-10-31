// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState } from "react";

import { useServerApis } from "../data/server-apis";
import { useT } from "../i18n/i18n";
import { Icon } from "../shell/icon";

import type { ExportTarget } from "./api";
import { ExportPanel } from "./export-panel";

/**
 * The Export button and the panel it opens; nothing in demo mode, which
 * has no server to typeset on.
 */
export function ExportButton({
  target,
  label,
}: {
  readonly target: ExportTarget;
  /** The button's text; an icon button when absent. */
  readonly label?: string;
}) {
  const t = useT();
  const { transfer } = useServerApis();
  const [open, setOpen] = useState(false);
  if (transfer === undefined) return;
  const toggle = () => {
    setOpen(!open);
  };
  return (
    <>
      {label === undefined ? (
        <button
          type="button"
          className="vl-icon-button"
          aria-label={t("export-open")}
          title={t("export-open")}
          aria-expanded={open}
          onClick={toggle}
        >
          <Icon name="download" />
        </button>
      ) : (
        <button
          type="button"
          className="vl-text-button"
          aria-expanded={open}
          onClick={toggle}
        >
          <Icon name="download" size={20} />
          {label}
        </button>
      )}
      {open && (
        <ExportPanel
          target={target}
          api={transfer}
          onClose={() => {
            setOpen(false);
          }}
        />
      )}
    </>
  );
}
