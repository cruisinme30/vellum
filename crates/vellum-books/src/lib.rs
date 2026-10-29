// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Books (brief §7, R-BK-1 to R-BK-10).
pub mod book;
pub mod convert;
pub mod error;
pub mod id;
pub mod manifest;
pub mod markdown;
pub mod numbering;
pub mod ops;
pub mod path;
pub mod subtree;
pub mod summary;
pub mod toc;
pub mod xref;

pub use book::{Book, Repair};
pub use error::{BookError, Result};
pub use id::{BookId, IdSource, NodeId, SequentialIds};
