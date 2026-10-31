// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import {
  NodeViewContent,
  NodeViewWrapper,
  type ReactNodeViewProps,
} from "@tiptap/react";
import { useId, useState } from "react";

import { useT } from "../../i18n/i18n";
import { Icon } from "../../shell/icon";

/**
 * `::: details`: collapsed by default; the open state is per viewer, never
 * saved.
 */
export function DetailsView({
  node,
  updateAttributes,
  editor,
}: ReactNodeViewProps) {
  const [open, setOpen] = useState(false);
  const t = useT();
  const bodyId = useId();
  return (
    <NodeViewWrapper className="vl-block vl-details" data-open={open}>
      <div className="vl-details-head" contentEditable={false}>
        <button
          type="button"
          className="vl-icon-button"
          aria-expanded={open}
          aria-controls={bodyId}
          aria-label={t(open ? "details-collapse" : "details-expand")}
          onClick={() => {
            setOpen(!open);
          }}
        >
          <Icon name={open ? "keyboard_arrow_down" : "chevron_right"} />
        </button>
        <input
          className="vl-inline-input vl-details-summary"
          aria-label={t("details-summary")}
          value={String(node.attrs["summary"])}
          readOnly={!editor.isEditable}
          onChange={(event) => {
            updateAttributes({ summary: event.target.value });
          }}
        />
      </div>
      <NodeViewContent className="vl-details-body" id={bodyId} hidden={!open} />
    </NodeViewWrapper>
  );
}
