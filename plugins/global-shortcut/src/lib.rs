// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! Register global shortcuts.
//!
//! - Supported platforms: Windows, Linux, macOS and OpenHarmony.

#![doc(
    html_logo_url = "https://github.com/tauri-apps/tauri/raw/dev/app-icon.png",
    html_favicon_url = "https://github.com/tauri-apps/tauri/raw/dev/app-icon.png"
)]
#![cfg(not(any(target_os = "android", target_os = "ios")))]

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use serde::Serialize;
use tauri::{
    ipc::Channel,
    plugin::{Builder as PluginBuilder, TauriPlugin},
    AppHandle, Manager, Runtime, State,
};

mod error;

pub use error::Error;
type Result<T> = std::result::Result<T, Error>;

type HotKeyId = u32;

// ─── Platform-specific type aliases ─────────────────────────────────────────

#[cfg(not(target_env = "ohos"))]
pub use global_hotkey::{
    hotkey::{Code, HotKey as Shortcut, Modifiers},
    GlobalHotKeyEvent as ShortcutEvent, HotKeyState as ShortcutState,
};

#[cfg(not(target_env = "ohos"))]
use global_hotkey::GlobalHotKeyEvent;

#[cfg(not(target_env = "ohos"))]
use std::str::FromStr;

// ─── OpenHarmony backend ────────────────────────────────────────────────────

#[cfg(target_env = "ohos")]
mod ohos;

#[cfg(target_env = "ohos")]
pub use ohos::{Code, Modifiers, Shortcut, ShortcutEvent, ShortcutState};

#[cfg(target_env = "ohos")]
use ohos::{lock_shortcuts, ohos_setup};

// ─── Common types ───────────────────────────────────────────────────────────

type HandlerFn<R> = Box<dyn Fn(&AppHandle<R>, &Shortcut, ShortcutEvent) + Send + Sync + 'static>;

pub struct ShortcutWrapper(Shortcut);

impl From<Shortcut> for ShortcutWrapper {
    fn from(value: Shortcut) -> Self {
        Self(value)
    }
}

#[cfg(not(target_env = "ohos"))]
impl TryFrom<&str> for ShortcutWrapper {
    type Error = global_hotkey::hotkey::HotKeyParseError;
    fn try_from(value: &str) -> std::result::Result<Self, Self::Error> {
        Shortcut::from_str(value).map(ShortcutWrapper)
    }
}

#[cfg(target_env = "ohos")]
impl TryFrom<&str> for ShortcutWrapper {
    type Error = ohos::HotKeyParseError;
    fn try_from(value: &str) -> std::result::Result<Self, Self::Error> {
        value
            .parse::<Shortcut>()
            .map(ShortcutWrapper)
            .map_err(|e| ohos::HotKeyParseError(e))
    }
}

struct RegisteredShortcut<R: Runtime> {
    shortcut: Shortcut,
    handler: Option<Arc<HandlerFn<R>>>,
}

// ─── Platform-specific GlobalHotKeyManager ──────────────────────────────────

#[cfg(not(target_env = "ohos"))]
struct GlobalHotKeyManager(global_hotkey::GlobalHotKeyManager);

#[cfg(not(target_env = "ohos"))]
/// SAFETY: we ensure it is run on main thread only
unsafe impl Send for GlobalHotKeyManager {}
#[cfg(not(target_env = "ohos"))]
/// SAFETY: we ensure it is run on main thread only
unsafe impl Sync for GlobalHotKeyManager {}

pub struct GlobalShortcut<R: Runtime> {
    #[cfg(not(target_env = "ohos"))]
    app: AppHandle<R>,
    #[cfg(not(target_env = "ohos"))]
    manager: Arc<GlobalHotKeyManager>,
    #[cfg(target_env = "ohos")]
    client: Option<openharmony_ability_plugin_global_shortcut::GlobalShortcutClient>,
    shortcuts: Arc<Mutex<HashMap<HotKeyId, RegisteredShortcut<R>>>>,
}

#[cfg(not(target_env = "ohos"))]
macro_rules! run_main_thread {
    ($handle:expr, $manager:expr, |$m:ident| $ex:expr) => {{
        let (tx, rx) = std::sync::mpsc::channel();
        let manager = $manager.clone();
        let task = move || {
            let f = |$m: &GlobalHotKeyManager| $ex;
            let _ = tx.send(f(&*manager));
        };
        $handle.run_on_main_thread(task)?;
        rx.recv()?
    }};
}

#[cfg(not(target_env = "ohos"))]
fn lock_shortcuts<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap()
}

