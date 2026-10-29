-- SPDX-FileCopyrightText: 2026 The Vellum contributors
-- SPDX-License-Identifier: AGPL-3.0-only

-- Replicas of one deployment (ADR 0037). Each replica refreshes its row in
-- cluster_nodes every few seconds; a row older than the liveness window
-- is a replica that is gone. cluster_leases names work that one replica
-- does at a time (a sweep, a scheduler): the holder renews before
-- expires_ms, and anyone may take a lease once it has expired. Times are
-- milliseconds since the Unix epoch from the replicas' clocks. Keep the
-- SQLite and Postgres files in step.

CREATE TABLE cluster_nodes (
    name TEXT NOT NULL PRIMARY KEY CHECK (name <> ''),
    address TEXT NOT NULL,
    heartbeat_ms BIGINT NOT NULL
);

CREATE TABLE cluster_leases (
    name TEXT NOT NULL PRIMARY KEY CHECK (name <> ''),
    holder TEXT NOT NULL,
    expires_ms BIGINT NOT NULL
);
