// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { JSONContent } from "@tiptap/core";
import type { Schema } from "@tiptap/pm/model";
import { prosemirrorJSONToYXmlFragment } from "@tiptap/y-tiptap";
import * as Y from "yjs";

/** The XML fragment the editor binds. */
export const FIELD = "default";

/** The client id every seed is written with. */
const SEED_CLIENT = 1;

/**
 * Encodes initial content as a Yjs update written by a fixed client id.
 * Every browser that applies it creates the same items, so seeding twice,
 * or in two tabs at once, never duplicates the content; and content a user
 * deleted stays deleted.
 */
export function seedUpdate(schema: Schema, content: JSONContent): Uint8Array {
  const document_ = new Y.Doc();
  document_.clientID = SEED_CLIENT;
  prosemirrorJSONToYXmlFragment(
    schema,
    content,
    document_.getXmlFragment(FIELD),
  );
  const update = Y.encodeStateAsUpdate(document_);
  document_.destroy();
  return update;
}
