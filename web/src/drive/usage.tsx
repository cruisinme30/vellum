// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useId } from "react";

import { failureText } from "../data/failure";
import { useLoaded } from "../data/use-loaded";
import { useT } from "../i18n/i18n";

import type { DriveApi, SpaceView, UsageView } from "./api";
import { formatBytes, usedShare } from "./format";

/** At or above this share of its quota a space shows a warning. */
const NEAR_FULL = 0.9;

/** How much of its quota a space uses: a meter and a line of text. */
function UsageLine({ usage }: { readonly usage: UsageView }) {
  const t = useT();
  const id = useId();
  const share = usedShare(usage.used_bytes, usage.quota_bytes);
  const used = formatBytes(usage.used_bytes);
  if (share === undefined || usage.quota_bytes === undefined || usage.quota_bytes === null) {
    return <p className="vl-drive-usage vl-muted">{t("drive-usage-unlimited", { used })}</p>;
  }
  const text = t("drive-usage", { used, quota: formatBytes(usage.quota_bytes) });
  return (
    <div className="vl-drive-usage">
      <meter
        aria-labelledby={id}
        min={0}
        max={1}
        low={0.75}
        high={NEAR_FULL}
        optimum={0}
        value={share}
      />
      <span id={id}>{text}</span>
      {share >= NEAR_FULL && (
        <span className="vl-field-error" role="status">
          {t(share >= 1 ? "drive-usage-full" : "drive-usage-near-full")}
        </span>
      )}
    </div>
  );
}

/** A space's usage against its quota (R-DR-10); nothing while it loads. */
export function UsageMeter({ api, space }: { readonly api: DriveApi; readonly space: SpaceView }) {
  const loaded = useLoaded(() => api.usage(space.id), [api, space.id], failureText);
  if (loaded.state === "loading") return;
  if (loaded.state === "failed") {
    return <p className="vl-muted">{loaded.message}</p>;
  }
  return <UsageLine usage={loaded.value} />;
}
