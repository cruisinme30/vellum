// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { render, screen, within } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { DeviceKeyInfo } from "../../api/api.gen";
import { I18nProvider } from "../../i18n/i18n";

import type { PrivateSpace, PrivateSpacesApi } from "./api";
import { PrivateSpaces } from "./private-spaces-view";

const MINE = "aaaabbbbccccddddeeeeffff00001111";
const OTHER = "22223333444455556666777788889999";

function key(fingerprint: string, label: string, mine: boolean): DeviceKeyInfo {
  return {
    user_id: mine ? "01JDB0USR00000000000000001" : "01JDB0USR00000000000000002",
    public_key: "ab".repeat(32),
    fingerprint,
    label,
    added_at: "2026-10-29T12:00:00Z",
    mine,
  };
}

function space(id: string, name: string, keys: DeviceKeyInfo[]): PrivateSpace {
  return {
    id,
    name,
    keys: { space_id: id, version: keys.length > 0 ? "v1" : "", keys },
  };
}

function setup(spaces: PrivateSpace[]) {
  const listSpaces = vi.fn<PrivateSpacesApi["spaces"]>(() =>
    Promise.resolve(spaces),
  );
  const revoke = vi.fn<PrivateSpacesApi["revoke"]>(() =>
    Promise.resolve({ space_id: "s", removed: [] }),
  );
  const api: PrivateSpacesApi = { spaces: listSpaces, revoke };
  const enrol = vi.fn<(space: string) => Promise<{ lines: string[] }>>(() =>
    Promise.resolve({ lines: ["enrolled laptop"] }),
  );
  const reseal = vi.fn<
    (space: string, trust?: readonly string[]) => Promise<{ lines: string[] }>
  >(() => Promise.resolve({ lines: ["1 resealed"] }));
  const device = {
    deviceKey: () => Promise.resolve({ publicKey: "ab", fingerprint: MINE }),
    enrol,
    reseal,
  };
  return { api, device, listSpaces, revoke, enrol, reseal };
}

async function text(found: Promise<HTMLElement>): Promise<string> {
  const element = await found;
  return element.textContent;
}

describe("private spaces screen", () => {
  it("offers to make a shared drive private, and enrols this device", async () => {
    const user = userEvent.setup();
    const { api, device, enrol, listSpaces } = setup([
      space("01JDB0BRD00000000000000000", "Board", []),
    ]);
    render(
      <I18nProvider>
        <PrivateSpaces api={api} device={device} />
      </I18nProvider>,
    );
    expect(
      await screen.findByText("aaaa bbbb cccc dddd eeee ffff 0000 1111"),
    ).toBeTruthy();
    expect(screen.getByText(/Not private/)).toBeTruthy();
    await user.click(
      screen.getByRole("button", {
        name: "Make private and enrol this device",
      }),
    );
    expect(enrol).toHaveBeenCalledWith("01JDB0BRD00000000000000000");
    expect(await text(screen.findByRole("status"))).toContain("enrolled");
    expect(listSpaces).toHaveBeenCalledTimes(2);
  });

  it("shows the devices, reseals, and removes only one's own", async () => {
    const user = userEvent.setup();
    const { api, device, reseal, revoke } = setup([
      space("01JDB0BRD00000000000000000", "Board", [
        key(MINE, "Ana's laptop", true),
        key(OTHER, "Ben's phone", false),
      ]),
    ]);
    render(
      <I18nProvider>
        <PrivateSpaces api={api} device={device} />
      </I18nProvider>,
    );
    const board = await screen.findByRole("region", { name: "Board" });
    expect(within(board).getByText(/Sealed to 2 devices/)).toBeTruthy();
    expect(within(board).getByText(/this device/)).toBeTruthy();
    expect(within(board).queryByRole("button", { name: /Enrol/ })).toBeNull();
    expect(
      within(board).queryByRole("button", { name: "Remove Ben's phone" }),
    ).toBeNull();
    await user.click(
      within(board).getByRole("button", {
        name: "Seal pages for every device",
      }),
    );
    expect(reseal).toHaveBeenCalledWith("01JDB0BRD00000000000000000");
    expect(await screen.findByText("1 resealed")).toBeTruthy();
    const reloaded = await screen.findByRole("region", { name: "Board" });
    await user.click(
      within(reloaded).getByRole("button", { name: "Remove Ana's laptop" }),
    );
    expect(revoke).toHaveBeenCalledWith("01JDB0BRD00000000000000000", MINE);
  });

  it("explains that enrolling needs the desktop app, and shows failures", async () => {
    const user = userEvent.setup();
    const { api, device, enrol } = setup([
      space("01JDB0BRD00000000000000000", "Board", []),
    ]);
    const { unmount } = render(
      <I18nProvider>
        <PrivateSpaces api={api} device={undefined} />
      </I18nProvider>,
    );
    expect(
      await screen.findByText(/needs the Vellum desktop app/),
    ).toBeTruthy();
    expect(screen.queryByRole("button")).toBeNull();
    unmount();

    enrol.mockRejectedValueOnce(
      new Error("only a manager of the space can do this"),
    );
    render(
      <I18nProvider>
        <PrivateSpaces api={api} device={device} />
      </I18nProvider>,
    );
    await user.click(
      await screen.findByRole("button", { name: /Make private/ }),
    );
    expect(await text(screen.findByRole("alert"))).toContain("only a manager");
  });
});

describe("untrusted device keys", () => {
  it("lists the new keys and seals once the person trusts them", async () => {
    const user = userEvent.setup();
    const { api, device, reseal } = setup([
      space("01JDB0BRD00000000000000000", "Board", [
        key(MINE, "Ana's laptop", true),
      ]),
    ]);
    reseal.mockRejectedValueOnce(
      new Error(
        `Error invoking remote method: untrusted device keys: ${OTHER} (Ben's phone); compare`,
      ),
    );
    render(
      <I18nProvider>
        <PrivateSpaces api={api} device={device} />
      </I18nProvider>,
    );
    await user.click(
      await screen.findByRole("button", {
        name: "Seal pages for every device",
      }),
    );
    const check = await screen.findByRole("region", {
      name: "New devices to check",
    });
    expect(
      within(check).getByText("2222 3333 4444 5555 6666 7777 8888 9999"),
    ).toBeTruthy();
    await user.click(
      within(check).getByRole("button", { name: /trust and seal/ }),
    );
    expect(reseal).toHaveBeenLastCalledWith("01JDB0BRD00000000000000000", [
      OTHER,
    ]);
    expect(await screen.findByText("1 resealed")).toBeTruthy();
    expect(
      screen.queryByRole("region", { name: "New devices to check" }),
    ).toBeNull();
  });
});
