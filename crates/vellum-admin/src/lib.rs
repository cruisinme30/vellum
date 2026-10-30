// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The audit/admin service (brief §3, §11, §12; ADR 0021).
//!
//! - [`api`]: the operations the gateway serves (`vellum_contracts::Service`);
//! - [`audit`]: the append-only, hash-chained, sealed audit log;
//! - [`audited`]: audits the admin actions other services serve;
//! - [`settings`]: typed, validated, versioned admin settings;
//! - [`counts`]: real usage counts, cached and exported as gauges;
//! - [`usage`]: opt-in anonymous telemetry with an exact preview;
//! - [`backup`]: encrypted backups, restore and the monthly restore drill;
//! - [`profile`]: the `vellum profile` command;
//! - [`budgets`]: the `vellum budgets` gate on per-route p95 budgets;
//! - [`service`]: the application service behind the admin console,
//!   including Admin → Requests, RED metrics, SLOs and profiles from
//!   `vellum-observe`.

pub mod api;
pub mod audit;
pub mod audited;
pub mod backup;
pub mod budgets;
pub mod counts;
mod fsutil;
mod http;
pub mod profile;
pub mod service;
pub mod settings;
pub mod usage;

pub use audit::{
    AuditActor, AuditLog, AuditRecord, FileAuditLog, MemoryAuditLog, NewAuditEvent, Outcome,
};
pub use service::{AdminPaths, AdminService, Caller};
pub use settings::{
    AdminSettings, AppearanceSettings, EXPORT_THEMES, FileSettingsStore, MemorySettingsStore,
    SITE_THEMES, SecretScanning, SettingsStore,
};
