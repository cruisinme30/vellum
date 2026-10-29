// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { NodeViewWrapper, type ReactNodeViewProps } from "@tiptap/react";

import { useT } from "../../i18n/i18n";
import { Icon } from "../../shell/icon";
import { embedTarget, safeUrl } from "../urls";

interface FieldProperties {
  readonly label: string;
  readonly value: string;
  readonly onChange: (value: string) => void;
  readonly type?: "url" | "text";
  readonly readOnly: boolean;
}

function Field({ label, value, onChange, type = "text", readOnly }: FieldProperties) {
  return (
    <label className="vl-field">
      <span>{label}</span>
      <input
        type={type}
        value={value}
        readOnly={readOnly}
        onChange={(event) => {
          onChange(event.target.value);
        }}
      />
    </label>
  );
}

/** Shows the attribute form while the block is selected or still empty. */
const editing = (properties: ReactNodeViewProps, key: string) =>
  properties.editor.isEditable &&
  (properties.selected || String(properties.node.attrs[key]) === "");

/** `::: link <url>`: a rich link preview. */
export function LinkCardView(properties: ReactNodeViewProps) {
  const t = useT();
  const { node, updateAttributes, editor } = properties;
  const url = String(node.attrs["url"]);
  const title = String(node.attrs["title"]);
  const href = safeUrl(url);
  return (
    <NodeViewWrapper className="vl-block vl-linkCard" contentEditable={false}>
      {href ? (
        <a className="vl-card-link" href={href} target="_blank" rel="noopener noreferrer">
          <Icon name="link" />
          <span>
            <strong>{title === "" ? href : title}</strong>
            <small>{new URL(href, globalThis.location.href).hostname}</small>
          </span>
        </a>
      ) : (
        <p className="vl-muted">{t("link-empty")}</p>
      )}
      {editing(properties, "url") && (
        <div className="vl-form">
          <Field
            label={t("field-url")}
            type="url"
            value={url}
            readOnly={!editor.isEditable}
            onChange={(v) => {
              updateAttributes({ url: v });
            }}
          />
          <Field
            label={t("field-title")}
            value={title}
            readOnly={!editor.isEditable}
            onChange={(v) => {
              updateAttributes({ title: v });
            }}
          />
        </div>
      )}
    </NodeViewWrapper>
  );
}

/** `::: embed <url>`: a sandboxed frame for supported providers, a link otherwise. */
export function EmbedView(properties: ReactNodeViewProps) {
  const t = useT();
  const { node, updateAttributes, editor } = properties;
  const url = String(node.attrs["url"]);
  const target = embedTarget(url);
  const href = safeUrl(url);
  return (
    <NodeViewWrapper className="vl-block vl-embed" contentEditable={false}>
      {target ? (
        <iframe
          src={target.src}
          title={t("embed-title", { provider: target.provider })}
          sandbox="allow-scripts allow-same-origin allow-popups allow-presentation"
          referrerPolicy="strict-origin-when-cross-origin"
          loading="lazy"
          allowFullScreen
        />
      ) : (
        <p className="vl-muted">
          {href ? (
            <a href={href} target="_blank" rel="noopener noreferrer">
              {t("embed-unsupported", { url: href })}
            </a>
          ) : (
            t("embed-empty")
          )}
        </p>
      )}
      {editing(properties, "url") && (
        <div className="vl-form">
          <Field
            label={t("field-url")}
            type="url"
            value={url}
            readOnly={!editor.isEditable}
            onChange={(v) => {
              updateAttributes({ url: v });
            }}
          />
        </div>
      )}
    </NodeViewWrapper>
  );
}

/** A block image with alt text, width and alignment. */
export function ImageView(properties: ReactNodeViewProps) {
  const t = useT();
  const { node, updateAttributes, editor } = properties;
  const source = String(node.attrs["src"]);
  const alt = String(node.attrs["alt"]);
  const width = String(node.attrs["width"]);
  const safe = safeUrl(source);
  return (
    <NodeViewWrapper
      className="vl-block vl-image"
      data-align={String(node.attrs["align"])}
      contentEditable={false}
    >
      {safe ? (
        <figure>
          <img
            src={safe}
            alt={alt}
            style={width === "" ? undefined : { width }}
            draggable={false}
          />
          {String(node.attrs["title"]) !== "" && (
            <figcaption>{String(node.attrs["title"])}</figcaption>
          )}
        </figure>
      ) : (
        <p className="vl-muted">{t("image-empty")}</p>
      )}
      {editing(properties, "src") && (
        <div className="vl-form">
          <Field
            label={t("field-image-url")}
            type="url"
            value={source}
            readOnly={!editor.isEditable}
            onChange={(v) => {
              updateAttributes({ src: v });
            }}
          />
          <Field
            label={t("field-alt")}
            value={alt}
            readOnly={!editor.isEditable}
            onChange={(v) => {
              updateAttributes({ alt: v });
            }}
          />
        </div>
      )}
    </NodeViewWrapper>
  );
}
