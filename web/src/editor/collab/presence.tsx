// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useCallback, useSyncExternalStore } from "react";
import type { Awareness } from "y-protocols/awareness";

import { useT } from "../../i18n/i18n";

import type { LiveState } from "./provider";

/** Someone else with the page open. */
export interface Peer {
  readonly clientId: number;
  readonly name: string;
  readonly color: string;
  /** Whether they may edit, when the server says; `undefined` in demo mode. */
  readonly canWrite: boolean | undefined;
}

/** The most avatars shown before "+N". */
const SHOWN = 4;

const COLOR = /^#[0-9a-f]{6}$/i;

/**
 * The other people on the page (#98): every awareness state except ours
 * that carries a user. Against a server only clients the server lists are
 * shown, because a name in awareness is whatever that browser claims; the
 * server vouches for who is behind each client ID (ADR 0009).
 */
export function peersOf(awareness: Awareness, live?: LiveState): Peer[] {
  const peers: Peer[] = [];
  for (const [clientId, state] of awareness.getStates()) {
    if (clientId === awareness.clientID) continue;
    const user = (state as { user?: { name?: unknown; color?: unknown } }).user;
    if (typeof user?.name !== "string" || user.name.trim() === "") continue;
    const vouched = live?.peers.get(clientId);
    if (live !== undefined && vouched === undefined) continue;
    peers.push({
      clientId,
      name: user.name.trim().slice(0, 80),
      color:
        typeof user.color === "string" && COLOR.test(user.color)
          ? user.color
          : "#4f5b92",
      canWrite: vouched?.canWrite,
    });
  }
  return peers.toSorted(
    (a, b) => a.name.localeCompare(b.name) || a.clientId - b.clientId,
  );
}

/** Black or white, whichever reads better on `color` (`#rrggbb`). */
export function inkOn(color: string): string {
  const channel = (offset: number) => {
    const value = Number.parseInt(color.slice(offset, offset + 2), 16) / 255;
    return value <= 0.039_28 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  };
  const luminance =
    0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5);
  // Contrast with white is 1.05 / (L + 0.05), with black (L + 0.05) / 0.05.
  return 1.05 / (luminance + 0.05) >= (luminance + 0.05) / 0.05
    ? "#ffffff"
    : "#000000";
}

/** Up to two initials of a name. */
export function initials(name: string): string {
  const words = name.trim().split(/\s+/).filter(Boolean);
  const letters = words.length > 1 ? [words[0], words.at(-1)] : [words[0]];
  return letters
    .map((word) =>
      word === undefined
        ? ""
        : ([...new Intl.Segmenter().segment(word)][0]?.segment ?? ""),
    )
    .join("")
    .toUpperCase();
}

/**
 * Re-renders on awareness changes, keyed so an unchanged list keeps its
 * identity.
 */
function useAwarenessKey(awareness: Awareness): string {
  const subscribe = useCallback(
    (listener: () => void) => {
      awareness.on("change", listener);
      return () => {
        awareness.off("change", listener);
      };
    },
    [awareness],
  );
  const snapshot = useCallback(
    () =>
      [...awareness.getStates()]
        .map(
          ([id, state]) =>
            `${String(id)}:${JSON.stringify((state as { user?: unknown }).user ?? "")}`,
        )
        .join("|"),
    [awareness],
  );
  return useSyncExternalStore(subscribe, snapshot);
}

/** Avatars of the other people on the page, beside the editor's controls. */
export function PresenceBar({
  awareness,
  live,
}: {
  readonly awareness: Awareness;
  readonly live: LiveState | undefined;
}) {
  const t = useT();
  useAwarenessKey(awareness);
  const peers = peersOf(awareness, live);
  if (peers.length === 0) return;
  const shown = peers.slice(0, SHOWN);
  const more = peers.length - shown.length;
  return (
    <ul
      className="vl-presence"
      aria-label={t("presence-label", { count: peers.length })}
    >
      {shown.map((peer) => {
        const label =
          peer.canWrite === false
            ? t("presence-viewing", { name: peer.name })
            : t("presence-editing", { name: peer.name });
        return (
          <li
            key={peer.clientId}
            className="vl-avatar"
            style={{ backgroundColor: peer.color, color: inkOn(peer.color) }}
            title={label}
            aria-label={label}
          >
            <span aria-hidden="true">{initials(peer.name)}</span>
          </li>
        );
      })}
      {more > 0 && (
        <li
          className="vl-avatar vl-avatar-more"
          title={peers
            .slice(SHOWN)
            .map((peer) => peer.name)
            .join(", ")}
          aria-label={t("presence-more", { count: more })}
        >
          <span aria-hidden="true">+{more}</span>
        </li>
      )}
    </ul>
  );
}
