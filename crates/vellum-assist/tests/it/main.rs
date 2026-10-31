// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The integration tests of `vellum-assist`, one module per area.
//!
//! They build as one test program instead of one per file: every
//! program links the whole dependency graph again, and about 165
//! of them overflowed the coverage runner's disk (ADR 0076, D324).
//! Add new tests as a module here, not as a file in `tests/`.

mod service;
