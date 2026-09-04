// Copyright 2019-2024 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use crate::{error::Error, Result};

use openharmony_ability_plugin_continuation::{ContinuationClient, ContinuationExt};

/// wantParam size budget for the continuation payload: ~100 KiB platform limit
/// minus headroom for the surrounding Want fields.
const CONTINUATION_DATA_MAX_BYTES: usize = 96 * 1024;

/// Runs `f` against the global OHOS app instance held in [`tauri::ohos::APP`]
/// (same pattern as the deep-link plugin). Returns `None` when the app is not
/// initialized yet or its lock is poisoned — commands then degrade to the
/// pre-migration defaults (`false` / empty), matching the facade's documented
/// lock-poisoning semantics. The `ContinuationClient` carries the app handle
/// since issue #87 major-9 migrated the want/continuation statics into
/// `OpenHarmonyAppInner`.
fn with_continuation_client<T>(
    f: impl FnOnce(&ContinuationClient) -> T,
) -> Option<T> {
    let guard = tauri::ohos::APP.lock().ok()?;
    guard.as_ref().map(|app| f(&app.continuation()))
}

/// Returns whether the current launch is an app-continuation restore.
///
/// Peek semantics: idempotent, does not consume `get_continuation_data`.
#[tauri::command]
pub async fn is_continuation_restore() -> Result<bool> {
    // No locks are held across an await (there are no awaits).
    Ok(with_continuation_client(|client| client.is_continuation_restore())
        .unwrap_or(false))
}

/// Returns the continuation payload JSON from the source device, consuming it.
///
/// Draining take: the first call returns `Some(json)` on a continuation restore,
/// subsequent calls return `None`. `None` also means the launch was not a
/// continuation restore. The payload is passed through verbatim.
#[tauri::command]
pub async fn get_continuation_data() -> Result<Option<String>> {
    let data = with_continuation_client(|client| client.take_continuation_data())
        .unwrap_or_default();
    // Empty string means "no continuation data" — normalize to null for JS.
    Ok((!data.is_empty()).then_some(data))
}

/// Pre-registers the source-side continuation snapshot (overwrite semantics).
///
/// The ArkTS `onContinue` callback reads the snapshot synchronously when the
/// system initiates a migration and forwards it as `wantParam.continuationData`;
/// an empty string clears the snapshot (`onContinue` then refuses with MISMATCH).
/// Reading is peek-only — a cancelled migration leaves the snapshot for a retry.
///
/// No app handle needed: the snapshot is process-level state (the NAPI reader
/// has no receiver), so this is a direct free-function call.
#[tauri::command]
pub async fn set_continuation_data(data: String) -> Result<()> {
    if data.len() > CONTINUATION_DATA_MAX_BYTES {
        return Err(Error::PayloadTooLarge);
    }
    openharmony_ability_plugin_continuation::set_continuation_data(data);
    Ok(())
}
