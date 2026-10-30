// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { NodeViewContent, NodeViewWrapper, type ReactNodeViewProps } from "@tiptap/react";
import { useState } from "react";

import { CollectionEmbedPreview } from "../../collections/embed-preview";
import { useServerApis } from "../../data/server-apis";
import { useT } from "../../i18n/i18n";
import type { RenderApi } from "../../render/api";
import { RenderedHtml } from "../../render/rendered-html";
import { useBlockRender } from "../../render/use-block-render";
import { CopyDiagnostic } from "../../shell/copy-diagnostic";
import { Icon } from "../../shell/icon";
import { NODES } from "../schema/blocks.gen";

import { KROKI, parseChart, parseCsv, type ChartPreview } from "./fence-preview";

const ICONS: Readonly<Record<string, string>> = Object.fromEntries(
  NODES.find((n) => n.name === "dataFence")?.slash.map((s) => [s.attrs.lang, s.icon]) ?? [],
);

function ChartSvg({ chart }: { readonly chart: ChartPreview }) {
  const t = useT();
  const max = Math.max(...chart.values, 1);
  const width = 320;
  const height = 140;
  const step = width / chart.values.length;
  const summary = chart.labels
    .map((label, index) => `${label} ${String(chart.values[index] ?? "")}`)
    .join(", ");
  const line = chart.type === "line" || chart.type === "area";
  const points = chart.values.map(
    (v, index) =>
      `${String(index * step + step / 2)},${String(height - (v / max) * (height - 10))}`,
  );
  return (
    <figure className="vl-chart">
      {chart.title !== "" && <figcaption>{chart.title}</figcaption>}
      <svg
        viewBox={`0 0 ${String(width)} ${String(height + 20)}`}
        role="img"
        aria-label={t("chart-summary", { title: chart.title, summary })}
      >
        {line ? (
          <polyline points={points.join(" ")} fill="none" className="vl-chart-line" />
        ) : (
          chart.values.map((v, index) => (
            <rect
              key={`${String(index)}-${String(v)}`}
              x={index * step + step * 0.15}
              y={height - (v / max) * (height - 10)}
              width={step * 0.7}
              height={(v / max) * (height - 10)}
              rx="4"
              className="vl-chart-bar"
            />
          ))
        )}
        {chart.labels.map((label, index) => (
          <text
            key={`${String(index)}-${label}`}
            x={index * step + step / 2}
            y={height + 15}
            textAnchor="middle"
          >
            {label}
          </text>
        ))}
      </svg>
    </figure>
  );
}

function CsvTable({ rows }: { readonly rows: string[][] }) {
  const [header = [], ...body] = rows;
  return (
    <table className="vl-table vl-csv">
      <thead>
        <tr>
          {header.map((cell, index) => (
            <th key={`${String(index)}-${cell}`} scope="col">
              {cell}
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {body.map((row, r) => (
          <tr key={`${String(r)}-${row.join(",")}`}>
            {row.map((cell, index) => (
              <td key={`${String(index)}-${cell}`}>{cell}</td>
            ))}
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function Preview({ lang, body }: { readonly lang: string; readonly body: string }) {
  const t = useT();
  if (lang === "chart") {
    const chart = parseChart(body);
    return chart ? (
      <ChartSvg chart={chart} />
    ) : (
      <p className="vl-muted">{t("fence-chart-invalid")}</p>
    );
  }
  if (lang === "table") {
    return <CsvTable rows={parseCsv(body)} />;
  }
  if (lang === "collection") {
    return <CollectionEmbedPreview fence={body} />;
  }
  return (
    <p className="vl-muted">{t(KROKI.has(lang) ? "fence-kroki" : "fence-rendered", { lang })}</p>
  );
}

/** Fences the server cannot draw on its own: collections need the live store. */
const CLIENT_ONLY = new Set(["collection"]);

/**
 * The server's drawing of a fence (ADR 0068, D288), with its warnings and,
 * when the server could not be asked, the light client preview.
 */
function ServerPreview({
  api,
  lang,
  args,
  body,
}: {
  readonly api: RenderApi;
  readonly lang: string;
  readonly args: string;
  readonly body: string;
}) {
  const t = useT();
  const { drawn, failure, busy } = useBlockRender(api, { lang, args, source: body });
  if (failure !== undefined) {
    return (
      <>
        <p className="vl-fence-warning" role="alert">
          <Icon name="warning" />
          {t("fence-render-failed", { message: failure.text })}
          <CopyDiagnostic diagnostic={failure.diagnostic} />
        </p>
        <Preview lang={lang} body={body} />
      </>
    );
  }
  if (drawn === undefined) {
    return (
      <p className="vl-muted" role="status">
        {t("fence-drawing", { lang })}
      </p>
    );
  }
  return (
    <>
      <RenderedHtml api={api} html={drawn.html} className="vl-fence-render" busy={busy} />
      {drawn.warnings.length > 0 && (
        <p className="vl-fence-warning" role="status">
          <Icon name="warning" />
          {drawn.warnings.join(" ")}
          <CopyDiagnostic
            diagnostic={{
              message: drawn.warnings.join(" "),
              route: "POST /api/v1/render/block",
              at: new Date(),
            }}
          />
        </p>
      )}
    </>
  );
}

/**
 * A data fence. With a server, the block is drawn by the renderer and its
 * source opens with the Source button; without one, a
 * light preview sits above the always-open source.
 */
export function DataFenceView({ node, updateAttributes, editor }: ReactNodeViewProps) {
  const t = useT();
  const { render } = useServerApis();
  const lang = String(node.attrs["lang"]);
  const arguments_ = String(node.attrs["args"]);
  const body = node.textContent;
  const server = render !== undefined && !CLIENT_ONLY.has(lang) ? render : undefined;
  const [sourceChoice, setSourceChoice] = useState<boolean>();
  // Source shows by default only where there is nothing drawn to look at.
  const sourceOpen = sourceChoice ?? (server === undefined || body.trim() === "");
  return (
    <NodeViewWrapper className="vl-block vl-fence" data-lang={lang}>
      <div className="vl-fence-head" contentEditable={false}>
        <Icon name={ICONS[lang] ?? "data_object"} />
        <span className="vl-fence-lang">{lang}</span>
        <input
          className="vl-inline-input vl-fence-args"
          aria-label={t("fence-args", { lang })}
          placeholder={t("fence-args-placeholder")}
          value={arguments_}
          readOnly={!editor.isEditable}
          onChange={(event) => {
            updateAttributes({ args: event.target.value });
          }}
        />
        {server !== undefined && (
          <button
            type="button"
            className="vl-text-button vl-fence-toggle"
            aria-pressed={sourceOpen}
            onClick={() => {
              setSourceChoice(!sourceOpen);
            }}
          >
            <Icon name="code" />
            {t("fence-source-toggle")}
          </button>
        )}
      </div>
      <div className="vl-fence-preview" contentEditable={false}>
        {server === undefined ? (
          <Preview lang={lang} body={body} />
        ) : (
          <ServerPreview api={server} lang={lang} args={arguments_} body={body} />
        )}
      </div>
      <pre
        className="vl-fence-source"
        aria-label={t("fence-source", { lang })}
        hidden={!sourceOpen}
      >
        <NodeViewContent<"code"> as="code" />
      </pre>
    </NodeViewWrapper>
  );
}
