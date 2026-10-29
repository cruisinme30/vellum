// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useEffect, useState } from "react";

import { useT } from "../i18n/i18n";
import { describeError } from "../views/sign-in";

import type { NotificationServices } from "./context";

type DeviceState = "checking" | "on" | "off" | "denied" | "unsupported" | "demo";

function initialState(device: NotificationServices["device"]): DeviceState {
  if (device === undefined) return "demo";
  return device.supported ? "checking" : "unsupported";
}

/** Push on this browser: separate from the per-kind settings, which follow the person. */
export function ThisDevice({ device }: { readonly device: NotificationServices["device"] }) {
  const t = useT();
  const [state, setState] = useState<DeviceState>(() => initialState(device));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | undefined>();

  useEffect(() => {
    if (device?.supported !== true) return;
    let live = true;
    device.subscribed().then(
      (on) => {
        if (live) setState(on ? "on" : "off");
      },
      () => {
        if (live) setState("off");
      },
    );
    return () => {
      live = false;
    };
  }, [device]);

  const run = (action: () => Promise<DeviceState>) => {
    setBusy(true);
    setError(undefined);
    action().then(
      (next) => {
        setBusy(false);
        setState(next);
      },
      (error_: unknown) => {
        setBusy(false);
        setError(describeError(error_, t("notify-device-error")));
      },
    );
  };

  return (
    <section aria-labelledby="notify-device-heading">
      <h2 id="notify-device-heading">{t("notify-device")}</h2>
      <p className="vl-muted" role="status">
        {t(`notify-device-${state}`)}
      </p>
      {device !== undefined && (state === "off" || state === "denied") ? (
        <button
          type="button"
          className="vl-filled-button"
          disabled={busy}
          onClick={() => {
            run(async () => {
              const result = await device.enable();
              return result === "enabled" ? "on" : result;
            });
          }}
        >
          {t("notify-device-enable")}
        </button>
      ) : undefined}
      {device !== undefined && state === "on" ? (
        <button
          type="button"
          className="vl-text-button"
          disabled={busy}
          onClick={() => {
            run(async () => {
              await device.disable();
              return "off";
            });
          }}
        >
          {t("notify-device-disable")}
        </button>
      ) : undefined}
      {error === undefined ? undefined : (
        <p className="vl-field-error" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}
