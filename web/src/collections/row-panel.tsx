// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useEffect, useId, useRef, useState } from "react";

import { useT } from "../i18n/i18n";
import { Icon } from "../shell/icon";

import { useLoad } from "./context";
import { useWriter } from "./editing";
import { RowField } from "./row-fields";
import type { CollectionInfo } from "./types";

interface RowPanelProperties {
  readonly info: CollectionInfo;
  readonly rowId: string;
  /** Puts the caret in the title once the row loads: for a new row. */
  readonly focusTitle: boolean;
  readonly onClose: () => void;
}

function BodyField({
  body,
  onSave,
}: {
  readonly body: string;
  readonly onSave: (body: string) => void;
}) {
  const t = useT();
  const [draft, setDraft] = useState(body);
  return (
    <label className="vl-field vl-row-field">
      {t("collection-body")}
      <textarea
        className="vl-row-body"
        rows={6}
        value={draft}
        onChange={(event) => {
          setDraft(event.target.value);
        }}
        onBlur={() => {
          if (draft !== body) {
            onSave(draft);
          }
        }}
      />
    </label>
  );
}

/**
 * A row's every property in a side panel, all editable but the computed
 * ones. It is a non-modal dialog: Escape or the close button closes it and
 * focus goes back to what opened it.
 */
export function RowPanel({ info, rowId, focusTitle, onClose }: RowPanelProperties) {
  const t = useT();
  const headingId = useId();
  const panel = useRef<HTMLDivElement>(null);
  const write = useWriter(info.id);
  const [bodyError, setBodyError] = useState<string>();
  const detail = useLoad(`row:${info.id}:${rowId}`, (store) =>
    store.getRow({ collection_id: info.id, row_id: rowId }),
  );
  const loaded = detail.data !== undefined;
  const close = useRef(onClose);
  useEffect(() => {
    close.current = onClose;
  });

  useEffect(() => {
    const opener = document.activeElement as HTMLElement | null;
    panel.current?.querySelector<HTMLElement>(".vl-row-close")?.focus();
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        close.current();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("keydown", onKey);
      if (opener?.isConnected) {
        opener.focus();
      }
    };
  }, []);
  useEffect(() => {
    if (loaded && focusTitle) {
      panel.current?.querySelector<HTMLElement>("[data-title]")?.focus();
    }
  }, [loaded, focusTitle]);

  const title = detail.data?.title ?? "";
  return (
    <div
      ref={panel}
      className="vl-row-panel"
      role="dialog"
      aria-modal="false"
      aria-labelledby={headingId}
    >
      <div className="vl-row-panel-head">
        <h2 id={headingId}>{title === "" ? t("page-untitled") : title}</h2>
        <button
          type="button"
          className="vl-icon-button vl-row-close"
          aria-label={t("collection-close-row")}
          onClick={onClose}
        >
          <Icon name="close" />
        </button>
      </div>
      {detail.error !== undefined && (
        <p className="vl-field-error" role="alert">
          {detail.error}
        </p>
      )}
      {detail.data && (
        <div className="vl-row-fields">
          {info.properties.map((property) => (
            <RowField
              key={property.key}
              property={property}
              cell={detail.data?.row.cells[property.key]}
              onSave={(value) => write(rowId, { [property.key]: value })}
            />
          ))}
          <BodyField
            key={detail.data.body}
            body={detail.data.body}
            onSave={(body) => {
              void write(rowId, {}, body).then(setBodyError);
            }}
          />
          {bodyError !== undefined && (
            <p className="vl-field-error" role="alert">
              {bodyError}
            </p>
          )}
        </div>
      )}
    </div>
  );
}
