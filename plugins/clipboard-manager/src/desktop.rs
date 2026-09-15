// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use serde::de::DeserializeOwned;
use std::borrow::Cow;
use tauri::{image::Image, plugin::PluginApi, AppHandle, Runtime};

#[cfg(all(desktop, not(target_env = "ohos")))]
use std::sync::Mutex;

// ─── Desktop (non-OHOS): arboard-based clipboard ───

#[cfg(all(desktop, not(target_env = "ohos")))]
use arboard::ImageData;

#[cfg(all(desktop, not(target_env = "ohos")))]
pub fn init<R: Runtime, C: DeserializeOwned>(
    app: &AppHandle<R>,
    _api: PluginApi<R, C>,
) -> crate::Result<Clipboard<R>> {
    Ok(Clipboard {
        app: app.clone(),
        clipboard: arboard::Clipboard::new().map(|c| Mutex::new(Some(c))),
    })
}

#[cfg(all(desktop, not(target_env = "ohos")))]
/// Access to the clipboard APIs.
pub struct Clipboard<R: Runtime> {
    #[allow(dead_code)]
    app: AppHandle<R>,
    // According to arboard docs the clipboard must be dropped before exit.
    // Since tauri doesn't call drop on exit we'll use an Option to take() on RunEvent::Exit.
    clipboard: Result<Mutex<Option<arboard::Clipboard>>, arboard::Error>,
}

#[cfg(all(desktop, not(target_env = "ohos")))]
impl<R: Runtime> Clipboard<R> {
    pub fn write_text<'a, T: Into<Cow<'a, str>>>(&self, text: T) -> crate::Result<()> {
        match &self.clipboard {
            Ok(clipboard) => clipboard
                .lock()
                .unwrap()
                .as_mut()
                .unwrap()
                .set_text(text)
                .map_err(Into::into),
            Err(e) => Err(crate::Error::Clipboard(e.to_string())),
        }
    }

    pub fn write_image(&self, image: &Image<'_>) -> crate::Result<()> {
        match &self.clipboard {
            Ok(clipboard) => clipboard
                .lock()
                .unwrap()
                .as_mut()
                .unwrap()
                .set_image(ImageData {
                    bytes: Cow::Borrowed(image.rgba()),
                    width: image.width() as usize,
                    height: image.height() as usize,
                })
                .map_err(Into::into),
            Err(e) => Err(crate::Error::Clipboard(e.to_string())),
        }
    }

    /// Warning: This method should not be used on the main thread! Otherwise the underlying libraries may deadlock on Linux, freezing the whole app, when trying to copy data copied from this app, for example if the user copies text from the WebView.
    pub fn read_text(&self) -> crate::Result<String> {
        match &self.clipboard {
            Ok(clipboard) => {
                let text = clipboard.lock().unwrap().as_mut().unwrap().get_text()?;
                Ok(text)
            }
            Err(e) => Err(crate::Error::Clipboard(e.to_string())),
        }
    }

    pub fn write_html<'a, T: Into<Cow<'a, str>>>(
        &self,
        html: T,
        alt_text: Option<T>,
    ) -> crate::Result<()> {
        match &self.clipboard {
            Ok(clipboard) => clipboard
                .lock()
                .unwrap()
                .as_mut()
                .unwrap()
                .set_html(html, alt_text)
                .map_err(Into::into),
            Err(e) => Err(crate::Error::Clipboard(e.to_string())),
        }
    }

    pub fn clear(&self) -> crate::Result<()> {
        match &self.clipboard {
            Ok(clipboard) => clipboard
                .lock()
                .unwrap()
                .as_mut()
                .unwrap()
                .clear()
                .map_err(Into::into),
            Err(e) => Err(crate::Error::Clipboard(e.to_string())),
        }
    }

    /// Warning: This method should not be used on the main thread! Otherwise the underlying libraries may deadlock on Linux, freezing the whole app, when trying to copy data copied from this app, for example if the user copies text from the WebView.
    pub fn read_image(&self) -> crate::Result<Image<'_>> {
        match &self.clipboard {
            Ok(clipboard) => {
                let image = clipboard.lock().unwrap().as_mut().unwrap().get_image()?;
                let image = Image::new_owned(
                    image.bytes.to_vec(),
                    image.width as u32,
                    image.height as u32,
                );
                Ok(image)
            }
            Err(e) => Err(crate::Error::Clipboard(e.to_string())),
        }
    }

    pub(crate) fn cleanup(&self) {
        if let Ok(clipboard) = &self.clipboard {
            clipboard.lock().unwrap().take();
        }
    }
}

// ─── OHOS: no arboard; write_image via TSFN bridge, other methods unsupported ───

/// Runs `f` against the global OHOS app instance held in [`tauri::ohos::APP`]
/// (same pattern as the deep-link/global-shortcut plugins). Returns `None`
/// when the app is not initialized or its lock is poisoned.
#[cfg(target_env = "ohos")]
fn with_ohos_app<R>(
    f: impl FnOnce(&tauri::ohos::openharmony_ability::OpenHarmonyApp) -> R,
) -> Option<R> {
    let guard = tauri::ohos::APP.lock().ok()?;
    guard.as_ref().map(f)
}

