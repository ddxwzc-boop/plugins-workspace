// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! Types and functions related to shell.

use serde::{Deserialize, Deserializer};

use crate::scope::OpenScope;
use std::str::FromStr;

/// Program to use on the [`open()`] call.
#[deprecated(since = "2.1.0", note = "Use tauri-plugin-opener instead.")]
pub enum Program {
    /// Use the `open` program.
    Open,
    /// Use the `start` program.
    Start,
    /// Use the `xdg-open` program.
    XdgOpen,
    /// Use the `gio` program.
    Gio,
    /// Use the `gnome-open` program.
    GnomeOpen,
    /// Use the `kde-open` program.
    KdeOpen,
    /// Use the `wslview` program.
    WslView,
    /// Use the `Firefox` program.
    Firefox,
    /// Use the `Google Chrome` program.
    Chrome,
    /// Use the `Chromium` program.
    Chromium,
    /// Use the `Safari` program.
    Safari,
}

impl FromStr for Program {
    type Err = super::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let p = match s.to_lowercase().as_str() {
            "open" => Self::Open,
            "start" => Self::Start,
            "xdg-open" => Self::XdgOpen,
            "gio" => Self::Gio,
            "gnome-open" => Self::GnomeOpen,
            "kde-open" => Self::KdeOpen,
            "wslview" => Self::WslView,
            "firefox" => Self::Firefox,
            "chrome" | "google chrome" => Self::Chrome,
            "chromium" => Self::Chromium,
            "safari" => Self::Safari,
            _ => return Err(crate::Error::UnknownProgramName(s.to_string())),
        };
        Ok(p)
    }
}

impl<'de> Deserialize<'de> for Program {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Program::from_str(&s).map_err(|e| serde::de::Error::custom(e.to_string()))
    }
}

impl Program {
    /// Maps the program to the binary name handed to the `open` crate —
    /// desktop-only dispatch, unused on OHOS (system default application).
    #[cfg(not(target_env = "ohos"))]
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Start => "start",
            Self::XdgOpen => "xdg-open",
            Self::Gio => "gio",
            Self::GnomeOpen => "gnome-open",
            Self::KdeOpen => "kde-open",
            Self::WslView => "wslview",

            #[cfg(target_os = "macos")]
            Self::Firefox => "Firefox",
            #[cfg(not(target_os = "macos"))]
            Self::Firefox => "firefox",

            #[cfg(target_os = "macos")]
            Self::Chrome => "Google Chrome",
            #[cfg(not(target_os = "macos"))]
            Self::Chrome => "google-chrome",

            #[cfg(target_os = "macos")]
            Self::Chromium => "Chromium",
            #[cfg(not(target_os = "macos"))]
            Self::Chromium => "chromium",

            #[cfg(target_os = "macos")]
            Self::Safari => "Safari",
            #[cfg(not(target_os = "macos"))]
            Self::Safari => "safari",
        }
    }
}

/// Opens path or URL with the program specified in `with`, or system default if `None`.
///
/// The path will be matched against the shell open validation regex, defaulting to `^((mailto:\w+)|(tel:\w+)|(https?://\w+)).+`.
/// A custom validation regex may be supplied in the config in `plugins > shell > scope > open`.
///
/// # Examples
///
/// ```rust,no_run
/// use tauri_plugin_shell::ShellExt;
/// tauri::Builder::default()
///   .setup(|app| {
///     // open the given URL on the system default browser
///     app.shell().open("https://github.com/tauri-apps/tauri", None)?;
///     Ok(())
///   });
/// ```
#[cfg(not(target_env = "ohos"))]
#[deprecated(since = "2.1.0", note = "Use tauri-plugin-opener instead.")]
pub fn open<P: AsRef<str>>(
    scope: Option<&OpenScope>,
    path: P,
    with: Option<Program>,
) -> crate::Result<()> {
    // validate scope if we have any (JS calls)
    if let Some(scope) = scope {
        scope.open(path.as_ref(), with).map_err(Into::into)
    } else {
        // when running directly from Rust code we don't need to validate the path
        match with.map(Program::name) {
            Some(program) => ::open::with_detached(path.as_ref(), program),
            None => ::open::that_detached(path.as_ref()),
        }
        .map_err(Into::into)
    }
}

