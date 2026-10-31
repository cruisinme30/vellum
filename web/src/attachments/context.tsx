// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { createContext, use, useMemo, type ReactNode } from "react";

import { useServerApi } from "../data/server-apis";

import type { AttachmentsApi } from "./api";

/** The page being edited and the API for its files. */
export interface PageFiles {
  readonly api: AttachmentsApi;
  readonly pageId: string;
}

const PageFilesContext = createContext<PageFiles | undefined>(undefined);

/**
 * Lets the editor's blocks upload to the page on view. Without a server
 * (demo mode) there is nothing to upload to, and blocks offer links only.
 */
export function PageFilesProvider({
  pageId,
  children,
}: {
  readonly pageId: string;
  readonly children: ReactNode;
}) {
  const api = useServerApi("attachments");
  const value = useMemo(
    () => (api === undefined ? undefined : { api, pageId }),
    [api, pageId],
  );
  return <PageFilesContext value={value}>{children}</PageFilesContext>;
}

/** The page's files, or `undefined` when uploads are not available. */
export function usePageFiles(): PageFiles | undefined {
  return use(PageFilesContext);
}
