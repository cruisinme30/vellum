// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { ApiError } from "../api/http";
import { renderApp, renderWithProviders } from "../test/render-app";

import { NotificationsProvider, type NotificationServices } from "./context";
import type { PushDevice } from "./device";
import {
  NotificationSettingsView,
  snoozeActive,
} from "./notification-settings";
import {
  createMockNotificationPreferencesStore,
  DEFAULT_PREFERENCES,
  type NotificationPreferencesStore,
} from "./preferences";

const NOW = Date.parse("2026-10-29T12:00:00Z");

function renderSettings(services: NotificationServices) {
  return renderWithProviders(
    <NotificationsProvider services={services}>
      <NotificationSettingsView now={() => NOW} />
    </NotificationsProvider>,
    "/settings/notifications",
  );
}

function fakeDevice(overrides: Partial<PushDevice> = {}): PushDevice {
  return {
    supported: true,
    subscribed: vi.fn(() => Promise.resolve(false)),
    enable: vi.fn(() => Promise.resolve("enabled" as const)),
    disable: vi.fn(() => Promise.resolve()),
    ...overrides,
  };
}

describe("the notification settings screen", () => {
  it("opens from the inbox and shows the Northstar demo", async () => {
    const { user, location } = renderApp("/inbox");
    await user.click(
      screen.getByRole("link", { name: "Notification settings" }),
    );
    expect(location()).toBe("/settings/notifications");
    expect(
      await screen.findByRole("heading", { level: 1, name: "Notifications" }),
    ).toBeDefined();
    // The demo gets comments by push and has two muted issues.
    expect(
      screen.getByRole("checkbox", {
        name: "Push for Comments on things you follow",
      }),
    ).toHaveProperty("checked", true);
    expect(
      screen.getByRole("button", { name: "Unmute ENG-142" }),
    ).toBeDefined();
    expect(screen.getByText(/the demo can't send them/)).toBeDefined();
  });

  it("saves channel, email, snooze and mute changes in one go", async () => {
    const store = createMockNotificationPreferencesStore(DEFAULT_PREFERENCES);
    const save = vi.spyOn(store, "save");
    const { user } = renderSettings({ store });

    const email = await screen.findByRole("checkbox", {
      name: "Email for Mentions",
    });
    const saveButton = screen.getByRole("button", { name: "Save" });
    expect(saveButton).toHaveProperty("disabled", true);

    await user.click(email);
    expect(email).toHaveProperty("checked", false);
    await user.selectOptions(screen.getByLabelText("Send email after"), "30");
    await user.click(screen.getByRole("button", { name: "1 day" }));
    expect(screen.getByText(/^Snoozed until/)).toBeDefined();
    await user.type(
      screen.getByLabelText("Issue key or page ID"),
      "  ENG-7{Enter}",
    );
    expect(screen.getByText("ENG-7")).toBeDefined();

    await user.click(saveButton);
    expect(save).toHaveBeenCalledWith({
      channels: { mentioned: ["in_app", "push"] },
      emailDelayMinutes: 30,
      muted: ["ENG-7"],
      snoozedUntil: "2026-10-30T12:00:00.000Z",
    });
    expect(await screen.findByText("Saved.")).toBeDefined();
    expect(saveButton).toHaveProperty("disabled", true);
  });

  it("discards unsaved changes, resumes a snooze and unmutes", async () => {
    const store = createMockNotificationPreferencesStore({
      ...DEFAULT_PREFERENCES,
      muted: ["ENG-1"],
      snoozedUntil: "2026-10-29T13:00:00Z",
    });
    const { user } = renderSettings({ store });
    await user.click(await screen.findByRole("button", { name: "Resume now" }));
    await user.click(screen.getByRole("button", { name: "Unmute ENG-1" }));
    expect(screen.getByText("Nothing is muted.")).toBeDefined();
    expect(screen.getByRole("button", { name: "1 hour" })).toBeDefined();

    await user.click(screen.getByRole("button", { name: "Discard changes" }));
    expect(screen.getByText("ENG-1")).toBeDefined();
    expect(screen.getByText(/^Snoozed until/)).toBeDefined();
  });

  it("ignores blank and duplicate mutes", async () => {
    const store = createMockNotificationPreferencesStore({
      ...DEFAULT_PREFERENCES,
      muted: ["A"],
    });
    const { user } = renderSettings({ store });
    const field = await screen.findByLabelText("Issue key or page ID");
    await user.type(field, "A");
    await user.click(screen.getByRole("button", { name: "Mute" }));
    await user.clear(field);
    await user.type(field, "   {Enter}");
    expect(screen.getAllByRole("listitem")).toHaveLength(1);
    expect(screen.getByRole("button", { name: "Save" })).toHaveProperty(
      "disabled",
      true,
    );
  });

  it("offers a delay the server set that is not in the list", async () => {
    const store = createMockNotificationPreferencesStore({
      ...DEFAULT_PREFERENCES,
      emailDelayMinutes: 15,
    });
    renderSettings({ store });
    const select = await screen.findByLabelText("Send email after");
    expect(select).toHaveProperty("value", "15");
    expect(
      within(select)
        .getAllByRole("option")
        .map((option) => option.textContent),
    ).toEqual([
      "Straight away",
      "5 minutes",
      "10 minutes",
      "15 minutes",
      "30 minutes",
      "60 minutes",
    ]);
  });

  it("shows load and save errors with the request ID, and retries", async () => {
    const failing: NotificationPreferencesStore = {
      load: vi
        .fn()
        .mockRejectedValueOnce(
          new ApiError("unavailable", "The server is busy.", {
            requestId: "01JR",
          }),
        )
        .mockResolvedValue(DEFAULT_PREFERENCES),
      save: vi
        .fn()
        .mockRejectedValue(new ApiError("invalid_input", "Too many mutes.")),
    };
    const { user } = renderSettings({ store: failing });
    const loadAlert = await screen.findByRole("alert");
    expect(loadAlert.textContent).toContain(
      "The server is busy. (request 01JR)",
    );
    await user.click(screen.getByRole("button", { name: "Try again" }));
    await user.click(
      await screen.findByRole("checkbox", { name: "Push for Status changes" }),
    );
    await user.click(screen.getByRole("button", { name: "Save" }));
    const saveAlert = await screen.findByRole("alert");
    expect(saveAlert.textContent).toContain("Too many mutes.");
    // The edit is still there to save again.
    expect(
      screen.getByRole("checkbox", { name: "Push for Status changes" }),
    ).toHaveProperty("checked", true);
  });

  it("marks urgent kinds and labels every checkbox", async () => {
    renderSettings({
      store: createMockNotificationPreferencesStore(DEFAULT_PREFERENCES),
    });
    const table = await screen.findByRole("table");
    expect(within(table).getAllByRole("checkbox")).toHaveLength(33);
    expect(within(table).getAllByText("Urgent")).toHaveLength(2);
    expect(
      within(table).getByRole("rowheader", { name: /SLA breached/ }),
    ).toBeDefined();
  });
});

describe("push on this device", () => {
  const store = createMockNotificationPreferencesStore(DEFAULT_PREFERENCES);

  it("turns push on and off", async () => {
    const enable = vi.fn(() => Promise.resolve("enabled" as const));
    const disable = vi.fn(() => Promise.resolve());
    const device = fakeDevice({ enable, disable });
    const { user } = renderSettings({ store, device });
    expect(
      await screen.findByText("Push notifications are off for this browser."),
    ).toBeDefined();
    await user.click(screen.getByRole("button", { name: "Turn on push" }));
    expect(enable).toHaveBeenCalledOnce();
    expect(
      await screen.findByText("Push notifications are on for this browser."),
    ).toBeDefined();
    await user.click(screen.getByRole("button", { name: "Turn off push" }));
    expect(disable).toHaveBeenCalledOnce();
    expect(
      await screen.findByText("Push notifications are off for this browser."),
    ).toBeDefined();
  });

  it("shows an error when push cannot be turned off", async () => {
    const device = fakeDevice({
      subscribed: () => Promise.resolve(true),
      disable: () => Promise.reject(new Error("offline")),
      enable: () => Promise.resolve("denied"),
    });
    const { user } = renderSettings({ store, device });
    await user.click(
      await screen.findByRole("button", { name: "Turn off push" }),
    );
    expect(
      await screen.findByText("Push couldn't be changed. Try again."),
    ).toBeDefined();
  });

  it("says when permission is denied", async () => {
    const device = fakeDevice({ enable: () => Promise.resolve("denied") });
    const { user } = renderSettings({ store, device });
    await user.click(
      await screen.findByRole("button", { name: "Turn on push" }),
    );
    expect(
      await screen.findByText(/This browser blocked notifications/),
    ).toBeDefined();
    expect(screen.getByRole("button", { name: "Turn on push" })).toBeDefined();
  });

  it("treats a failed check as off", async () => {
    renderSettings({
      store,
      device: fakeDevice({ subscribed: () => Promise.reject(new Error("x")) }),
    });
    expect(
      await screen.findByText("Push notifications are off for this browser."),
    ).toBeDefined();
  });

  it("says when the browser cannot push", async () => {
    renderSettings({ store, device: fakeDevice({ supported: false }) });
    await waitFor(() => {
      expect(
        screen.getByText(/can't receive push notifications/),
      ).toBeDefined();
    });
    expect(screen.queryByRole("button", { name: "Turn on push" })).toBeNull();
  });
});

describe("snoozeActive", () => {
  it("is on only for a future, readable time", () => {
    expect(snoozeActive(undefined, NOW)).toBe(false);
    expect(snoozeActive("2026-10-29T11:00:00Z", NOW)).toBe(false);
    expect(snoozeActive("not a time", NOW)).toBe(false);
    expect(snoozeActive("2026-10-29T13:00:00Z", NOW)).toBe(true);
  });
});
