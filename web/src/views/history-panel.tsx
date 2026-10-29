// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useEffect, useId, useRef, useState, type RefObject } from "react";

import { formatSize } from "../data/format";
import type { PageHistory, PageVersion, VersionAuthor } from "../data/history";
import { useLoaded, type Loaded } from "../data/use-loaded";
import { useT } from "../i18n/i18n";
import { Icon } from "../shell/icon";

/** Props of {@link HistoryPanel}. */
export interface HistoryPanelProperties {
  readonly pageId: string;
  readonly history: PageHistory;
  /** The signed-in person's user ID, to show their versions as "You". */
  readonly meId?: string | undefined;
  /** Whether the person may change the page; viewers only browse. */
  readonly canRestore: boolean;
  /**
   * Writes the editor's unsaved changes as a version before a restore, so
   * they stay in the history. Resolves `false` when that failed.
   */
  readonly saveFirst?: (() => Promise<boolean>) | undefined;
  readonly onClose: () => void;
}

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));

const DATE = new Intl.DateTimeFormat("en", {
  month: "short",
  day: "numeric",
  year: "numeric",
  hour: "numeric",
  minute: "2-digit",
});

function authorKey(author: VersionAuthor, meId: string | undefined): string {
  switch (author.type) {
    case "user": {
      return author.user === meId ? "history-by-you" : "history-by-person";
    }
    case "model": {
      return "history-by-model";
    }
    case "external": {
      return "history-by-external";
    }
    case "system": {
      return "history-by-system";
    }
  }
}

/** A loading line or an error, for a load that has not succeeded. */
function LoadState({
  loaded,
}: {
  readonly loaded: Exclude<Loaded<unknown>, { readonly state: "ready" }>;
}) {
  const t = useT();
  return loaded.state === "loading" ? (
    <p className="vl-muted">{t("history-loading")}</p>
  ) : (
    <p className="vl-field-error" role="alert">
      {loaded.message}
    </p>
  );
}

/** The saved versions, newest first; one can be selected. */
function VersionList({
  versions,
  selected,
  meId,
  onSelect,
}: {
  readonly versions: readonly PageVersion[];
  readonly selected: string | undefined;
  readonly meId: string | undefined;
  readonly onSelect: (id: string | undefined) => void;
}) {
  const t = useT();
  if (versions.length === 0) {
    return <p className="vl-muted">{t("history-empty")}</p>;
  }
  const newest = versions[0]?.id;
  return (
    <ol className="vl-version-list" aria-label={t("history-versions")}>
      {versions.map((version) => (
        <li key={version.id}>
          <button
            type="button"
            className="vl-version"
            aria-pressed={version.id === selected}
            onClick={() => {
              onSelect(version.id === selected ? undefined : version.id);
            }}
          >
            <time dateTime={version.savedAt.toISOString()}>{DATE.format(version.savedAt)}</time>
            <span className="vl-version-meta">
              {t(authorKey(version.author, meId))} · {formatSize(version.size)}
              {version.id === newest ? ` · ${t("history-current")}` : ""}
            </span>
          </button>
        </li>
      ))}
    </ol>
  );
}

/** Asks before a restore, then runs it. */
function RestoreButton({ onRestore }: { readonly onRestore: () => Promise<void> }) {
  const t = useT();
  const [confirming, setConfirming] = useState(false);
  const [restoring, setRestoring] = useState(false);
  if (!confirming) {
    return (
      <button
        type="button"
        className="vl-text-button"
        onClick={() => {
          setConfirming(true);
        }}
      >
        <Icon name="history" size={20} />
        {t("history-restore")}
      </button>
    );
  }
  return (
    <div className="vl-version-confirm">
      <p>{t("history-restore-explain")}</p>
      <button
        type="button"
        className="vl-filled-button"
        disabled={restoring}
        onClick={() => {
          setRestoring(true);
          void onRestore().finally(() => {
            setRestoring(false);
            setConfirming(false);
          });
        }}
      >
        {restoring ? t("history-restoring") : t("history-restore-confirm")}
      </button>
      <button
        type="button"
        className="vl-text-button"
        disabled={restoring}
        onClick={() => {
          setConfirming(false);
        }}
      >
        {t("history-cancel")}
      </button>
    </div>
  );
}

