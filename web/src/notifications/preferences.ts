// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { Http } from "../api/http";

/**
 * Notification preferences: which channels each kind of notification uses,
 * how long email waits, what is muted and whether everything is snoozed.
 *
 * The wire body is the server's `PreferencesView` (`/api/v1/notifications/preferences`):
 *
 * ```json
 * { "channels": { "mentioned": ["in_app", "push"] },
 *   "email_delay_minutes": 10, "muted": ["ENG-142"], "snoozed_until": null }
 * ```
 *
 * The server lists every kind; the app keeps only those that differ from
 * the defaults.
 * A kind missing from `channels` uses its defaults, which
 * {@link defaultChannels} copies from `NotificationKind::default_channels`.
 */

/** A delivery channel (`vellum_notify::Channel`). */
export type Channel = "in_app" | "email" | "push";

/** Every channel, in the order the settings table shows them. */
export const CHANNELS: readonly Channel[] = ["in_app", "email", "push"];

/** What happened (`vellum_notify::NotificationKind`), in its stable order. */
export const KINDS = [
  "issue_assigned",
  "mentioned",
  "commented",
  "state_changed",
  "triage_new",
  "sla_high_risk",
  "sla_breached",
  "review_requested",
  "project_update",
  "reminder",
  "pull_request",
] as const;

export type NotificationKind = (typeof KINDS)[number];

/** One person's settings, in app form. */
export interface NotificationPreferences {
  /** Channels per kind; a missing kind uses {@link defaultChannels}. */
  readonly channels: Readonly<Partial<Record<NotificationKind, readonly Channel[]>>>;
  /** Minutes email waits for the inbox entry to be read. */
  readonly emailDelayMinutes: number;
  /** Subjects (issue keys, document IDs) that stay quiet. */
  readonly muted: readonly string[];
  /** Non-urgent push and email wait until this instant (ISO 8601), if set. */
  readonly snoozedUntil?: string | undefined;
}

const ALL: readonly Channel[] = ["in_app", "email", "push"];
const DEFAULTS: Record<NotificationKind, readonly Channel[]> = {
  issue_assigned: ALL,
  mentioned: ALL,
  sla_breached: ALL,
  review_requested: ALL,
  reminder: ALL,
  sla_high_risk: ["in_app", "push"],
  triage_new: ["in_app", "push"],
  commented: ["in_app", "email"],
  project_update: ["in_app", "email"],
  state_changed: ["in_app"],
  pull_request: ["in_app"],
};

/** The channels on by default for a new member. */
export function defaultChannels(kind: NotificationKind): readonly Channel[] {
  return DEFAULTS[kind];
}

/** Urgent kinds ignore mutes, snoozes and the email delay. */
export function isUrgent(kind: NotificationKind): boolean {
  return kind === "sla_breached" || kind === "reminder";
}

/** The channels on for `kind`. */
export function channelsFor(
  preferences: NotificationPreferences,
  kind: NotificationKind,
): readonly Channel[] {
  return preferences.channels[kind] ?? defaultChannels(kind);
}

/** Turns one channel of one kind on or off. Kinds left at their defaults stay out of the map. */
export function withChannel(
  preferences: NotificationPreferences,
  kind: NotificationKind,
  channel: Channel,
  on: boolean,
): NotificationPreferences {
  const current = new Set(channelsFor(preferences, kind));
  if (on) current.add(channel);
  else current.delete(channel);
  const next = CHANNELS.filter((each) => current.has(each));
  const defaults = defaultChannels(kind);
  const isDefault =
    next.length === defaults.length && next.every((each) => defaults.includes(each));
  const channels: Partial<Record<NotificationKind, readonly Channel[]>> = {};
  for (const each of KINDS) {
    if (each !== kind) {
      const list = preferences.channels[each];
      if (list !== undefined) channels[each] = list;
    } else if (!isDefault) {
      channels[each] = next;
    }
  }
  return { ...preferences, channels };
}

/** A new member's settings. */
export const DEFAULT_PREFERENCES: NotificationPreferences = {
  channels: {},
  emailDelayMinutes: 10,
  muted: [],
};