impl<R: Runtime> GlobalShortcut<R> {
    fn register_internal<F: Fn(&AppHandle<R>, &Shortcut, ShortcutEvent) + Send + Sync + 'static>(
        &self,
        shortcut: Shortcut,
        handler: Option<F>,
    ) -> Result<()> {
        let id = shortcut.id();
        let handler = handler.map(|h| Arc::new(Box::new(h) as HandlerFn<R>));

        #[cfg(not(target_env = "ohos"))]
        {
            run_main_thread!(self.app, self.manager, |m| m.0.register(shortcut.clone()))?;
        }

        #[cfg(target_env = "ohos")]
        {
            let client = self.client.as_ref().ok_or_else(|| {
                Error::GlobalHotkey("GlobalShortcutClient not initialized".to_string())
            })?;
            ohos::spawn_register(
                client.clone(),
                id,
                shortcut.modifiers(),
                shortcut.code().to_ohos_name(),
            );
        }

        lock_shortcuts(&self.shortcuts).insert(id, RegisteredShortcut { shortcut, handler });
        Ok(())
    }

    fn register_multiple_internal<S, F>(&self, shortcuts: S, handler: Option<F>) -> Result<()>
    where
        S: IntoIterator<Item = Shortcut>,
        F: Fn(&AppHandle<R>, &Shortcut, ShortcutEvent) + Send + Sync + 'static,
    {
        let handler = handler.map(|h| Arc::new(Box::new(h) as HandlerFn<R>));

        let hotkeys = shortcuts.into_iter().collect::<Vec<_>>();

        let mut shortcuts = lock_shortcuts(&self.shortcuts);
        for shortcut in hotkeys {
            #[cfg(not(target_env = "ohos"))]
            {
                run_main_thread!(self.app, self.manager, |m| m.0.register(shortcut.clone()))?;
            }

            #[cfg(target_env = "ohos")]
            {
                if let Some(ref client) = self.client {
                    ohos::spawn_register(
                        client.clone(),
                        shortcut.id(),
                        shortcut.modifiers(),
                        shortcut.code().to_ohos_name(),
                    );
                }
            }

            shortcuts.insert(
                shortcut.id(),
                RegisteredShortcut {
                    shortcut,
                    handler: handler.clone(),
                },
            );
        }

        Ok(())
    }
}

impl<R: Runtime> GlobalShortcut<R> {
    /// Register a shortcut.
    pub fn register<S>(&self, shortcut: S) -> Result<()>
    where
        S: TryInto<ShortcutWrapper>,
        S::Error: std::error::Error,
    {
        self.register_internal(
            try_into_shortcut(shortcut)?,
            None::<fn(&AppHandle<R>, &Shortcut, ShortcutEvent)>,
        )
    }

    /// Register a shortcut with a handler.
    pub fn on_shortcut<S, F>(&self, shortcut: S, handler: F) -> Result<()>
    where
        S: TryInto<ShortcutWrapper>,
        S::Error: std::error::Error,
        F: Fn(&AppHandle<R>, &Shortcut, ShortcutEvent) + Send + Sync + 'static,
    {
        self.register_internal(try_into_shortcut(shortcut)?, Some(handler))
    }

    /// Register multiple shortcuts.
    pub fn register_multiple<S, T>(&self, shortcuts: S) -> Result<()>
    where
        S: IntoIterator<Item = T>,
        T: TryInto<ShortcutWrapper>,
        T::Error: std::error::Error,
    {
        let mut s = Vec::new();
        for shortcut in shortcuts {
            s.push(try_into_shortcut(shortcut)?);
        }
        self.register_multiple_internal(s, None::<fn(&AppHandle<R>, &Shortcut, ShortcutEvent)>)
    }

    /// Register multiple shortcuts with a handler.
    pub fn on_shortcuts<S, T, F>(&self, shortcuts: S, handler: F) -> Result<()>
    where
        S: IntoIterator<Item = T>,
        T: TryInto<ShortcutWrapper>,
        T::Error: std::error::Error,
        F: Fn(&AppHandle<R>, &Shortcut, ShortcutEvent) + Send + Sync + 'static,
    {
        let mut s = Vec::new();
        for shortcut in shortcuts {
            s.push(try_into_shortcut(shortcut)?);
        }
        self.register_multiple_internal(s, Some(handler))
    }

    /// Unregister a shortcut
    pub fn unregister<S: TryInto<ShortcutWrapper>>(&self, shortcut: S) -> Result<()>
    where
        S::Error: std::error::Error,
    {
        let shortcut = try_into_shortcut(shortcut)?;

        #[cfg(not(target_env = "ohos"))]
        {
            run_main_thread!(self.app, self.manager, |m| m.0.unregister(shortcut.clone()))?;
        }

        #[cfg(target_env = "ohos")]
        {
            if let Some(ref client) = self.client {
                ohos::spawn_unregister(client.clone(), shortcut.id());
            }
        }

        lock_shortcuts(&self.shortcuts).remove(&shortcut.id());
        Ok(())
    }

