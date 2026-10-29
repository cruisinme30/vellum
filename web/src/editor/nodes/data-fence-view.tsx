// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { NodeViewContent, NodeViewWrapper, type ReactNodeViewProps } from "@tiptap/react";

import { CollectionEmbedPreview } from "../../collections/embed-preview";
import { useT } from "../../i18n/i18n";
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

/** A data fence: the source stays editable; a preview sits above it. */
export function DataFenceView({ node, updateAttributes, editor }: ReactNodeViewProps) {
  const t = useT();
  const lang = String(node.attrs["lang"]);
  return (
    <NodeViewWrapper className="vl-block vl-fence" data-lang={lang}>
      <div className="vl-fence-head" contentEditable={false}>
        <Icon name={ICONS[lang] ?? "data_object"} />
        <span className="vl-fence-lang">{lang}</span>
        <input
          className="vl-inline-input vl-fence-args"
          aria-label={t("fence-args", { lang })}
          placeholder={t("fence-args-placeholder")}
          value={String(node.attrs["args"])}
          readOnly={!editor.isEditable}
          onChange={(event) => {
            updateAttributes({ args: event.target.value });
          }}
        />
      </div>
      <div className="vl-fence-preview" contentEditable={false}>
        <Preview lang={lang} body={node.textContent} />
      </div>
      <pre className="vl-fence-source" aria-label={t("fence-source", { lang })}>
        <NodeViewContent<"code"> as="code" />
      </pre>
    </NodeViewWrapper>
  );
}
