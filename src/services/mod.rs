//! External integrations and background domain services.
//!
//! Submodules:
//! - `apk_variant`: Detection and categorization of APK CPU architectures.
//! - `github`: GitHub REST API client for querying releases, tags, and commits.
//! - `render`: Card formatting, markdown HTML parser, and Telegram send pipeline.

pub mod apk_variant;
pub mod github;
pub mod render;