    /// Unregister multiple shortcuts.
    pub fn unregister_multiple<T: TryInto<ShortcutWrapper>, S: IntoIterator<Item = T>>(
        &self,
        shortcuts: S,
    ) -> Result<()>
    where
        T::Error: std::error::Error,
    {
        let mut mapped_shortcuts = Vec::new();
        for shortcut in shortcuts {
            mapped_shortcuts.push(try_into_shortcut(shortcut)?);
        }

        #[cfg(not(target_env = "ohos"))]
        {
            let mapped_clone = mapped_shortcuts.clone();
            #[rustfmt::skip]
            run_main_thread!(self.app, self.manager, |m| m.0.unregister_all(&mapped_clone))?;
        }

        #[cfg(target_env = "ohos")]
        {
            if let Some(ref client) = self.client {
                for s in &mapped_shortcuts {
                    ohos::spawn_unregister(client.clone(), s.id());
                }
            }
        }

        let mut shortcuts = lock_shortcuts(&self.shortcuts);
        for s in mapped_shortcuts {
            shortcuts.remove(&s.id());
        }

        Ok(())
    }

    /// Unregister all registered shortcuts.
    pub fn unregister_all(&self) -> Result<()> {
        let mut shortcuts = lock_shortcuts(&self.shortcuts);
        let hotkeys = std::mem::take(&mut *shortcuts);

        #[cfg(not(target_env = "ohos"))]
        {
            let hotkey_vec = hotkeys.values().map(|s| s.shortcut.clone()).collect::<Vec<_>>();
            #[rustfmt::skip]
            let res = run_main_thread!(self.app, self.manager, |m| m.0.unregister_all(hotkey_vec.as_slice()));
            res.map_err(Into::into)
        }

        #[cfg(target_env = "ohos")]
        {
            let _ = &hotkeys; // suppress unused warning on OHOS
            if let Some(ref client) = self.client {
                ohos::spawn_unregister_all(client.clone());
            }
            Ok(())
        }
    }

    /// Determines whether the given shortcut is registered by this application or not.
    ///
    /// If the shortcut is registered by another application, it will still return `false`.
    ///
    /// # OHOS note
    /// Registration is fire-and-forget: `register()` returns immediately after
    /// queueing the request, while the actual `inputConsumer.on()` call happens
    /// asynchronously on the ArkTS main thread. This function queries the local
    /// `shortcuts` map, so it may return `true` before the ArkTS registration
    /// has completed (or even if ArkTS registration ultimately fails). Blocking
    /// to wait for confirmation would risk a deadlock with the main thread, so
    /// this timing gap is an accepted trade-off.
    pub fn is_registered<S: TryInto<ShortcutWrapper>>(&self, shortcut: S) -> bool
    where
        S::Error: std::error::Error,
    {
        if let Ok(shortcut) = try_into_shortcut(shortcut) {
            lock_shortcuts(&self.shortcuts).contains_key(&shortcut.id())
        } else {
            false
        }
    }
}

pub trait GlobalShortcutExt<R: Runtime> {
    fn global_shortcut(&self) -> &GlobalShortcut<R>;
}

impl<R: Runtime, T: Manager<R>> GlobalShortcutExt<R> for T {
    fn global_shortcut(&self) -> &GlobalShortcut<R> {
        self.state::<GlobalShortcut<R>>().inner()
    }
}

fn parse_shortcut<S: AsRef<str>>(shortcut: S) -> Result<Shortcut> {
    #[cfg(not(target_env = "ohos"))]
    {
        shortcut.as_ref().parse().map_err(Into::into)
    }
    #[cfg(target_env = "ohos")]
    {
        shortcut
            .as_ref()
            .parse::<Shortcut>()
            .map_err(|e| Error::GlobalHotkey(e))
    }
}

fn try_into_shortcut<S: TryInto<ShortcutWrapper>>(shortcut: S) -> Result<Shortcut>
where
    S::Error: std::error::Error,
{
    shortcut
        .try_into()
        .map(|s| s.0)
        .map_err(|e| Error::GlobalHotkey(e.to_string()))
}

#[derive(Clone, Serialize)]
struct ShortcutJsEvent {
    shortcut: String,
    id: u32,
    state: ShortcutState,
}

