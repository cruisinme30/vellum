// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/*
 * The hand-written collections types must stay in step with the server's
 * generated client: what the server returns must fit what views read, and
 * what views send must fit what the server accepts. `tsc` checks these.
 */
import { describe, expectTypeOf, it } from "vitest";

import type { Operations } from "../api/api.gen";

import type { CollectionsStore } from "./store";

type Op<K extends keyof Operations> = Operations[K];
/** Drops `readonly`, all the way down: the generated types are mutable. */
type Writable<T> = T extends readonly (infer U)[]
  ? Writable<U>[]
  : T extends object
    ? { -readonly [K in keyof T]: Writable<T[K]> }
    : T;
/**
 * What a view can read, as the server's schema states it: the server
 * declares enum-like fields (`type`) as plain strings and absent values as
 * `null`, and `Option` fields as optional, so read types are compared with
 * literals widened and nulls allowed.
 */
type Loose<T> = T extends string
  ? string
  : T extends readonly (infer U)[]
    ? Loose<U>[]
    : T extends object
      ? {
          -readonly [K in keyof T as null extends T[K] ? never : K]: Loose<T[K]> | Absent<T[K]>;
        } & {
          // `Option<T>` is optional in the schema, though the server sends `null`.
          -readonly [K in keyof T as null extends T[K] ? K : never]?: Loose<T[K]> | null;
        }
      : T;
type Absent<V> = undefined extends V ? null : never;
/** Whether every `A` is a `B`. */
type Fits<A, B> = [A] extends [B] ? true : false;
type In<M extends keyof CollectionsStore> = Parameters<CollectionsStore[M]>[0];
type Out<M extends keyof CollectionsStore> = Loose<Awaited<ReturnType<CollectionsStore[M]>>>;

describe("collections types against the generated client", () => {
  it("send what the server accepts", () => {
    expectTypeOf<
      Fits<Writable<In<"query">>, Op<"collections.query">["input"]>
    >().toEqualTypeOf<true>();
    expectTypeOf<
      Fits<Writable<In<"createRow">>, Op<"collections.create_row">["input"]>
    >().toEqualTypeOf<true>();
    expectTypeOf<
      Fits<Writable<In<"updateRow">>, Op<"collections.update_row">["input"]>
    >().toEqualTypeOf<true>();
    expectTypeOf<
      Fits<Writable<In<"deleteRow">>, Op<"collections.delete_row">["input"]>
    >().toEqualTypeOf<true>();
    expectTypeOf<
      Fits<Writable<In<"saveView">>, Op<"collections.save_view">["input"]>
    >().toEqualTypeOf<true>();
    expectTypeOf<
      Fits<Writable<In<"deleteView">>, Op<"collections.delete_view">["input"]>
    >().toEqualTypeOf<true>();
    expectTypeOf<
      Fits<Writable<In<"addProperty">>, Op<"collections.add_property">["input"]>
    >().toEqualTypeOf<true>();
    expectTypeOf<
      Fits<Writable<In<"updateProperty">>, Op<"collections.update_property">["input"]>
    >().toEqualTypeOf<true>();
    expectTypeOf<
      Fits<Writable<In<"deleteProperty">>, Op<"collections.delete_property">["input"]>
    >().toEqualTypeOf<true>();
    expectTypeOf<
      Fits<Writable<In<"exportCsv">>, Op<"collections.export_csv">["input"]>
    >().toEqualTypeOf<true>();
    expectTypeOf<
      Fits<Writable<In<"importCsv">>, Op<"collections.import_csv">["input"]>
    >().toEqualTypeOf<true>();
    expectTypeOf<
      Fits<Writable<In<"createFromCsv">>, Op<"collections.create_from_csv">["input"]>
    >().toEqualTypeOf<true>();
  });

  it("read what the server returns", () => {
    expectTypeOf<Fits<Op<"collections.list">["output"], Out<"list">>>().toEqualTypeOf<true>();
    expectTypeOf<Fits<Op<"collections.get">["output"], Out<"info">>>().toEqualTypeOf<true>();
    expectTypeOf<Fits<Op<"collections.query">["output"], Out<"query">>>().toEqualTypeOf<true>();
    expectTypeOf<Fits<Op<"collections.get_row">["output"], Out<"getRow">>>().toEqualTypeOf<true>();
    expectTypeOf<
      Fits<Op<"collections.create_row">["output"], Out<"createRow">>
    >().toEqualTypeOf<true>();
    expectTypeOf<
      Fits<Op<"collections.delete_row">["output"], Out<"deleteRow">>
    >().toEqualTypeOf<true>();
    expectTypeOf<
      Fits<Op<"collections.save_view">["output"], Out<"saveView">>
    >().toEqualTypeOf<true>();
    expectTypeOf<
      Fits<Op<"collections.add_property">["output"], Out<"addProperty">>
    >().toEqualTypeOf<true>();
    expectTypeOf<
      Fits<Op<"collections.export_csv">["output"], Out<"exportCsv">>
    >().toEqualTypeOf<true>();
    expectTypeOf<
      Fits<Op<"collections.import_csv">["output"], Out<"importCsv">>
    >().toEqualTypeOf<true>();
    expectTypeOf<
      Fits<Op<"collections.create_from_csv">["output"], Out<"createFromCsv">>
    >().toEqualTypeOf<true>();
    expectTypeOf<Fits<Op<"collections.embed">["output"], Out<"embed">>>().toEqualTypeOf<true>();
  });
});
