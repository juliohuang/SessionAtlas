//! SessionAtlas scanning core — shared by the `sessionatlas` CLI and the
//! Tauri console.
//!
//! Contract: this crate MUST NOT depend on Tauri or on CLI presentation
//! libraries. Scanners, indexer, store, config, path semantics, process
//! security and launcher logic live here so they stay reusable and testable.
//!
//! Module layout (each module is owned by exactly one migration task and is
//! predeclared here so later tasks never need to edit this file):
//!
//! | module    | owned by |
//! |-----------|----------|
//! | model     | R02      |
//! | path      | R02      |
//! | config    | R03      |
//! | scanner   | R04/R05  |
//! | indexer   | R06      |
//! | store     | R07      |
//! | process   | R10      |
//! | security  | R10      |
//! | launcher  | R10      |

pub mod config;
pub mod indexer;
pub mod launcher;
pub mod model;
pub mod path;
pub mod process;
pub mod scanner;
pub mod security;
pub mod store;
