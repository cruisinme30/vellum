// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useEffect, useState } from "react";

import { useT } from "../i18n/i18n";

import type { LiveState } from "./collab/provider";

/** How long joining may take before the page says so. */
const JOIN_NOTICE_MS = 1000;

/** One line under the toolbar when the page is not plainly editable and synced. */
export function LiveNotice({
  state,
  joining,
}: {
  readonly state: LiveState;
  readonly joining: boolean;
}) {
  const t = useT();
  // Joining (connecting, then the server saying whether this person may
  // write) is usually over within a frame or two; saying so straight away
  // would flash a line that pushes the page down and back.
  const [slow, setSlow] = useState(false);
  useEffect(() => {
    if (!joining) return;
    const timer = setTimeout(() => {
      setSlow(true);
    }, JOIN_NOTICE_MS);
    return () => {
      clearTimeout(timer);
    };
  }, [joining]);
  let key: string | undefined;
  switch (state.status) {
    case "revoked": {
      key = "editor-revoked";

      break;
    }
    case "offline": {
      key = "editor-offline";

      break;
    }
    case "connecting": {
      if (!joining || slow) key = "editor-connecting";

      break;
    }
    default: {
      if (state.canWrite === false) key = "editor-read-only";
      else if (joining && slow) key = "editor-connecting";
    }
  }
  // The live region stays mounted so screen readers announce each change.
  return (
    <div role="status">
      {key === undefined ? undefined : <p className="vl-editor-notice">{t(key)}</p>}
    </div>
  );
}
