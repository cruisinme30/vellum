// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Files attached to a page (`/api/v1/attachments`, ADR 0039 D211-D213).
 * Every upload passes the server's checks: size, type sniffing, image
 * re-encoding and the virus scanner. A page links to a file by its `ref`
 * (`vellum://attachment/{page}/{name}`) and shows it through its `url`.
 */

import { ApiError, type Http } from "../api/http";

/** One attached file. */
export interface Attachment {
  readonly page_id: string;
  readonly name: string;
  readonly size: number;
  /** The sniffed media type; only on the answer to an upload. */
  readonly media_type?: string | undefined;
  /** What the page's Markdown links to. */
  readonly ref: string;
  /** Where to download or delete it. */
  readonly url: string;
}

/** The attachment operations the web app uses. */
export interface AttachmentsApi {
  /** A page's files. */
  readonly list: (pageId: string) => Promise<Attachment[]>;
  /** Uploads a file to a page, under the file's own name. */
  readonly upload: (pageId: string, file: File) => Promise<Attachment>;
  /** Deletes a file by its `url`. */
  readonly remove: (url: string) => Promise<void>;
}

const ATTACHMENTS = "/api/v1/attachments";

/** Only the server's own attachment paths are ever deleted through `remove`. */
const isAttachmentUrl = (url: string) =>
  url.startsWith(`${ATTACHMENTS}/`) && !url.includes("..");

/** Attachments over a Vellum server. */
export function createHttpAttachmentsApi(http: Http): AttachmentsApi {
  return {
    async list(pageId) {
      const found = await http.get<{ items: Attachment[] }>(
        `${ATTACHMENTS}/${encodeURIComponent(pageId)}`,
      );
      return found.items.toSorted((a, b) => a.name.localeCompare(b.name));
    },
    upload: (pageId, file) =>
      http.upload<Attachment>(
        `${ATTACHMENTS}/${encodeURIComponent(pageId)}`,
        { name: file.name },
        file,
        file.type === "" ? "application/octet-stream" : file.type,
      ),
    async remove(url) {
      if (!isAttachmentUrl(url)) throw new Error(`not an attachment: ${url}`);
      await http.send("DELETE", url);
    },
  };
}

/**
 * Why an upload failed, for the person: the server's statuses (D212) in plain
 * words.
 */
export function uploadProblem(error: unknown): {
  readonly key: string;
  readonly detail: string;
} {
  const detail = error instanceof Error ? error.message : String(error);
  if (!(error instanceof ApiError)) return { key: "upload-failed", detail };
  const keys: Record<number, string> = {
    413: "upload-too-large",
    415: "upload-wrong-type",
    422: "upload-refused",
    503: "upload-scanner-down",
  };
  return { key: keys[error.status] ?? "upload-failed", detail };
}
