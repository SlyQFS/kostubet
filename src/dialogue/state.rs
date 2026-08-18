//! Dialogue states and persistent payload structs.
//!
//! Encapsulates user input accumulators for APK submissions and admin moderation edits.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingApk {
    pub variant: String,
    pub file_id: String,
    pub file_unique_id: String,
    pub file_name: Option<String>,
    pub file_size: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SubmitApkData {
    pub is_new_app: bool,
    pub app_id: Option<i64>,
    pub slug: String,
    pub name: String,
    /// App-level description (what the app is). New submissions only.
    pub description: Option<String>,
    pub version: String,
    pub title: Option<String>,
    pub changelog: Option<String>,
    pub diff_url: Option<String>,
    pub cover_image_file_id: Option<String>,
    pub apk_files: Vec<PendingApk>,
    pub tags: Vec<String>,
    pub submitted_by_username: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SubmitApkState {
    ChoosingMode,
    ChoosingExistingApp,
    WaitingName,
    WaitingDescription {
        data: Box<SubmitApkData>,
    },
    WaitingVersion {
        data: Box<SubmitApkData>,
    },
    WaitingTitle {
        data: Box<SubmitApkData>,
    },
    WaitingChangelog {
        data: Box<SubmitApkData>,
    },
    WaitingDiffUrl {
        data: Box<SubmitApkData>,
    },
    WaitingCover {
        data: Box<SubmitApkData>,
    },
    WaitingApkFiles {
        data: Box<SubmitApkData>,
    },
    ResolvingVariant {
        data: Box<SubmitApkData>,
        pending_file: PendingApk,
    },
    WaitingTags {
        data: Box<SubmitApkData>,
    },
    Confirm {
        data: Box<SubmitApkData>,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EditApkData {
    pub version_id: i64,
    pub app_name: String,
    pub version: String,
    /// App-level description loaded from the app; saved via `set_app_description`.
    pub description: Option<String>,
    pub title: Option<String>,
    pub changelog: Option<String>,
    pub diff_url: Option<String>,
    pub cover_image_file_id: Option<String>,
    pub tags: Vec<String>,
    pub submitted_by_username: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EditApkState {
    EditingTitle { data: Box<EditApkData> },
    EditingDescription { data: Box<EditApkData> },
    EditingChangelog { data: Box<EditApkData> },
    EditingDiffUrl { data: Box<EditApkData> },
    EditingTags { data: Box<EditApkData> },
    ConfirmEdit { data: Box<EditApkData> },
}

/// Admin panel input flows started from inline keyboards.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AdminState {
    /// Waiting for `owner/repo` or a GitHub URL of a repository to track.
    RepoLink,
    /// Parsed repo: choosing whether to post the current release immediately
    /// or track silently (buttons `adm:trackmode:*`).
    RepoMode { owner: String, name: String },
    /// Waiting for an optional repo description (or `/skip`).
    RepoDesc {
        owner: String,
        name: String,
        silent: bool,
    },
    /// Waiting for space-separated tags for the parsed repo (or `/done`).
    RepoTags {
        owner: String,
        name: String,
        silent: bool,
        #[serde(default)]
        description: Option<String>,
    },
    /// Waiting for a new description for an existing tracked repo
    /// (text sets it, `/skip` keeps it, `/clear` removes it).
    RepoDescription { tool_id: i64 },
    /// Waiting for a new app-level description of a custom app
    /// (text sets it, `/skip` keeps it, `/clear` removes it).
    AppDescription { app_id: i64 },
    /// Waiting for a new global tag name.
    NewTag,
    /// Waiting for a tag name that is also attached to an item ("tool"/"custom_app").
    ItemTag {
        item_type: String,
        item_id: i64,
        item_label: String,
    },
}

/// Button-driven repository suggestion flow (`/suggest` and `start:suggest`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SuggestData {
    pub owner: String,
    pub name: String,
    /// Optional description proposed by the author (or `/skip`).
    pub description: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SuggestState {
    /// Waiting for the user to send a GitHub link or `owner/repo`.
    WaitingLink,
    /// Confirmation card with [✅ Отправить] [🏷 Теги] [📝 Описание] [❌ Отмена].
    Confirm { data: Box<SuggestData> },
    /// Waiting for the description text (or `/skip`).
    WaitingDescription { data: Box<SuggestData> },
    /// Waiting for space-separated tags (or `/skip`).
    WaitingTags { data: Box<SuggestData> },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub enum DialogueState {
    #[default]
    Idle,
    SubmitApk(SubmitApkState),
    EditApk(EditApkState),
    Admin(Box<AdminState>),
    Suggest(SuggestState),
}
