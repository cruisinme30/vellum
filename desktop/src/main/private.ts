// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * End-to-end encrypted private spaces from the desktop app (ADR 0043).
 *
 * Every step runs the bundled `vellum` binary: it holds the device key in
 * the OS keychain and does all sealing, so the secret never enters this
 * process or the page. The app only passes the server and the signed-in
 * account's access token, through the environment rather than the
 * command line, where other users of the machine could read it.
 */

/** What a finished `vellum` run printed. */
export interface RunResult {
  readonly code: number;
  readonly stdout: string;
  readonly stderr: string;
}

export interface PrivateDeps {
  /** Runs the binary with these arguments and extra environment. */
  readonly run: (
    commandArguments: readonly string[],
    environment: Readonly<Record<string, string>>,
  ) => Promise<RunResult>;
  /** The machine's name, for the default device label. */
  readonly hostname: () => string;
  /** Where the binary keeps the device keys this device trusts. */
  readonly pinDir: string;
}

/** This device's public key, as members see it. */
export interface DeviceKey {
  readonly publicKey: string;
  readonly fingerprint: string;
}

/** The outcome of enrolling or resealing, in the binary's own words. */
export interface PrivateResult {
  readonly lines: readonly string[];
}

/** Space IDs are 26 Crockford base32 characters. */
const SPACE_ID = /^[0-9A-HJKMNP-TV-Z]{26}$/u;
const MAX_LABEL = 80;

export class PrivateSpaces {
  constructor(
    private readonly server: () => string,
    private readonly accessToken: () => Promise<string | undefined>,
    private readonly deps: PrivateDeps,
  ) {}

  /** This device's key, created in the OS keychain on first use. */
  async deviceKey(): Promise<DeviceKey> {
    const result = await this.deps.run(["private", "key", "--json"], {});
    const parsed = JSON.parse(this.output(result).join("\n")) as Record<string, unknown>;
    const { public_key: publicKey, fingerprint } = parsed;
    if (typeof publicKey !== "string" || typeof fingerprint !== "string")
      throw new Error("The device key could not be read");
    return { publicKey, fingerprint };
  }

  /** Enrols this device in `spaceId` for the signed-in account. */
  async enrol(spaceId: unknown, label?: unknown): Promise<PrivateResult> {
    const space = parseSpaceId(spaceId);
    const name = deviceLabel(label, this.deps.hostname());
    // `--label=` keeps a label that starts with "-" from reading as a flag.
    return this.remote(["enrol", "--space", space, `--label=${name}`]);
  }

  /**
   * Reseals the pages this device can open to the space's current keys,
   * and seals pages left readable. Keys new since the last reseal must be
   * in `trust`, after the person compared their fingerprints.
   */
  async reseal(spaceId: unknown, trust?: unknown): Promise<PrivateResult> {
    const trusted = parseFingerprints(trust).map((fingerprint) => `--trust=${fingerprint}`);
    return this.remote(["reseal", "--space", parseSpaceId(spaceId), ...trusted]);
  }

  private async remote(commandArguments: readonly string[]): Promise<PrivateResult> {
    const token = await this.accessToken();
    if (token === undefined) throw new Error("Sign in before using private spaces");
    const result = await this.deps.run(["private", ...commandArguments], {
      VELLUM_SERVER: this.server(),
      VELLUM_TOKEN: token,
      VELLUM_PRIVATE_DIR: this.deps.pinDir,
    });
    return { lines: this.output(result) };
  }

  private output(result: RunResult): string[] {
    if (result.code !== 0) {
      const reason = result.stderr.trim().split("\n").at(-1) ?? "";
      throw new Error(reason === "" ? "The private-space command failed" : reason);
    }
    return result.stdout
      .split("\n")
      .map((line) => line.trimEnd())
      .filter((line) => line !== "");
  }
}

const FINGERPRINT = /^[0-9a-f]{32}$/u;

/** Validates the fingerprints the person chose to trust. */
function parseFingerprints(input: unknown): string[] {
  if (input === undefined) return [];
  if (!Array.isArray(input) || input.length > 255)
    throw new TypeError("Trusted keys are a list of fingerprints");
  return input.map((value: unknown) => {
    if (typeof value !== "string" || !FINGERPRINT.test(value))
      throw new TypeError("A fingerprint is 32 lowercase hex digits");
    return value;
  });
}

/** Validates a space ID from the page before it reaches a command line. */
export function parseSpaceId(input: unknown): string {
  if (typeof input !== "string" || !SPACE_ID.test(input))
    throw new TypeError("A private space is named by its 26-character ID");
  return input;
}

/** The label members see for this device: the page's, or the machine's name. */
export function deviceLabel(input: unknown, hostname: string): string {
  const chosen = typeof input === "string" ? input : `${hostname} (Vellum desktop)`;
  // Control characters never reach the server or the command line.
  // eslint-disable-next-line no-control-regex
  const clean = chosen.replaceAll(/[\u0000-\u001F\u007F]/gu, " ").trim();
  if (clean === "") throw new TypeError("A device needs a name");
  // Code points, as the server counts them.
  // eslint-disable-next-line @typescript-eslint/no-misused-spread
  return [...clean].slice(0, MAX_LABEL).join("");
}