/** The server's `PreferencesView`. */
export interface PreferencesWire {
  readonly channels: Readonly<Record<string, readonly string[]>>;
  /** Minutes email waits for the inbox entry to be read (0 to 1440). */
  readonly email_delay_minutes: number;
  readonly muted: readonly string[];
  /** `null` or absent: not snoozed. */
  readonly snoozed_until?: string | null;
}

/** The longest email delay the server accepts: a day. */
const MAX_EMAIL_DELAY = 1440;

const isKind = (value: string): value is NotificationKind =>
  (KINDS as readonly string[]).includes(value);
const isChannel = (value: string): value is Channel =>
  (CHANNELS as readonly string[]).includes(value);

const sameChannels = (a: readonly Channel[], b: readonly Channel[]) =>
  a.length === b.length && a.every((each) => b.includes(each));

/**
 * Wire → app. Unknown kinds and channels (from a newer server) are
 * dropped, and so are kinds left at their defaults.
 */
export function fromWire(wire: PreferencesWire): NotificationPreferences {
  const channels: Partial<Record<NotificationKind, readonly Channel[]>> = {};
  for (const [kind, list] of Object.entries(wire.channels)) {
    if (!isKind(kind)) continue;
    const known = CHANNELS.filter((channel) => list.includes(channel));
    if (!sameChannels(known, defaultChannels(kind))) channels[kind] = known;
  }
  const delay = Number.isFinite(wire.email_delay_minutes)
    ? wire.email_delay_minutes
    : DEFAULT_PREFERENCES.emailDelayMinutes;
  return {
    channels,
    emailDelayMinutes: Math.min(MAX_EMAIL_DELAY, Math.max(0, Math.floor(delay))),
    muted: [...new Set(wire.muted)],
    snoozedUntil: wire.snoozed_until ?? undefined,
  };
}

/** App → wire. Kinds left out keep their defaults on the server. */
export function toWire(preferences: NotificationPreferences): PreferencesWire {
  const channels: Record<string, readonly string[]> = {};
  for (const kind of KINDS) {
    const list = preferences.channels[kind];
    if (list !== undefined) channels[kind] = list.filter((channel) => isChannel(channel));
  }
  const wire = {
    channels,
    email_delay_minutes: Math.min(
      MAX_EMAIL_DELAY,
      Math.max(0, Math.round(preferences.emailDelayMinutes)),
    ),
    muted: preferences.muted,
  };
  // A missing `snoozed_until` means not snoozed.
  return preferences.snoozedUntil === undefined
    ? wire
    : { ...wire, snoozed_until: preferences.snoozedUntil };
}

/** Where preferences are kept: the server, or the demo's memory. */
export interface NotificationPreferencesStore {
  load(): Promise<NotificationPreferences>;
  /** Saves and answers with what was stored. */
  save(preferences: NotificationPreferences): Promise<NotificationPreferences>;
}

/** The server's preferences of the signed-in person. */
export const PREFERENCES_PATH = "/api/v1/notifications/preferences";

/** Reads and writes the signed-in person's preferences on a Vellum server. */
export function createHttpNotificationPreferencesStore(http: Http): NotificationPreferencesStore {
  return {
    load: async () => fromWire(await http.get<PreferencesWire>(PREFERENCES_PATH)),
    save: async (preferences) =>
      fromWire(await http.send<PreferencesWire>("PUT", PREFERENCES_PATH, toWire(preferences))),
  };
}

const NORTHSTAR_PREFERENCES: NotificationPreferences = {
  ...DEFAULT_PREFERENCES,
  channels: { commented: ["in_app", "email", "push"] },
  muted: ["ENG-118", "ENG-142"],
};

/** Northstar demo: Priya muted two noisy issues and gets comments by push too. */
export function createMockNotificationPreferencesStore(
  initial?: NotificationPreferences,
): NotificationPreferencesStore {
  let stored = initial ?? NORTHSTAR_PREFERENCES;
  return {
    load: () => Promise.resolve(stored),
    save: (preferences) => {
      stored = preferences;
      return Promise.resolve(stored);
    },
  };
}
