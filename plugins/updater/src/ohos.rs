// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! OpenHarmony-specific implementation for the updater plugin.
//!
//! On OHOS, updates are hosted by AppGallery — the desktop updater API
//! (endpoints/pubkey from `tauri.conf.json`) does not apply. Instead of
//! faking the desktop commands, this module exposes two OHOS-only commands:
//! - `check_app_gallery_update` queries `updateManager.checkAppUpdate()`
//!   (pure metadata, no dialog)
//! - `show_app_gallery_update_dialog` triggers `updateManager.showUpdateDialog()`
//!   (system dialog driving the full download + install flow)

use crate::{Error, Result};
use openharmony_ability::Updater;
use serde::Serialize;
use tauri::{Manager, Runtime, Webview};

// ── Types ────────────────────────────────────────────────────────────

/// Result of an AppGallery update check, serialized as
/// `{ available, currentVersion, version }`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppGalleryUpdateInfo {
    available: bool,
    current_version: String,
    version: Option<String>,
}

// ── Helpers ──────────────────────────────────────────────────────────

/// Get the AppGallery updater bridge handle.
///
/// The `tauri::ohos::APP` guard is acquired and dropped inside this helper,
/// so the lock is never held across an await.
fn updater_bridge() -> Result<Updater> {
    let guard = tauri::ohos::APP
        .lock()
        .map_err(|_| Error::Network("OHOS APP mutex poisoned".into()))?;
    let app = guard
        .as_ref()
        .ok_or_else(|| Error::Network("OHOS APP not initialized".into()))?;
    app.updater()
        .map_err(|e| Error::Network(e.reason.to_string()))
}

// ── Commands ─────────────────────────────────────────────────────────

/// Check for updates via AppGallery (OHOS only).
///
/// This is a pure query — no dialog is shown. It takes no options: AppGallery
/// is the update source, so the desktop `check` arguments
/// (headers/timeout/proxy/target/allowDowngrades) do not apply.
#[tauri::command]
pub(crate) async fn check_app_gallery_update<R: Runtime>(
    webview: Webview<R>,
) -> Result<AppGalleryUpdateInfo> {
    let updater = updater_bridge()?;

    match updater.check().await {
        Ok(None) => {
            // No update available: AppGallery reports nothing, so fall back to
            // the bundled version from `Manager::package_info()`.
            Ok(AppGalleryUpdateInfo {
                available: false,
                current_version: webview.package_info().version.to_string(),
                version: None,
            })
        }
        Ok(Some(r)) => Ok(AppGalleryUpdateInfo {
            available: true,
            current_version: r.current_version,
            // The SDK 12 ArkTS fallback is the literal "unknown" (on API < 20
            // the new version number is not available); map that and the empty
            // string to `None` so JS can rely on `version == null`. Since
            // openharmony-ability#51 the bridge itself returns `Option<String>`
            // (null on API < 20), which is passed through here.
            version: r.version.filter(|v| !v.is_empty() && v != "unknown"),
        }),
        Err(e) => Err(Error::Network(e.reason.to_string())),
    }
}

/// Show the system AppGallery update dialog (OHOS only).
///
/// The dialog is user-driven: whether to download and install the update is
/// decided by the user, so resolving `Ok(())` does NOT mean the update was
/// installed.
#[tauri::command]
pub(crate) async fn show_app_gallery_update_dialog<R: Runtime>(
    _webview: Webview<R>,
) -> Result<()> {
    updater_bridge()?
        .download_and_install()
        .await
        .map_err(|e| Error::Network(e.reason.to_string()))?;

    Ok(())
}
