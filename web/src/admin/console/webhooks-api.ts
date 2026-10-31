// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * The webhooks admin (`/api/v1/admin/webhooks`, issue #149), over the
 * app's HTTP client so writes carry the CSRF token.
 */

import type {
  DeliveryList,
  DeliveryView,
  HookCreated,
  HookInput,
  HookList,
  HookView,
} from "../../api/api.gen";
import type { Http } from "../../api/http";

export type {
  DeliveryView,
  HookInput,
  HookList,
  HookView,
} from "../../api/api.gen";

/** The webhooks admin's server operations. */
export interface WebhooksApi {
  list(): Promise<HookList>;
  /** Adds a hook; the answer holds its secret, shown once. */
  create(input: HookInput): Promise<HookCreated>;
  update(id: string, input: HookInput, enabled: boolean): Promise<HookView>;
  remove(id: string): Promise<void>;
  /** A new secret; the old one keeps signing for 24 hours. */
  rotate(id: string): Promise<HookCreated>;
  /** Sends a signed `ping` now. */
  test(id: string): Promise<DeliveryView>;
  deliveries(id: string): Promise<DeliveryView[]>;
  redeliver(id: string, deliveryId: string): Promise<void>;
}

const HOOKS = "/api/v1/admin/webhooks";
const hook = (id: string) => `${HOOKS}/${encodeURIComponent(id)}`;

/** The webhooks admin over a Vellum server. */
export function createHttpWebhooksApi(http: Http): WebhooksApi {
  return {
    list: () => http.get<HookList>(HOOKS),
    create: (input) => http.send<HookCreated>("POST", HOOKS, input),
    update: (id, input, enabled) =>
      http.send<HookView>("PUT", hook(id), { ...input, enabled }),
    async remove(id) {
      await http.send("DELETE", hook(id));
    },
    rotate: (id) => http.send<HookCreated>("POST", `${hook(id)}/secret`, {}),
    test: (id) => http.send<DeliveryView>("POST", `${hook(id)}/test`, {}),
    async deliveries(id) {
      const list = await http.get<DeliveryList>(`${hook(id)}/deliveries`);
      return list.deliveries;
    },
    async redeliver(id, deliveryId) {
      await http.send("POST", `${hook(id)}/redeliver`, {
        delivery_id: deliveryId,
      });
    },
  };
}
