// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { afterEach, describe, expect, it, vi } from "vitest";

import {
  createPasskey,
  fromBase64Url,
  passkeysSupported,
  signWithPasskey,
  toBase64Url,
  type CreationOptionsJson,
  type RequestOptionsJson,
} from "./webauthn";

/** Stands in for the browser's credential class. */
class FakeCredential {
  constructor(
    readonly rawId: ArrayBuffer,
    readonly response: Record<string, ArrayBuffer>,
  ) {}
}

const bytes = (...values: number[]) => new Uint8Array(values).buffer;

function browser(answer?: unknown) {
  const create = vi.fn<
    (options: CredentialCreationOptions) => Promise<unknown>
  >(() => Promise.resolve(answer));
  const get = vi.fn<(options: CredentialRequestOptions) => Promise<unknown>>(
    () => Promise.resolve(answer),
  );
  vi.stubGlobal("PublicKeyCredential", FakeCredential);
  vi.stubGlobal("navigator", { credentials: { create, get } });
  return { create, get };
}

const CREATION: CreationOptionsJson = {
  challenge: "AQID",
  rp: { id: "vellum.northstar.dev", name: "Vellum" },
  user: { id: "BAU", name: "priya@northstar.dev", displayName: "Priya" },
  pubKeyCredParams: [{ type: "public-key", alg: -7 }],
};

const REQUEST: RequestOptionsJson = {
  challenge: "AQID",
  rpId: "vellum.northstar.dev",
};

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("passkeys in the browser", () => {
  it("knows when the browser has none", () => {
    // eslint-disable-next-line unicorn/no-useless-undefined -- stubGlobal takes a value: none here
    vi.stubGlobal("PublicKeyCredential", undefined);
    expect(passkeysSupported()).toBe(false);
    browser();
    expect(passkeysSupported()).toBe(true);
    vi.stubGlobal("navigator", { credentials: {} });
    expect(passkeysSupported()).toBe(false);
  });

  it("creates a passkey with the defaults the server leaves out", async () => {
    const { create } = browser(
      new FakeCredential(bytes(9), {
        clientDataJSON: bytes(1),
        attestationObject: bytes(2),
      }),
    );
    const made = await createPasskey(CREATION);
    expect(made).toEqual({
      id: "CQ",
      response: { clientDataJson: "AQ", attestationObject: "Ag" },
    });
    const options = create.mock.calls[0]?.[0].publicKey;
    expect(options?.attestation).toBe("none");
    expect(options?.excludeCredentials).toEqual([]);
    expect(options).not.toHaveProperty("timeout");
    expect(options).not.toHaveProperty("authenticatorSelection");
    expect([...new Uint8Array(options?.user.id as ArrayBuffer)]).toEqual([
      4, 5,
    ]);
  });

  it("passes on every option the server sends", async () => {
    const { create } = browser(
      new FakeCredential(bytes(9), {
        clientDataJSON: bytes(1),
        attestationObject: bytes(2),
      }),
    );
    await createPasskey({
      ...CREATION,
      excludeCredentials: [{ type: "public-key", id: "CQ" }],
      authenticatorSelection: { residentKey: "preferred" },
      attestation: "direct",
      timeout: 60_000,
    });
    const options = create.mock.calls[0]?.[0].publicKey;
    expect(options?.attestation).toBe("direct");
    expect(options?.timeout).toBe(60_000);
    expect(options?.authenticatorSelection).toEqual({
      residentKey: "preferred",
    });
    expect(options?.excludeCredentials?.length).toBe(1);
  });

  it("says when no passkey was made or answered", async () => {
    browser();
    await expect(createPasskey(CREATION)).rejects.toThrow(
      "No passkey was created.",
    );
    await expect(signWithPasskey(REQUEST)).rejects.toThrow(
      "No passkey answered.",
    );
  });

  it("signs with a passkey, with and without the optional settings", async () => {
    const { get } = browser(
      new FakeCredential(bytes(7), {
        clientDataJSON: bytes(1),
        authenticatorData: bytes(2),
        signature: bytes(3),
      }),
    );
    const signed = await signWithPasskey(REQUEST);
    expect(signed).toEqual({
      id: "Bw",
      response: {
        clientDataJson: "AQ",
        authenticatorData: "Ag",
        signature: "Aw",
      },
    });
    let options = get.mock.calls[0]?.[0].publicKey;
    expect(options?.userVerification).toBe("preferred");
    expect(options).not.toHaveProperty("timeout");
    await signWithPasskey({
      ...REQUEST,
      allowCredentials: [{ type: "public-key", id: "Bw" }],
      userVerification: "required",
      timeout: 30_000,
    });
    options = get.mock.calls[1]?.[0].publicKey;
    expect(options?.userVerification).toBe("required");
    expect(options?.timeout).toBe(30_000);
    expect(options?.allowCredentials?.length).toBe(1);
  });

  it("reads base64url with and without padding", () => {
    expect([...fromBase64Url("_-8")]).toEqual([255, 239]);
    expect([...fromBase64Url("_-8=")]).toEqual([255, 239]);
    expect(toBase64Url(new Uint8Array([255, 239]))).toBe("_-8");
  });
});
