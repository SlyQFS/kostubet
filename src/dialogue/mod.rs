//! Finite State Machine (FSM) dialogues for multi-step interactive flows.
//!
//! Submodules:
//! - `state`: Data structs and state enum definitions.
//! - `storage`: SQLite-backed persistent state serializer for dialogue persistence across restarts.
//! - `submit_apk`: Interactive wizard for user APK application creation and version submission.
//! - `edit_apk`: Admin dialogue for editing changelogs, titles, and metadata of pending APK versions.
//! - `admin`: Admin panel input flows (repo add, tag add).

pub mod admin;
pub mod edit_apk;
pub mod state;
pub mod storage;
pub mod submit_apk;
pub mod suggest;

use teloxide::dispatching::dialogue::Dialogue;

pub use admin::handle_admin_message;
pub use edit_apk::handle_edit_message;
pub use state::{AdminState, DialogueState};
pub use storage::SqliteDialogueStorage;
pub use submit_apk::handle_submit_message;
pub use suggest::{handle_suggest_message, send_suggest_confirm, validate_new_suggestion};

/// The single dialogue handle type used across handlers (defined once).
pub type BotDialogue = Dialogue<DialogueState, SqliteDialogueStorage>;
