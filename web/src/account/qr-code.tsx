// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * A QR code drawn from the server's modules: one string per row, `1` for
 * dark. Drawn as one SVG path with a quiet zone, dark on white in every
 * theme, so phone cameras read it.
 */
export function QrCode({
  rows,
  label,
}: {
  readonly rows: readonly string[];
  readonly label: string;
}) {
  const size = rows.length;
  if (size === 0) return;
  const quiet = 4;
  let path = "";
  for (const [y, row] of rows.entries()) {
    for (let x = row.indexOf("1"); x !== -1; x = row.indexOf("1", x + 1)) {
      path += `M${String(x + quiet)} ${String(y + quiet)}h1v1h-1z`;
    }
  }
  const box = String(size + quiet * 2);
  return (
    <svg
      className="vl-account-qr"
      viewBox={`0 0 ${box} ${box}`}
      role="img"
      aria-label={label}
      shapeRendering="crispEdges"
    >
      <rect width={box} height={box} fill="#fff" />
      <path d={path} fill="#000" />
    </svg>
  );
}
