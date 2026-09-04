// Copyright 2019-2024 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! OpenHarmony implementation for the huawei-account plugin.
//!
//! Routes to `openharmony_ability::HuaweiAccount` via the typed bridge plugin
//! (`ohos.account` AsyncPluginBase, ArkTS `AccountPlugin.ets`) using plain
//! `#[tauri::command]` + `generate_handler!`, bypassing the mobile plugin command
//! pipe (`run_mobile_plugin` / `dispatch_run_command` / `PENDING_PLUGIN_CALLS`).
//!
//! DESIGN DECISION (Phase 4 — p4-decoupling):
//! Huawei account login is currently a Tauri plugin-specific need, not a
//! general OHOS platform capability. Unlike window/clipboard/menu which have
//! been migrated to the typed bridge plugin facade pattern (WindowClient,
//! ClipboardClient, MenuClient), huawei-account retains its direct bridge
//! facade (`HuaweiAccount`) in `openharmony-ability`. This is classified as a
//! "core privilege" — if future OHOS apps need Huawei account as a generic
//! capability, it can be extracted into a `plugin-account` bridge crate
//! following the same pattern. For now, no change is needed.

use crate::{error::Error, models::AccountInfo};

type Result<T> = std::result::Result<T, Error>;

/// Builds a `HuaweiAccount` bound to the process-wide OHOS app.
///
/// The `APP` mutex guard is fully dropped before this returns (the client holds
/// its own `BridgeRuntime` clone), so callers may `.await` freely — the `!Send`
/// guard never crosses an await point. Same shape as the accessibility and
/// screenshot plugins' `client()` helpers.
fn account() -> Result<openharmony_ability::HuaweiAccount> {
    let guard = tauri::ohos::APP
        .lock()
        .map_err(|_| Error::from_napi_reason("OHOS APP mutex poisoned"))?;
    let app = guard
        .as_ref()
        .ok_or_else(|| Error::from_napi_reason("OHOS APP not initialized"))?;
    openharmony_ability::HuaweiAccount::new(app)
        .map_err(|e| Error::from_napi_reason(&e.reason))
}

/// Interactive login — forces the Huawei account login UI.
#[tauri::command]
pub async fn login() -> Result<AccountInfo> {
    let account = account()?;

    let info = account
        .login()
        .await
        .map_err(|e| Error::from_napi_reason(&e.reason))?;
    Ok(AccountInfo::from(info))
}

/// Silent login — no UI; succeeds only when already logged in & authorized.
#[tauri::command]
pub async fn silent_login() -> Result<AccountInfo> {
    let account = account()?;

    let info = account
        .silent_login()
        .await
        .map_err(|e| Error::from_napi_reason(&e.reason))?;
    Ok(AccountInfo::from(info))
}

/// Logout — cancels the app's Huawei account authorization (p1 design D8).
#[tauri::command]
pub async fn logout() -> Result<()> {
    let account = account()?;

    account
        .logout()
        .await
        .map_err(|e| Error::from_napi_reason(&e.reason))?;
    Ok(())
}

