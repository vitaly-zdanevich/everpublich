//! Everpublich core library.
//!
//! The deploy adapters are thin wrappers around this crate. Keeping the
//! publishing, token handling, widget expansion, and Zola generation here makes
//! the risky behavior testable without cloud credentials.

#![warn(missing_docs)]

pub mod admin;
pub mod auth;
pub mod crypto;
pub mod enml;
pub mod evernote;
pub mod evernote_api;
pub mod evernote_cache;
pub mod github;
pub mod models;
mod site_output;
pub mod slug;
pub mod store;
pub mod widgets;
pub mod zola;