/// OHOS variant of [`open`]: async — the path is scope-validated (when a scope
/// is given, e.g. for JS calls) and then routed through the openharmony-ability
/// URL bridge, which hands it to the system default application.
///
/// `'open with'` (the `with` program) is unsupported on OHOS and ignored —
/// system default only, mirroring `tauri-plugin-opener`'s OHOS behavior.
///
/// Absolute URLs (`https://`, `mailto:`, `tel:`, ...) are forwarded as-is;
/// everything else is treated as a filesystem path and canonicalized into an
/// absolute `file://` URI. Note the bridge itself requires a `://` scheme, so
/// `mailto:`/`tel:` URLs — allowed by the default validation regex — are
/// rejected by the bridge on OHOS.
#[cfg(target_env = "ohos")]
#[deprecated(since = "2.1.0", note = "Use tauri-plugin-opener instead.")]
pub async fn open<P: AsRef<str>>(
    scope: Option<&OpenScope>,
    path: P,
    with: Option<Program>,
) -> crate::Result<()> {
    use openharmony_ability_plugin_url::UrlExt;

    // 'open with' is unsupported on OHOS — the system default handler is used.
    if with.is_some() {
      log::debug!("shell.open: `with` program ignored on OHOS (system default handler only)");
    }
    let path = path.as_ref();
    if let Some(scope) = scope {
        scope.validate(path)?;
    }
    let uri = ohos_target_uri(path)?;
    let ohos_app = crate::ohos_app()?;
    ohos_app
        .open_url(uri)
        .await
        .map_err(|e| crate::Error::OpenharmonyAbility(e.to_string()))?;
    Ok(())
}

/// Discriminates a `shell open` argument between an absolute URL and a
/// filesystem path, producing the URI handed to the OHOS link opener.
///
/// Mirrors `tauri-plugin-opener`'s `open_url`/`open_path` split: a parseable
/// URL (any scheme, e.g. `https:`, `mailto:`, `tel:`) is forwarded as-is;
/// anything else is canonicalized into an absolute `file://` URI
/// (`Url::from_file_path` rejects relative paths).
#[cfg(target_env = "ohos")]
fn ohos_target_uri(path: &str) -> crate::Result<String> {
    match url::Url::parse(path) {
        Ok(url) => Ok(url.to_string()),
        Err(_) => {
            let canon = std::fs::canonicalize(path)?;
            url::Url::from_file_path(&canon)
                .map(|u| u.to_string())
                .map_err(|_| crate::Error::InvalidPath(path.to_string()))
        }
    }
}

#[cfg(all(test, target_env = "ohos"))]
mod tests {
    use super::ohos_target_uri;

    #[test]
    fn absolute_urls_pass_through() {
        // Url serialization normalizes a host-only https URL with a trailing slash
        assert_eq!(
            ohos_target_uri("https://tauri.app").unwrap(),
            "https://tauri.app/"
        );
        assert_eq!(
            ohos_target_uri("mailto:foo@bar.com").unwrap(),
            "mailto:foo@bar.com"
        );
        assert_eq!(ohos_target_uri("tel:12345").unwrap(), "tel:12345");
    }

    #[test]
    fn missing_relative_path_errors() {
        // no scheme and no such file relative to cwd — canonicalize fails with an io error
        assert!(ohos_target_uri("definitely-missing-file.abc").is_err());
    }

    #[test]
    fn existing_absolute_path_maps_to_file_uri() {
        // /proc/self exists on OHOS; canonicalize resolves it (through its symlink)
        let uri = ohos_target_uri("/proc/self").unwrap();
        assert!(uri.starts_with("file://"), "unexpected uri: {uri}");
    }
}
