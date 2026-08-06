use crate::deezer::download;
use crate::deezer::models::{AlbumResult, ArtistResult, DownloadResult, FileTagData, PlaylistResult, SearchResult, WriteTagData};
use crate::deezer::DeezerClient;
use crate::settings::{ArlStorageStatus, Settings};
use crate::themes as theme_store;
use crate::tray as app_tray;
use crate::AppState;
use serde_json::Value;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;
use url::Url;

mod account;
mod catalog;
mod downloads;
mod filesystem;
mod history;
mod settings;
mod tag_editor;
mod themes;
mod tray;

pub use account::*;
pub use catalog::*;
pub use downloads::*;
pub use filesystem::*;
pub use history::*;
pub use settings::*;
pub use tag_editor::*;
pub use themes::*;
pub use tray::*;

async fn run_blocking<T, F>(operation: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|e| format!("Blocking task failed: {}", e))?
}