/// Acquires the clipboard bridge client from the global OHOS app instance.
#[cfg(target_env = "ohos")]
fn clipboard_client() -> crate::Result<openharmony_ability_plugin_clipboard::ClipboardClient> {
    use openharmony_ability_plugin_clipboard::ClipboardExt;

    with_ohos_app(|app| app.clipboard().ok())
        .flatten()
        .ok_or_else(|| {
            crate::Error::Clipboard(
                "Failed to create ClipboardClient: OHOS APP not initialized".to_string(),
            )
        })
}

#[cfg(target_env = "ohos")]
pub fn init<R: Runtime, C: DeserializeOwned>(
    app: &AppHandle<R>,
    _api: PluginApi<R, C>,
) -> crate::Result<Clipboard<R>> {
    // Register the Rust-side Clipboard bridge plugin so ArkTS configurePlugins
    // can match it. Without this, bridge calls fail with
    // "Bridge plugin 'ohos.clipboard' is not installed for '<module>'".
    use openharmony_ability_plugin_clipboard::ClipboardBridgePlugin;
    if let Some(result) = with_ohos_app(|ohos_app| ohos_app.register_plugin(ClipboardBridgePlugin))
    {
        if let Err(e) = result {
            log::error!(
                "[clipboard-manager] failed to register ClipboardBridgePlugin: {}",
                e
            );
        }
    }
    Ok(Clipboard { app: app.clone() })
}

#[cfg(target_env = "ohos")]
/// Access to the clipboard APIs.
///
/// On OHOS, `write_text`, `read_text`, `write_image`, `read_image`,
/// `write_html`, and `clear` are all supported via the bridge plugin facade
/// (`openharmony-ability-plugin-clipboard`). Reads are subject to the
/// API 12+ READ_PASTEBOARD gate (see ClipboardPlugin.ets).
pub struct Clipboard<R: Runtime> {
    #[allow(dead_code)]
    app: AppHandle<R>,
}

#[cfg(target_env = "ohos")]
impl<R: Runtime> Clipboard<R> {
    // write_text on OHOS: bridge plugin via openharmony-ability-plugin-clipboard.
    pub async fn write_text<'a, T: Into<Cow<'a, str>>>(&self, text: T) -> crate::Result<()> {
        let client = clipboard_client()?;
        let text = text.into().to_string();
        client
            .write_text(text)
            .await
            .map_err(|e| crate::Error::Clipboard(e.to_string()))
    }

    // read_text on OHOS: bridge plugin via openharmony-ability-plugin-clipboard.
    pub async fn read_text(&self) -> crate::Result<String> {
        let client = clipboard_client()?;
        client
            .read_text()
            .await
            .map(|opt| opt.unwrap_or_default())
            .map_err(|e| crate::Error::Clipboard(e.to_string()))
    }

    // write_image on OHOS: bridge plugin facade via openharmony-ability-plugin-clipboard.
    // RGBA data is extracted in commands.rs before the .await boundary
    // (the ResourceTable MutexGuard is !Send and cannot cross .await).
    pub async fn write_image(&self, rgba: &[u8], width: u32, height: u32) -> crate::Result<()> {
        let client = clipboard_client()?;
        client
            .write_image(rgba, width, height)
            .await
            .map_err(|e| crate::Error::Clipboard(e.to_string()))?;
        Ok(())
    }

    // write_html on OHOS: bridge plugin facade via openharmony-ability-plugin-clipboard.
    pub async fn write_html<'a, T: Into<Cow<'a, str>>>(
        &self,
        html: T,
        _alt_text: Option<T>,
    ) -> crate::Result<()> {
        let client = clipboard_client()?;
        client
            .write_html(html.into().to_string())
            .await
            .map_err(|e| crate::Error::Clipboard(e.to_string()))?;
        Ok(())
    }

    // clear on OHOS: bridge plugin facade via openharmony-ability-plugin-clipboard.
    pub async fn clear(&self) -> crate::Result<()> {
        let client = clipboard_client()?;
        client
            .clear()
            .await
            .map_err(|e| crate::Error::Clipboard(e.to_string()))?;
        Ok(())
    }

    // read_image on OHOS: bridge read-image action — the ArkTS side packs the
    // pasteboard PixelMap as a base64 PNG, the plugin facade decodes it to
    // RGBA. Async like write_image (commands.rs drives both from a worker
    // thread); returns an owned Image (no arboard borrow on this path).
    pub async fn read_image(&self) -> crate::Result<Image<'static>> {
        let client = clipboard_client()?;
        let image = client
            .read_image()
            .await
            .map_err(|e| crate::Error::Clipboard(e.to_string()))?;
        Ok(Image::new_owned(image.rgba, image.width, image.height))
    }

    pub(crate) fn cleanup(&self) {
        // No arboard clipboard to clean up on OHOS
    }
}
