// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type {
  AdminTokenView,
  FactorsView,
  SignInView,
  TokenPolicy,
  TokenView,
} from "../../src/api/api.gen";

import { EARLIER, NOW, PRIYA } from "./northstar-work";

/** Priya's sign-in and token settings, and the admin's view of them (D285). */

export const MY_TOKENS: TokenView[] = [
  {
    id: "tok1",
    name: "Claude Desktop on the work laptop",
    scopes: ["read", "write:draft", "work"],
    createdAt: EARLIER,
    lastUsedAt: NOW,
    expiresAt: "2027-01-27T09:30:00Z",
  },
  { id: "tok2", name: "CI export", scopes: ["read"], createdAt: EARLIER },
];

export const TOKEN_POLICY: TokenPolicy = {
  maxDays: 90,
  scopes: ["read", "write:draft", "write:direct", "work", "admin"],
};

export const FACTORS: FactorsView = {
  required: true,
  totp: true,
  recoveryCodesLeft: 7,
  passkeys: [{ id: "pk1", name: "MacBook Touch ID", createdAt: EARLIER, lastUsedAt: NOW }],
};

export const ADMIN_TOKENS: AdminTokenView[] = MY_TOKENS.map((token) => ({
  token,
  owner: { id: PRIYA, name: "Priya Raman", email: "priya@northstar.dev" },
}));

export const SIGN_IN: SignInView = {
  breakGlassAdmins: 1,
  maxTokenDays: 90,
  oidcCallbackUrl: "https://vellum.northstar.dev/api/v1/auth/oidc/callback",
  requireSso: false,
  secondFactor: "admins",
  providers: [
    {
      id: "google",
      kind: "oidc",
      name: "Google Workspace",
      issuer: "https://accounts.google.com",
      clientId: "1234567890-northstar.apps.googleusercontent.com",
      hasClientSecret: true,
      enabled: true,
      autoCreate: true,
      allowedDomains: ["northstar.dev"],
      readOnly: false,
      startUrl: "/api/v1/auth/oidc/google/start",
    },
  ],
};
