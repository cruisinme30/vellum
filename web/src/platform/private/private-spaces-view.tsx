// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState } from "react";

import { Ready, ServerScreen } from "../../data/server-apis";
import { useLoaded } from "../../data/use-loaded";
import { useT } from "../../i18n/i18n";
import { describeError } from "../../views/sign-in";
import { desktopBridge, type DesktopBridge } from "../platform";

import type { PrivateSpace, PrivateSpacesApi } from "./api";

type DeviceActions = NonNullable<DesktopBridge["privateSpaces"]>;

/** What the binary says before the fingerprints it does not trust yet. */
const UNTRUSTED = /untrusted device keys: ([0-9a-f,]+)/u;

/** The fingerprints a reseal refused, if that is why it failed. */
function untrustedKeys(message: string): string[] | undefined {
  return UNTRUSTED.exec(message)?.[1]?.split(",");
}

/** A fingerprint in groups of four, easier to read aloud and compare. */
function grouped(fingerprint: string): string {
  return fingerprint.match(/.{1,4}/gu)?.join(" ") ?? fingerprint;
}

/** Private spaces: which devices each shared drive is sealed to (ADR 0043). */
export function PrivateSpacesView() {
  const t = useT();
  return (
    <ServerScreen api="privateSpaces" help="app.private-spaces" title={t("private-title")}>
      {(api) => <PrivateSpaces api={api} device={desktopBridge()?.privateSpaces} />}
    </ServerScreen>
  );
}

/** The screen's body; `device` is absent outside the desktop app. */
export function PrivateSpaces({
  api,
  device,
}: {
  readonly api: PrivateSpacesApi;
  readonly device: DeviceActions | undefined;
}) {
  const t = useT();
  const [revision, setRevision] = useState(0);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<{
    text: string;
    failed: boolean;
    /** Keys a reseal refused, and the space, while the person decides. */
    untrusted?: { space: string; keys: string[] };
  }>();
  const key = useLoaded(
    async () => (device === undefined ? undefined : device.deviceKey()),
    [device],
  );
  const spaces = useLoaded(() => api.spaces(), [api, revision]);
  const mine = key.state === "ready" ? key.value?.fingerprint : undefined;

  const run = (action: () => Promise<readonly string[]>, space?: string) => {
    setBusy(true);
    setMessage(undefined);
    action().then(
      (lines) => {
        setBusy(false);
        setMessage({ text: lines.join(" "), failed: false });
        setRevision((r) => r + 1);
      },
      (error: unknown) => {
        setBusy(false);
        const fallback = error instanceof Error ? error.message : t("private-error");
        const keys = untrustedKeys(fallback);
        setMessage(
          keys === undefined || space === undefined
            ? { text: describeError(error, fallback), failed: true }
            : { text: t("private-untrusted"), failed: true, untrusted: { space, keys } },
        );
      },
    );
  };

  return (
    <>
      <p>{t("private-intro")}</p>
      <section aria-labelledby="private-device-heading">
        <h2 id="private-device-heading">{t("private-device")}</h2>
        {device === undefined ? (
          <p className="vl-muted">{t("private-device-desktop-only")}</p>
        ) : (
          <Ready loaded={key}>
            {(found) => (
              <p>
                {t("private-device-fingerprint")}{" "}
                <code>{found === undefined ? "" : grouped(found.fingerprint)}</code>
              </p>
            )}
          </Ready>
        )}
      </section>
      {message === undefined ? undefined : (
        <p
          className={message.failed ? "vl-field-error" : "vl-muted"}
          role={message.failed ? "alert" : "status"}
        >
          {message.text}
        </p>
      )}
      {message?.untrusted === undefined || device === undefined ? undefined : (
        <UntrustedKeys
          keys={message.untrusted.keys}
          busy={busy}
          onTrust={() => {
            const { space, keys } = message.untrusted ?? { space: "", keys: [] };
            run(async () => {
              const { lines } = await device.reseal(space, keys);
              return lines;
            }, space);
          }}
        />
      )}
      <Ready loaded={spaces}>
        {(list) =>
          list.length === 0 ? (
            <p className="vl-muted">{t("private-no-spaces")}</p>
          ) : (
            list.map((space) => (
              <SpaceCard
                key={space.id}
                space={space}
                mine={mine}
                busy={busy}
                device={device}
                api={api}
                run={run}
              />
            ))
          )
        }
      </Ready>
    </>
  );
}

function SpaceCard({
  space,
  mine,
  busy,
  device,
  api,
  run,
}: {
  readonly space: PrivateSpace;
  readonly mine: string | undefined;
  readonly busy: boolean;
  readonly device: DeviceActions | undefined;
  readonly api: PrivateSpacesApi;
  readonly run: (action: () => Promise<readonly string[]>, space?: string) => void;
}) {
  const t = useT();
  const heading = `private-space-${space.id}`;
  const keys = space.keys.keys;
  const enrolled = mine !== undefined && keys.some((k) => k.fingerprint === mine);
  return (
    <section aria-labelledby={heading}>
      <h2 id={heading}>{space.name}</h2>
      <p className="vl-muted">
        {keys.length === 0
          ? t("private-not-private")
          : t("private-sealed-to", { count: keys.length })}
      </p>
      {keys.length === 0 ? undefined : (
        <ul>
          {keys.map((k) => (
            <li key={k.fingerprint}>
              {k.label} <code>{grouped(k.fingerprint)}</code>
              {k.fingerprint === mine ? ` ${t("private-this-device")}` : ""}
              {k.mine ? (
                <button
                  type="button"
                  className="vl-text-button"
                  disabled={busy}
                  aria-label={t("private-remove-label", { device: k.label })}
                  onClick={() => {
                    run(async () => {
                      await api.revoke(space.id, k.fingerprint);
                      return [t("private-removed", { device: k.label })];
                    });
                  }}
                >
                  {t("private-remove")}
                </button>
              ) : undefined}
            </li>
          ))}
        </ul>
      )}
      {device !== undefined && mine !== undefined && !enrolled ? (
        <button
          type="button"
          className="vl-filled-button"
          disabled={busy}
          onClick={() => {
            run(async () => {
              const { lines } = await device.enrol(space.id);
              return lines;
            });
          }}
        >
          {keys.length === 0 ? t("private-make-private") : t("private-enrol")}
        </button>
      ) : undefined}
      {device !== undefined && enrolled ? (
        <button
          type="button"
          className="vl-text-button"
          disabled={busy}
          onClick={() => {
            run(async () => {
              const { lines } = await device.reseal(space.id);
              return lines;
            }, space.id);
          }}
        >
          {t("private-reseal")}
        </button>
      ) : undefined}
    </section>
  );
}

/** Device keys new since this device last sealed, to compare before trusting. */
function UntrustedKeys({
  keys,
  busy,
  onTrust,
}: {
  readonly keys: readonly string[];
  readonly busy: boolean;
  readonly onTrust: () => void;
}) {
  const t = useT();
  return (
    <section aria-labelledby="private-untrusted-heading">
      <h2 id="private-untrusted-heading">{t("private-untrusted-heading")}</h2>
      <p>{t("private-untrusted-help")}</p>
      <ul>
        {keys.map((fingerprint) => (
          <li key={fingerprint}>
            <code>{grouped(fingerprint)}</code>
          </li>
        ))}
      </ul>
      <button type="button" className="vl-filled-button" disabled={busy} onClick={onTrust}>
        {t("private-trust")}
      </button>
    </section>
  );
}
