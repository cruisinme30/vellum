// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/** Where the Drive screens live, and where each kind of item opens. */

import type { NodeView } from "./api";

const segment = encodeURIComponent;

export const DRIVE_HOME = "/drive";
export const SHARED_DRIVES = "/drive/shared";
export const SHARED_WITH_ME = "/drive/shared-with-me";
export const STARRED = "/drive/starred";
export const RECENT = "/drive/recent";
export const DRIVE_SEARCH = "/drive/search";

export const spacePath = (space: string) => `/drive/s/${segment(space)}`;
export const folderPath = (space: string, folder: string) =>
  `${spacePath(space)}/f/${segment(folder)}`;
export const trashPath = (space: string) => `${spacePath(space)}/trash`;
export const settingsPath = (space: string) => `${spacePath(space)}/settings`;

/**
 * Where an item opens: a folder in the Drive, a page in the editor, a book
 * in the reader and a collection in its views. A shortcut has no place of
 * its own; it opens its target.
 */
export function openPath(node: NodeView): string | undefined {
  switch (node.kind) {
    case "folder": {
      return folderPath(node.space_id, node.id);
    }
    case "page": {
      return `/p/${segment(node.id)}`;
    }
    case "book": {
      return `/books/${segment(node.id)}`;
    }
    case "collection": {
      return `/data/${segment(node.id)}`;
    }
    case "shortcut": {
      return undefined;
    }
  }
}
