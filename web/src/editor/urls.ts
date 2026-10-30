// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * URL policy for links, images and embeds (R-SE-4): only http(s) and
 * workspace-relative paths are ever placed in the DOM.
 */

/** The URL if it is safe to link to, otherwise `undefined`. */
export function safeUrl(raw: string): string | undefined {
  const value = raw.trim();
  if (value === "") {
    return undefined;
  }
  if (/^(?:\/(?!\/)|assets\/)[^\s]*$/.test(value)) {
    return value;
  }
  try {
    const url = new URL(value);
    return url.protocol === "https:" || url.protocol === "http:"
      ? url.href
      : undefined;
  } catch {
    return undefined;
  }
}

/**
 * `vellum://attachment/{page}/{name}`: a file attached to a page (ADR 0039
 * D211).
 */
const ATTACHMENT_REF =
  /^vellum:\/\/attachment\/([0-9A-HJKMNP-TV-Za-hjkmnp-tv-z]{26})\/([^\s/?#]+)$/;

/**
 * Where to load an attachment `ref` from: the server's download path for
 * it, or `undefined` for anything that is not an attachment ref.
 */
export function attachmentUrl(raw: string): string | undefined {
  const match = ATTACHMENT_REF.exec(raw.trim());
  if (match === null) return undefined;
  const [, page = "", name = ""] = match;
  return `/api/v1/attachments/${page}/${name}`;
}

/**
 * An image's source to put in the DOM: an attachment's download path, or a safe
 * URL.
 */
export function mediaUrl(raw: string): string | undefined {
  return attachmentUrl(raw) ?? safeUrl(raw);
}

/** An embed the sandboxed frame can show. */
export interface EmbedTarget {
  readonly provider: string;
  readonly src: string;
}

type Matcher = (
  url: URL,
  host: string,
  parts: readonly string[],
) => EmbedTarget | undefined;

const youtubeId = (url: URL, host: string, parts: readonly string[]) => {
  if (host === "youtu.be") {
    return parts[0];
  }
  return host === "youtube.com"
    ? (url.searchParams.get("v") ?? undefined)
    : undefined;
};

const MATCHERS: readonly Matcher[] = [
  (url, host, parts) => {
    const id = youtubeId(url, host, parts);
    return id !== undefined && /^[\w-]{6,}$/.test(id)
      ? {
          provider: "YouTube",
          src: `https://www.youtube-nocookie.com/embed/${id}`,
        }
      : undefined;
  },
  (_url, host, [kind, id]) =>
    host === "loom.com" && kind === "share" && id !== undefined
      ? { provider: "Loom", src: `https://www.loom.com/embed/${id}` }
      : undefined,
  (url, host, [kind]) =>
    host === "figma.com" &&
    ["file", "design", "proto", "board"].includes(kind ?? "")
      ? {
          provider: "Figma",
          src: `https://www.figma.com/embed?embed_host=vellum&url=${encodeURIComponent(url.href)}`,
        }
      : undefined,
  (_url, host, [user, kind, id]) =>
    host === "codepen.io" &&
    kind === "pen" &&
    user !== undefined &&
    id !== undefined
      ? {
          provider: "CodePen",
          src: `https://codepen.io/${user}/embed/${id}?default-tab=result`,
        }
      : undefined,
  (_url, host, [user, id]) =>
    host === "gist.github.com" && user !== undefined && id !== undefined
      ? {
          provider: "GitHub Gist",
          src: `https://gist.github.com/${user}/${id}.pibb`,
        }
      : undefined,
  (url, host, [maps, embed]) =>
    host === "google.com" && maps === "maps" && embed === "embed"
      ? { provider: "Google Maps", src: url.href }
      : undefined,
];

/**
 * Turns a share URL from a supported provider (YouTube, Loom, Figma,
 * CodePen, GitHub gists, Google Maps) into its embed URL.
 */
export function embedTarget(raw: string): EmbedTarget | undefined {
  let url: URL;
  try {
    url = new URL(raw.trim());
  } catch {
    return undefined;
  }
  if (url.protocol !== "https:") {
    return undefined;
  }
  const host = url.hostname.replace(/^www\./, "");
  const parts = url.pathname.split("/").filter(Boolean);
  for (const match of MATCHERS) {
    const target = match(url, host, parts);
    if (target) {
      return target;
    }
  }
  return undefined;
}
