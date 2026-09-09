//! These mutations deliberately use the core writer service, not the desktop's
//! ordinary read-only query connection. No caller-supplied database paths.
#[cfg(test)]
mod tests;
use sessionatlas_core::store::{
    change_content_index_at, content_index_status_at, ContentIndexAction, ContentIndexStatus,
};

#[tauri::command]
pub async fn get_content_index_status() -> Result<ContentIndexStatus, String> {
    let path = super::db_path();
    tauri::async_runtime::spawn_blocking(move || {
        content_index_status_at(path).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn update_content_index(
    action: ContentIndexAction,
) -> Result<ContentIndexStatus, String> {
    let path = super::db_path();
    tauri::async_runtime::spawn_blocking(move || {
        change_content_index_at(path, action).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
