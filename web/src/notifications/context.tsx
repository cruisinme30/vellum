// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { createContext, use, type ReactNode } from "react";

import type { PushDevice } from "./device";
import type { NotificationPreferencesStore } from "./preferences";

/** What the notification settings screen talks to. */
export interface NotificationServices {
  readonly store: NotificationPreferencesStore;
  /** This browser's push; absent in demo mode, where no server can send pushes. */
  readonly device?: PushDevice | undefined;
}

const NotificationsContext = createContext<NotificationServices | undefined>(undefined);

/** Provides the notification services to the app. */
export function NotificationsProvider({
  services,
  children,
}: {
  readonly services: NotificationServices;
  readonly children: ReactNode;
}) {
  return <NotificationsContext value={services}>{children}</NotificationsContext>;
}

/** The notification services; throws outside a provider. */
export function useNotificationServices(): NotificationServices {
  const services = use(NotificationsContext);
  if (!services) {
    throw new Error("Vellum: useNotificationServices needs a NotificationsProvider");
  }
  return services;
}