/** One version's text, and Restore when it is not the current one. */
function VersionPreview({
  pageId,
  history,
  version,
  restorable,
  onRestore,
}: {
  readonly pageId: string;
  readonly history: PageHistory;
  readonly version: PageVersion;
  readonly restorable: boolean;
  readonly onRestore: (version: PageVersion) => Promise<void>;
}) {
  const t = useT();
  const text = useLoaded(() => history.read(pageId, version.id), [history, pageId, version.id]);
  return (
    <section className="vl-version-preview" aria-label={t("history-preview")}>
      {text.state !== "ready" && <LoadState loaded={text} />}
      {text.state === "ready" &&
        (text.value === undefined ? (
          <p className="vl-muted">{t("history-not-text")}</p>
        ) : (
          <pre className="vl-version-text">{text.value}</pre>
        ))}
      {restorable && <RestoreButton onRestore={() => onRestore(version)} />}
    </section>
  );
}

/** Focus moves into the panel when it opens and back when it closes; Escape closes it. */
function useDialogFocus(panel: RefObject<HTMLDivElement | null>, onClose: () => void) {
  const close = useRef(onClose);
  useEffect(() => {
    close.current = onClose;
  });
  useEffect(() => {
    const opener = document.activeElement as HTMLElement | null;
    panel.current?.querySelector<HTMLElement>(".vl-history-close")?.focus();
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        close.current();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("keydown", onKey);
      if (opener?.isConnected) {
        opener.focus();
      }
    };
  }, [panel]);
}

/**
 * A page's saved versions in a side panel: newest first, each one's text
 * on selection, and Restore, which makes an old version the page's text
 * again as a new version, so nothing is lost. It is a non-modal dialog:
 * Escape or the close button closes it and focus goes back to the opener.
 */
export function HistoryPanel({
  pageId,
  history,
  meId,
  canRestore,
  saveFirst,
  onClose,
}: HistoryPanelProperties) {
  const t = useT();
  const headingId = useId();
  const panel = useRef<HTMLDivElement>(null);
  useDialogFocus(panel, onClose);
  const [reload, setReload] = useState(0);
  const versions = useLoaded(() => history.list(pageId), [history, pageId, reload]);
  const [selected, setSelected] = useState<string>();
  const [notice, setNotice] = useState<{ kind: "done" | "failed"; message: string }>();

  const list = versions.state === "ready" ? versions.value : [];
  const chosen = list.find((version) => version.id === selected);

  const restore = async (version: PageVersion) => {
    setNotice(undefined);
    try {
      if (saveFirst !== undefined && !(await saveFirst())) {
        setNotice({ kind: "failed", message: t("history-save-first-failed") });
        return;
      }
      await history.restore(pageId, version.id);
      setNotice({
        kind: "done",
        message: t("history-restored", { when: DATE.format(version.savedAt) }),
      });
      setSelected(undefined);
      setReload((n) => n + 1);
    } catch (error) {
      setNotice({ kind: "failed", message: errorText(error) });
    }
  };

  return (
    <div
      ref={panel}
      className="vl-history-panel"
      role="dialog"
      aria-modal="false"
      aria-labelledby={headingId}
    >
      <div className="vl-history-head">
        <h2 id={headingId}>{t("history-title")}</h2>
        <button
          type="button"
          className="vl-icon-button vl-history-close"
          aria-label={t("history-close")}
          onClick={onClose}
        >
          <Icon name="close" />
        </button>
      </div>
      <div role="status" className="vl-history-notice">
        {notice?.kind === "done" ? <p>{notice.message}</p> : undefined}
      </div>
      {notice?.kind === "failed" && (
        <p className="vl-field-error" role="alert">
          {notice.message}
        </p>
      )}
      {versions.state !== "ready" && <LoadState loaded={versions} />}
      {versions.state === "ready" && (
        <VersionList versions={list} selected={selected} meId={meId} onSelect={setSelected} />
      )}
      {chosen !== undefined && (
        <VersionPreview
          key={chosen.id}
          pageId={pageId}
          history={history}
          version={chosen}
          restorable={canRestore && chosen.id !== list[0]?.id}
          onRestore={restore}
        />
      )}
    </div>
  );
}
