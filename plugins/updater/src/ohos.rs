// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! OpenHarmony-specific implementation for the updater plugin.
//!
//! On OHOS, updates are handled via AppGallery and exposed through two
//! OHOS-specific commands (the desktop `check`/`download`/`install`/
//! `download_and_install` commands are NOT registered here — OHOS aligns
//! with the upstream android/ios stance of not implementing them):
//! - `check_app_gallery_update`     → `updateManager.checkAppUpdate()` —
//!   pure metadata query, no dialog
//! - `show_app_gallery_update_dialog` → `updateManager.showUpdateDialog()` —
//!   system-owned dialog that drives the full download + install flow

use crate::{Error, Result};
use serde::Serialize;
use tauri::{Runtime, Webview};

/// AppGallery update info (OHOS only).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppGalleryUpdate {
  /// Always `true` when this struct is returned (a `null` result means no
  /// update is available). Kept for API-explicitness.
  available: bool,
  current_version: String,
  /// `None` on API levels below 20 (`versionName` unavailable).
  version: Option<String>,
}

/// Obtain the AppGallery updater bridge client from the global OHOS app.
async fn updater_client() -> Result<openharmony_ability::Updater> {
  let updater = {
    let guard = tauri::ohos::APP
      .lock()
      .map_err(|_| Error::Network("OHOS APP mutex poisoned".into()))?;
    let app = guard
      .as_ref()
      .ok_or_else(|| Error::Network("OHOS APP not initialized".into()))?;
    app
      .updater()
      .map_err(|e| Error::Network(e.reason.to_string()))?
  }; // guard dropped before await
  Ok(updater)
}

/// Check for updates via AppGallery (OHOS).
///
/// This is a pure query — no dialog is shown.
/// Returns `None` if no update is available, or `Some(AppGalleryUpdate)`
/// if an update exists.
#[tauri::command]
pub(crate) async fn check_app_gallery_update<R: Runtime>(
  _webview: Webview<R>,
) -> Result<Option<AppGalleryUpdate>> {
  let updater = updater_client().await?;

  let result = updater
    .check()
    .await
    .map_err(|e| Error::Network(e.reason.to_string()))?;

  Ok(result.map(|r| AppGalleryUpdate {
    available: true,
    current_version: r.current_version,
    version: r.version,
  }))
}

/// Show the system AppGallery update dialog (OHOS).
///
/// This triggers the system-owned `showUpdateDialog()` which handles the
/// entire download and installation flow. Whether to update is the user's
/// decision; the promise resolves once the dialog request is dispatched.
#[tauri::command]
pub(crate) async fn show_app_gallery_update_dialog<R: Runtime>(
  _webview: Webview<R>,
) -> Result<()> {
  let updater = updater_client().await?;

  updater
    .download_and_install()
    .await
    .map_err(|e| Error::Network(e.reason.to_string()))?;

  Ok(())
}
