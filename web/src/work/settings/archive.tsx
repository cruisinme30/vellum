// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useId, useState } from "react";

import { failureText } from "../../data/failure";
import { useT } from "../../i18n/i18n";
import type { TeamOut, WorkApi } from "../api";

/**
 * Archive or restore the team (D303). Archiving asks first: the team
 * leaves team lists and takes no new issues, though its issues stay.
 */
export function ArchiveSection({
  api,
  team,
  onChanged,
}: {
  readonly api: WorkApi;
  readonly team: TeamOut;
  readonly onChanged: () => void;
}) {
  const t = useT();
  const heading = useId();
  const [asking, setAsking] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const change = (archived: boolean) => {
    setBusy(true);
    setError(undefined);
    api.setTeamArchived(team.key, archived).then(
      () => {
        setBusy(false);
        setAsking(false);
        onChanged();
      },
      (error_: unknown) => {
        setBusy(false);
        setError(failureText(error_));
      },
    );
  };
  return (
    <section
      className="vl-settings-section vl-settings-archive"
      aria-labelledby={heading}
    >
      <h2 id={heading}>{t("team-settings-archive-heading")}</h2>
      {team.archived && (
        <>
          <p role="status">
            {t("team-settings-archived-note", { name: team.name })}
          </p>
          <button
            type="button"
            className="vl-filled-button"
            disabled={busy}
            onClick={() => {
              change(false);
            }}
          >
            {t("team-settings-restore")}
          </button>
        </>
      )}
      {!team.archived && asking && (
        <div
          role="group"
          aria-label={t("team-settings-archive-confirm-label", {
            name: team.name,
          })}
        >
          <p>{t("team-settings-archive-confirm", { name: team.name })}</p>
          <button
            type="button"
            className="vl-filled-button"
            disabled={busy}
            onClick={() => {
              change(true);
            }}
          >
            {t("team-settings-archive-yes")}
          </button>
          <button
            type="button"
            className="vl-text-button"
            disabled={busy}
            onClick={() => {
              setAsking(false);
            }}
          >
            {t("work-cancel")}
          </button>
        </div>
      )}
      {!team.archived && !asking && (
        <>
          <p className="vl-muted">{t("team-settings-archive-hint")}</p>
          <button
            type="button"
            className="vl-text-button"
            onClick={() => {
              setAsking(true);
            }}
          >
            {t("team-settings-archive", { name: team.name })}
          </button>
        </>
      )}
      {error !== undefined && (
        <p className="vl-field-error" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}
