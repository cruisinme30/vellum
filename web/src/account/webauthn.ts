// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Passkeys in the browser (issue #523, ADR 0069 D290). The server sends
 * WebAuthn options as JSON with binary fields in base64url; these helpers
 * turn them into what `navigator.credentials` takes, and its answers back
 * into JSON the server reads.
 */

/** Base64url without padding, from bytes. */
export function toBase64Url(bytes: ArrayBuffer | Uint8Array): string {
  const view = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
  let text = "";
  for (const byte of view) text += String.fromCodePoint(byte);
  return btoa(text).replaceAll("+", "-").replaceAll("/", "_").replaceAll("=", "");
}

/** Bytes, from base64url with or without padding. */
export function fromBase64Url(text: string): Uint8Array<ArrayBuffer> {
  const base64 = text.replaceAll("-", "+").replaceAll("_", "/");
  const padded = base64 + "=".repeat((4 - (base64.length % 4)) % 4);
  const binary = atob(padded);
  const bytes = new Uint8Array(new ArrayBuffer(binary.length));
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.codePointAt(index) ?? 0;
  }
  return bytes;
}

interface Descriptor {
  readonly type: "public-key";
  readonly id: string;
}

/** Creation options as the server sends them. */
export interface CreationOptionsJson {
  readonly challenge: string;
  readonly rp: { readonly id: string; readonly name: string };
  readonly user: { readonly id: string; readonly name: string; readonly displayName: string };
  readonly pubKeyCredParams: readonly { readonly type: "public-key"; readonly alg: number }[];
  readonly excludeCredentials?: readonly Descriptor[];
  readonly authenticatorSelection?: AuthenticatorSelectionCriteria;
  readonly attestation?: AttestationConveyancePreference;
  readonly timeout?: number;
}

/** Request options as the server sends them. */
export interface RequestOptionsJson {
  readonly challenge: string;
  readonly rpId: string;
  readonly allowCredentials?: readonly Descriptor[];
  readonly userVerification?: UserVerificationRequirement;
  readonly timeout?: number;
}

/** A new passkey, as the server reads it. */
export interface RegistrationJson {
  readonly id: string;
  readonly response: { readonly clientDataJson: string; readonly attestationObject: string };
}

/** A passkey's signature, as the server reads it. */
export interface AssertionJson {
  readonly id: string;
  readonly response: {
    readonly clientDataJson: string;
    readonly authenticatorData: string;
    readonly signature: string;
  };
}

const descriptors = (list: readonly Descriptor[] | undefined): PublicKeyCredentialDescriptor[] =>
  (list ?? []).map((d) => ({ type: d.type, id: fromBase64Url(d.id) }));

/** Whether this browser can use passkeys at all. */
export function passkeysSupported(): boolean {
  return (
    typeof globalThis.PublicKeyCredential === "function" &&
    "credentials" in navigator &&
    typeof navigator.credentials.create === "function"
  );
}

/** Asks the browser for a new passkey. */
export async function createPasskey(options: CreationOptionsJson): Promise<RegistrationJson> {
  const credential = await navigator.credentials.create({
    publicKey: {
      challenge: fromBase64Url(options.challenge),
      rp: options.rp,
      user: { ...options.user, id: fromBase64Url(options.user.id) },
      pubKeyCredParams: [...options.pubKeyCredParams],
      excludeCredentials: descriptors(options.excludeCredentials),
      ...(options.authenticatorSelection === undefined
        ? {}
        : { authenticatorSelection: options.authenticatorSelection }),
      attestation: options.attestation ?? "none",
      ...(options.timeout === undefined ? {} : { timeout: options.timeout }),
    },
  });
  if (!(credential instanceof PublicKeyCredential)) throw new Error("No passkey was created.");
  const response = credential.response as AuthenticatorAttestationResponse;
  return {
    id: toBase64Url(credential.rawId),
    response: {
      clientDataJson: toBase64Url(response.clientDataJSON),
      attestationObject: toBase64Url(response.attestationObject),
    },
  };
}

/** Asks the browser to sign the challenge with one of the allowed passkeys. */
export async function signWithPasskey(options: RequestOptionsJson): Promise<AssertionJson> {
  const credential = await navigator.credentials.get({
    publicKey: {
      challenge: fromBase64Url(options.challenge),
      rpId: options.rpId,
      allowCredentials: descriptors(options.allowCredentials),
      userVerification: options.userVerification ?? "preferred",
      ...(options.timeout === undefined ? {} : { timeout: options.timeout }),
    },
  });
  if (!(credential instanceof PublicKeyCredential)) throw new Error("No passkey answered.");
  const response = credential.response as AuthenticatorAssertionResponse;
  return {
    id: toBase64Url(credential.rawId),
    response: {
      clientDataJson: toBase64Url(response.clientDataJSON),
      authenticatorData: toBase64Url(response.authenticatorData),
      signature: toBase64Url(response.signature),
    },
  };
}
