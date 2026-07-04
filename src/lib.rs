//! Everpublich core library.
//!
//! The Lambda binaries are thin adapters around this crate. Keeping the
//! publishing, token handling, widget expansion, and Zola generation here makes
//! the risky behavior testable without AWS credentials.

#![warn(missing_docs)]

pub mod admin;
pub mod auth;
pub mod crypto;
pub mod enml;
pub mod evernote;
pub mod github;
pub mod lambda_app;
pub mod models;
pub mod slug;
pub mod store;
pub mod widgets;
pub mod zola;