#[tauri::command]
fn register<R: Runtime>(
    _app: AppHandle<R>,
    global_shortcut: State<'_, GlobalShortcut<R>>,
    shortcuts: Vec<String>,
    handler: Channel<ShortcutJsEvent>,
) -> Result<()> {
    let mut hotkeys = Vec::new();

    let mut shortcut_map = HashMap::new();
    for shortcut in shortcuts {
        let hotkey = parse_shortcut(&shortcut)?;
        shortcut_map.insert(hotkey.id(), shortcut);
        hotkeys.push(hotkey);
    }

    global_shortcut.register_multiple_internal(
        hotkeys,
        Some(
            move |_app: &AppHandle<R>, shortcut: &Shortcut, e: ShortcutEvent| {
                let js_event = ShortcutJsEvent {
                    id: e.id,
                    state: e.state,
                    shortcut: shortcut.into_string(),
                };
                let _ = handler.send(js_event);
            },
        ),
    )
}

#[tauri::command]
fn unregister<R: Runtime>(
    _app: AppHandle<R>,
    global_shortcut: State<'_, GlobalShortcut<R>>,
    shortcuts: Vec<String>,
) -> Result<()> {
    let mut hotkeys = Vec::new();
    for shortcut in shortcuts {
        hotkeys.push(parse_shortcut(&shortcut)?);
    }
    global_shortcut.unregister_multiple(hotkeys)
}

#[tauri::command]
fn unregister_all<R: Runtime>(
    _app: AppHandle<R>,
    global_shortcut: State<'_, GlobalShortcut<R>>,
) -> Result<()> {
    global_shortcut.unregister_all()
}

#[tauri::command]
fn is_registered<R: Runtime>(
    _app: AppHandle<R>,
    global_shortcut: State<'_, GlobalShortcut<R>>,
    shortcut: String,
) -> Result<bool> {
    Ok(global_shortcut.is_registered(parse_shortcut(shortcut)?))
}

pub struct Builder<R: Runtime> {
    shortcuts: Vec<Shortcut>,
    handler: Option<HandlerFn<R>>,
}

impl<R: Runtime> Default for Builder<R> {
    fn default() -> Self {
        Self {
            shortcuts: Vec::new(),
            handler: Default::default(),
        }
    }
}

impl<R: Runtime> Builder<R> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a shortcut to be registerd.
    pub fn with_shortcut<T>(mut self, shortcut: T) -> Result<Self>
    where
        T: TryInto<ShortcutWrapper>,
        T::Error: std::error::Error,
    {
        self.shortcuts.push(try_into_shortcut(shortcut)?);
        Ok(self)
    }

    /// Add multiple shortcuts to be registerd.
    pub fn with_shortcuts<S, T>(mut self, shortcuts: S) -> Result<Self>
    where
        S: IntoIterator<Item = T>,
        T: TryInto<ShortcutWrapper>,
        T::Error: std::error::Error,
    {
        for shortcut in shortcuts {
            self.shortcuts.push(try_into_shortcut(shortcut)?);
        }

        Ok(self)
    }

    /// Specify a global shortcut handler that will be triggered for any and all shortcuts.
    pub fn with_handler<F: Fn(&AppHandle<R>, &Shortcut, ShortcutEvent) + Send + Sync + 'static>(
        mut self,
        handler: F,
    ) -> Self {
        self.handler.replace(Box::new(handler));
        self
    }

    pub fn build(self) -> TauriPlugin<R> {
        let handler = self.handler;
        let shortcuts = self.shortcuts;
        PluginBuilder::new("global-shortcut")
            .invoke_handler(tauri::generate_handler![
                register,
                unregister,
                unregister_all,
                is_registered,
            ])
            .setup(move |app, _api| {
                let mut store = HashMap::<HotKeyId, RegisteredShortcut<R>>::new();

                // ─── Desktop: use global-hotkey ──────────────────────────
                #[cfg(not(target_env = "ohos"))]
                {
                    let manager = global_hotkey::GlobalHotKeyManager::new()?;
                    for shortcut in shortcuts {
                        manager.register(shortcut.clone())?;
                        store.insert(
                            shortcut.id(),
                            RegisteredShortcut {
                                shortcut,
                                handler: None,
                            },
                        );
                    }

                    let shortcuts = Arc::new(Mutex::new(store));
                    let shortcuts_ = shortcuts.clone();

                    let app_handle = app.clone();
                    GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| {
                        if let Some(shortcut) = shortcuts_.lock().unwrap().get(&e.id) {
                            if let Some(handler) = &shortcut.handler {
                                handler(&app_handle, &shortcut.shortcut, e.clone());
                            }
                            if let Some(handler) = &handler {
                                handler(&app_handle, &shortcut.shortcut, e);
                            }
                        }
                    }));

                    app.manage(GlobalShortcut {
                        app: app.clone(),
                        manager: Arc::new(GlobalHotKeyManager(manager)),
                        shortcuts,
                    });
                }

                // ─── OHOS: use openharmony-ability ───────────────────────
                #[cfg(target_env = "ohos")]
                ohos_setup(app.clone(), shortcuts, handler, store);

                Ok(())
            })
            .build()
    }
}
