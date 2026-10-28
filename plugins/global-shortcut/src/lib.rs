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
    /// OHOS: monotonic registration stamp (doubles as the request's
    /// `generation` on the ArkTS wire — see `ohos::next_registration_stamp`).
    /// The entry is inserted synchronously and removed from a worker thread
    /// when ArkTS rejects the request; the stamp makes that removal
    /// conditional so a same-id re-registration that already replaced the
    /// entry is not undone by the stale worker of the earlier attempt.
    #[cfg(target_env = "ohos")]
    stamp: u32,
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

/// Unified version gate for OHOS global shortcuts: `inputConsumer.on('hotkeyChange')`
/// is API 14+. Every register/unregister/query entry point checks this up front.
/// Why the gate must live on this side (plugins#34 review F8, second round):
/// the OHOS register dispatch is fire-and-forget — `register_internal` inserts
/// into the local map and returns `Ok(())` before ArkTS answers, and the ArkTS
/// `registerHotkey` catches the sub-14 `inputConsumer.on` failure itself
/// (worker log only, returns false), after which the spawned worker rolls the
/// local entry back. A <14 caller without this gate therefore gets a resolved
/// register whose shortcut silently never exists; the gate turns that into an
/// immediate, descriptive Err. Note the one visible behavior change this
/// introduces, called out in the PR description: `isRegistered` on <14 rejects
/// with this message instead of resolving a bool (on other platforms the
/// command always answers with a bool).
#[cfg(target_env = "ohos")]
fn ohos_require_hotkey_api(op: &str) -> Result<()> {
    use openharmony_ability_plugin_global_shortcut::version;

    const MIN_HOTKEY_API_VERSION: i32 = 14;
    let current = version::sdk_api_version();
    if current < MIN_HOTKEY_API_VERSION {
        return Err(Error::GlobalHotkey(format!(
            "{op} requires API level {MIN_HOTKEY_API_VERSION}+ on OpenHarmony (current: {current})"
        )));
    }
    Ok(())
}

impl<R: Runtime> GlobalShortcut<R> {
    fn register_internal<F: Fn(&AppHandle<R>, &Shortcut, ShortcutEvent) + Send + Sync + 'static>(
        &self,
        shortcut: Shortcut,
        handler: Option<F>,
    ) -> Result<()> {
        let id = shortcut.id();
        let handler = handler.map(|h| Arc::new(Box::new(h) as HandlerFn<R>));

        #[cfg(target_env = "ohos")]
        ohos_require_hotkey_api("register")?;

        #[cfg(not(target_env = "ohos"))]
        {
            run_main_thread!(self.app, self.manager, |m| m.0.register(shortcut.clone()))?;
            lock_shortcuts(&self.shortcuts)
                .insert(id, RegisteredShortcut { shortcut, handler });
        }

        #[cfg(target_env = "ohos")]
        {
            let client = self.client.as_ref().ok_or_else(|| {
                Error::GlobalHotkey("GlobalShortcutClient not initialized".to_string())
            })?;
            let modifiers = shortcut.modifiers().to_vec();
            let key = shortcut.code().to_ohos_name().to_string();
            // Insert synchronously (a hotkey that fires immediately must find
            // its handler); if ArkTS later rejects the registration, the
            // spawned call removes the entry again so `isRegistered` cannot
            // report a ghost registration — unless the same id was
            // re-registered in between (the stamp makes the removal
            // generation-aware). The stamp is taken under the same lock as
            // the insert so two racing registrations of the same id cannot
            // leave the older attempt's stamp in the map (its worker would
            // then remove the newer registration).
            let stamp = {
                let mut shortcuts = lock_shortcuts(&self.shortcuts);
                let stamp = ohos::next_registration_stamp();
                shortcuts.insert(
                    id,
                    RegisteredShortcut {
                        shortcut,
                        handler,
                        stamp,
                    },
                );
                stamp
            };
            ohos::spawn_register(
                client.clone(),
                id,
                &modifiers,
                &key,
                stamp,
                self.shortcuts.clone(),
            );
        }

        Ok(())
    }

    fn register_multiple_internal<S, F>(&self, shortcuts: S, handler: Option<F>) -> Result<()>
    where
        S: IntoIterator<Item = Shortcut>,
        F: Fn(&AppHandle<R>, &Shortcut, ShortcutEvent) + Send + Sync + 'static,
    {
        let handler = handler.map(|h| Arc::new(Box::new(h) as HandlerFn<R>));

        #[cfg(target_env = "ohos")]
        ohos_require_hotkey_api("register")?;

        let hotkeys = shortcuts.into_iter().collect::<Vec<_>>();

        let mut shortcuts = lock_shortcuts(&self.shortcuts);
        for shortcut in hotkeys {
            #[cfg(not(target_env = "ohos"))]
            {
                run_main_thread!(self.app, self.manager, |m| m.0.register(shortcut.clone()))?;
            }

            // Stamp before dispatch so the insert below and the spawned
            // removal check agree on the registration generation.
            #[cfg(target_env = "ohos")]
            let stamp = ohos::next_registration_stamp();

            #[cfg(target_env = "ohos")]
            {
                let client = self.client.as_ref().ok_or_else(|| {
                    Error::GlobalHotkey("GlobalShortcutClient not initialized".to_string())
                })?;
                let modifiers = shortcut.modifiers().to_vec();
                let key = shortcut.code().to_ohos_name().to_string();
                let sid = shortcut.id();
                let client = client.clone();
                // If ArkTS rejects the registration, the spawned call removes
                // the entry this loop inserts below so `isRegistered` cannot
                // report a ghost registration. This loop holds the lock, so the
                // removal can only run after every insert has completed; the
                // stamp keeps a stale worker from removing a re-registered id.
                ohos::spawn_register(client, sid, &modifiers, &key, stamp, self.shortcuts.clone());
            }

            shortcuts.insert(
                shortcut.id(),
                RegisteredShortcut {
                    shortcut,
                    handler: handler.clone(),
                    #[cfg(target_env = "ohos")]
                    stamp,
                },
            );
        }

        Ok(())
    }

    /// Awaited variant of [`register_multiple_internal`] for the JS-facing
    /// `register` command on OHOS: awaits the real ArkTS receipt so a
    /// rejected registration rejects the command's promise (desktop parity —
    /// upstream `register` blocks on the main thread and returns the error).
    ///
    /// Keeps the same insert-before-dispatch + stamp discipline as the
    /// fire-and-forget path: the store lock is never held across an await,
    /// a hotkey that fires immediately finds its handler, and a rejection
    /// removes the entry again (generation-aware). Like desktop fail-fast,
    /// earlier successes in the batch stay registered when one entry fails.
    #[cfg(target_env = "ohos")]
    async fn register_multiple_internal_awaited<S, F>(&self, shortcuts: S, handler: Option<F>) -> Result<()>
    where
        S: IntoIterator<Item = Shortcut>,
        F: Fn(&AppHandle<R>, &Shortcut, ShortcutEvent) + Send + Sync + 'static,
    {
        ohos_require_hotkey_api("register")?;

        let handler = handler.map(|h| Arc::new(Box::new(h) as HandlerFn<R>));
        let hotkeys = shortcuts.into_iter().collect::<Vec<_>>();

        for shortcut in hotkeys {
            let id = shortcut.id();
            let client = self.client.as_ref().ok_or_else(|| {
                Error::GlobalHotkey("GlobalShortcutClient not initialized".to_string())
            })?;
            let modifiers = shortcut.modifiers().to_vec();
            let key = shortcut.code().to_ohos_name().to_string();
            let modifier_names = ohos::to_ohos_modifier_names(&modifiers);
            let stamp = {
                let mut shortcuts = lock_shortcuts(&self.shortcuts);
                let stamp = ohos::next_registration_stamp();
                shortcuts.insert(
                    id,
                    RegisteredShortcut {
                        shortcut,
                        handler: handler.clone(),
                        stamp,
                    },
                );
                stamp
            };
            ohos::register_and_cleanup(client, id, &modifier_names, &key, stamp, &self.shortcuts)
                .await
                .map_err(Error::GlobalHotkey)?;
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
        #[cfg(target_env = "ohos")]
        ohos_require_hotkey_api("unregister")?;

        let shortcut = try_into_shortcut(shortcut)?;

        #[cfg(not(target_env = "ohos"))]
        {
            run_main_thread!(self.app, self.manager, |m| m.0.unregister(shortcut.clone()))?;
            lock_shortcuts(&self.shortcuts).remove(&shortcut.id());
        }

        #[cfg(target_env = "ohos")]
        {
            // The fresh stamp travels as the request's generation; the
            // ArkTS side raises the per-id watermark to it, so an in-flight
            // register for this id dispatched before this call is rejected
            // on arrival instead of resurrecting the hotkey. Taken outside
            // the store lock: the counter is atomic, so the ordering that
            // matters (this stamp > every stamp of an earlier-issued
            // register for this id) follows from call order alone.
            let generation = ohos::next_registration_stamp();
            if let Some(ref client) = self.client {
                ohos::spawn_unregister(client.clone(), shortcut.id(), generation);
            }

            // Remove the local entry only when it predates this unregister:
            // an entry stamped later belongs to a re-register that raced
            // this call, and on the ArkTS side its newer generation either
            // survives the watermark this unregister raises or makes this
            // very unregister stale (skipped there) — the physical
            // registration stays live, so the local store must keep
            // reporting it. Removing unconditionally would leave a silent
            // dead hotkey: `isRegistered` false, no way to re-trigger it
            // except re-registering the same id.
            let mut shortcuts = lock_shortcuts(&self.shortcuts);
            if shortcuts
                .get(&shortcut.id())
                .is_some_and(|entry| entry.stamp < generation)
            {
                shortcuts.remove(&shortcut.id());
            }
        }

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
        #[cfg(target_env = "ohos")]
        ohos_require_hotkey_api("unregister")?;

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
            // Fresh stamps → generations, same call-order argument as
            // `unregister`: each per-id watermark keeps a stale in-flight
            // register for that id from resurrecting it.
            let mut generations = HashMap::new();
            for s in &mapped_shortcuts {
                let generation = ohos::next_registration_stamp();
                generations.insert(s.id(), generation);
                if let Some(ref client) = self.client {
                    ohos::spawn_unregister(client.clone(), s.id(), generation);
                }
            }

            // Per-id conditional removal, same rule as `unregister`: an
            // entry stamped after this batch's unregister for its id is a
            // re-register that raced the batch and survives on the ArkTS
            // side, so the local store keeps it.
            let mut shortcuts = lock_shortcuts(&self.shortcuts);
            for (id, generation) in &generations {
                if shortcuts
                    .get(id)
                    .is_some_and(|entry| entry.stamp < *generation)
                {
                    shortcuts.remove(id);
                }
            }
        }

        #[cfg(not(target_env = "ohos"))]
        {
            let mut shortcuts = lock_shortcuts(&self.shortcuts);
            for s in mapped_shortcuts {
                shortcuts.remove(&s.id());
            }
        }

        Ok(())
    }

    /// Unregister all registered shortcuts.
    ///
    /// # OHOS note
    /// The local store is cleared synchronously while the physical
    /// `inputConsumer.off()` calls happen asynchronously on the ArkTS main
    /// thread. The `mem::take` and the sweep's fresh generation stamp are
    /// taken under one store lock, and a register stamps and inserts under
    /// that same lock, so the two never interleave: every entry present at
    /// the take predates the sweep's generation — its in-flight register is
    /// rejected by the ArkTS generation floor, so a worker completing after
    /// this sweep cannot resurrect a hotkey this call has cleared (no ghost
    /// registrations). Conversely, a register that stamps after the sweep
    /// inserts after the take, so the local store keeps it — and the ArkTS
    /// sweep keeps the physical registration of any id whose per-id
    /// watermark postdates the sweep (no silent dead hotkeys).
    pub fn unregister_all(&self) -> Result<()> {
        #[cfg(target_env = "ohos")]
        ohos_require_hotkey_api("unregisterAll")?;

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
                // Fresh stamp → the sweep's generation (same call-order
                // argument as `unregister`: every register this sweep must
                // cancel took its stamp before this call, so this one is
                // larger). The ArkTS side raises its global generation
                // floor to it.
                let generation = ohos::next_registration_stamp();
                ohos::spawn_unregister_all(client.clone(), generation);
            }
            Ok(())
        }
    }

    /// Determines whether the given shortcut is registered by this application or not.
    ///
    /// If the shortcut is registered by another application, it will still return `false`.
    ///
    /// # OHOS note
    /// Registration is asynchronous: the Rust-side `register()` returns
    /// immediately after queueing the request, while the actual
    /// `inputConsumer.on()` call happens asynchronously on the ArkTS main
    /// thread. This function queries the local `shortcuts` map, so it may
    /// return `true` briefly before the ArkTS registration completes. The
    /// JS-facing `register` command awaits the real ArkTS receipt, so from
    /// JavaScript a resolved `register()` implies the registration is live
    /// and a rejection has already cleaned the local entry. If ArkTS
    /// rejects a fire-and-forget registration (Rust-side API), the entry is
    /// removed again, so subsequent calls report `false`. The reverse
    /// drift — a physical registration the local map does not know about —
    /// is closed by the generation guard on the ArkTS side: an unregister /
    /// unregister-all that raced an in-flight register supersedes it, and
    /// the stale register is rejected there instead of taking effect.
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

// Desktop: byte-identical upstream command (blocks on the main thread and
// propagates a registration failure as the command's error).
#[cfg(not(target_env = "ohos"))]
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

// OHOS: async so the command runs on the tokio pool (never the ArkTS main
// thread, where awaiting the bridge reply would deadlock) and can await the
// real ArkTS registration receipt — a rejected registration (hotkey taken,
// unsupported key, …) rejects the JS promise instead of resolving like the
// fire-and-forget path did (issue C71).
#[cfg(target_env = "ohos")]
#[tauri::command]
async fn register<R: Runtime>(
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

    global_shortcut
        .register_multiple_internal_awaited(
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
        .await
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
    #[cfg(target_env = "ohos")]
    ohos_require_hotkey_api("isRegistered")?;

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
                // `mut` is desktop-only: the OHOS path just moves the store
                // into `ohos_setup`.
                #[cfg(not(target_env = "ohos"))]
                let mut store = HashMap::<HotKeyId, RegisteredShortcut<R>>::new();
                #[cfg(target_env = "ohos")]
                let store = HashMap::<HotKeyId, RegisteredShortcut<R>>::new();

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

// ─── OHOS register/unregister race model tests ─────────────────────────────
//
// The OHOS backend updates its local store synchronously while the physical
// `inputConsumer` calls happen asynchronously on the ArkTS main thread, in
// arbitrary arrival order. The generation scheme (stamp → wire
// `generation` → ArkTS floor / per-id watermarks) keeps both sides
// consistent in BOTH directions:
// - forward (ghost hotkey): an in-flight register predating an
//   unregister / unregister-all must not resurrect a hotkey the Rust store
//   no longer tracks;
// - reverse (silent dead hotkey): a register dispatched *after* an
//   unregister / unregister-all must survive it — the Rust-side local
//   removal is conditional on the entry's stamp predating the unregister,
//   and the ArkTS sweep keeps ids whose per-id watermark postdates it.
//
// These tests model both sides of the wire — the Rust store with its
// stamps (lib.rs `register_internal` / `unregister` / `unregister_multiple`
// / `unregister_all`, ohos.rs `spawn_register`'s rejection cleanup) and the
// ArkTS floor / watermark / physical state exactly as
// `GlobalShortcutPlugin.ets` implements them — and enumerate every
// thread-interleaving × message-arrival order for the racing operation
// pairs. `unregister_multiple` follows the same per-id rule as
// `unregister` and is covered by the same model. Physical registration
// failures (`inputConsumer.on` BusinessError paths) are out of scope: they
// route through the same stamp-conditional cleanup as the stale rejection
// modeled here.
//
// The model is single-threaded; each operation's sub-steps mirror the real
// code's lock granularity (a register's stamp+insert happens under the
// store lock and is one atomic step, as is the `unregister_all` take+stamp;
// `unregister`'s stamp, dispatch and conditional removal are separate
// steps, the stamp being taken outside the lock). Draining the queue only
// after all steps finished is sound because the local-store steps and the
// message processing are causally independent — both touch the local map
// only through order-insensitive conditional idempotent removals.
//
// Invariant: at quiescence, the Rust local store and the ArkTS physical
// registrations hold exactly the same set of ids.

#[cfg(test)]
mod race_model {
    use std::collections::{BTreeSet, HashMap, HashSet};

    /// A message on the Rust → ArkTS bridge queue.
    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Msg {
        Register { id: u32, gen: u32 },
        Unregister { id: u32, gen: u32 },
        Sweep { gen: u32 },
    }

    /// Model of both sides of the generation protocol.
    #[derive(Clone, Default)]
    struct World {
        counter: u32,
        /// Rust side: the `shortcuts` store, id → entry stamp.
        local: HashMap<u32, u32>,
        /// Bridge queue (TSFN); arrival order is arbitrary.
        queue: Vec<Msg>,
        /// ArkTS side: mirrors `GlobalShortcutPlugin.ets`.
        floor: u32,
        watermarks: HashMap<u32, u32>,
        physical: HashSet<u32>,
    }

    impl World {
        /// `ohos::next_registration_stamp`.
        fn next_stamp(&mut self) -> u32 {
            self.counter += 1;
            self.counter
        }

        /// `GlobalShortcutPlugin.ets registerHotkey` (stale guards + success
        /// path).
        fn arkts_register(&mut self, id: u32, gen: u32) {
            if gen < self.floor || self.watermarks.get(&id).is_some_and(|&s| gen < s) {
                // Rejected as stale; the register worker's cleanup removes
                // the entry it inserted — but only if it still carries this
                // stamp (a newer re-register has replaced it otherwise).
                if self.local.get(&id) == Some(&gen) {
                    self.local.remove(&id);
                }
                return;
            }
            self.physical.insert(id);
            self.watermarks.insert(id, gen);
        }

        /// `GlobalShortcutPlugin.ets unregisterHotkey`.
        fn arkts_unregister(&mut self, id: u32, gen: u32) {
            if self.watermarks.get(&id).is_some_and(|&s| gen < s) {
                return; // stale: a newer request for this id was served
            }
            self.watermarks.insert(id, gen);
            self.physical.remove(&id);
        }

        /// `GlobalShortcutPlugin.ets unregisterAllHotkeys` — floor raise,
        /// subsumed-watermark prune, and the id-filtered physical sweep.
        fn arkts_sweep(&mut self, gen: u32) {
            if gen > self.floor {
                self.floor = gen;
            }
            self.watermarks.retain(|_, &mut g| g > gen);
            let ids: Vec<u32> = self.physical.iter().copied().collect();
            for id in ids {
                if !self.watermarks.get(&id).is_some_and(|&g| g > gen) {
                    self.physical.remove(&id);
                }
            }
        }

        /// Processes the queue in the given arrival order.
        fn drain(&mut self, order: &[usize]) {
            for &i in order {
                match self.queue[i].clone() {
                    Msg::Register { id, gen } => self.arkts_register(id, gen),
                    Msg::Unregister { id, gen } => self.arkts_unregister(id, gen),
                    Msg::Sweep { gen } => self.arkts_sweep(gen),
                }
            }
        }

        fn local_ids(&self) -> BTreeSet<u32> {
            self.local.keys().copied().collect()
        }

        fn physical_ids(&self) -> BTreeSet<u32> {
            self.physical.iter().copied().collect()
        }
    }

    /// One sub-step of an operation; the `u32` is the operation's local
    /// stamp scratch.
    type Step = Box<dyn Fn(&mut World, &mut u32)>;

    /// `register` (lib.rs `register_internal`): stamp+insert under the
    /// store lock, then dispatch.
    fn op_register(id: u32) -> Vec<Step> {
        vec![
            Box::new(move |w, t| {
                *t = w.next_stamp();
                w.local.insert(id, *t);
            }),
            Box::new(move |w, t| w.queue.push(Msg::Register { id, gen: *t })),
        ]
    }

    /// `unregister` (lib.rs): stamp outside the lock, dispatch, then the
    /// conditional local removal.
    fn op_unregister(id: u32) -> Vec<Step> {
        vec![
            Box::new(move |w, t| *t = w.next_stamp()),
            Box::new(move |w, t| w.queue.push(Msg::Unregister { id, gen: *t })),
            Box::new(move |w, t| {
                if w.local.get(&id).is_some_and(|&s| s < *t) {
                    w.local.remove(&id);
                }
            }),
        ]
    }

    /// `unregister_all` (lib.rs): the `mem::take` and the stamp happen
    /// under one store lock, so they are one atomic step here.
    fn op_unregister_all() -> Vec<Step> {
        vec![Box::new(move |w, t| {
            w.local.clear();
            *t = w.next_stamp();
            w.queue.push(Msg::Sweep { gen: *t });
        })]
    }

    /// Runs a single operation's steps sequentially (its own scratch).
    fn run_op(op: &[Step], w: &mut World) {
        let mut t = 0u32;
        for step in op {
            step(w, &mut t);
        }
    }

    /// Runs `a` and `b` in every interleaving (preserving each operation's
    /// step order) and calls `f` with the resulting world.
    fn for_each_interleaving(a: &[Step], b: &[Step], mut f: impl FnMut(World)) {
        let n = a.len() + b.len();
        for mask in 0u32..(1 << n) {
            if mask.count_ones() != a.len() as u32 {
                continue;
            }
            let mut w = World::default();
            let (mut ai, mut bi, mut ta, mut tb) = (0, 0, 0, 0);
            for i in 0..n {
                if mask & (1 << i) != 0 {
                    a[ai](&mut w, &mut ta);
                    ai += 1;
                } else {
                    b[bi](&mut w, &mut tb);
                    bi += 1;
                }
            }
            f(w);
        }
    }

    /// All arrival orders of `len` queued messages.
    fn all_orders(len: usize) -> Vec<Vec<usize>> {
        fn go(prefix: Vec<usize>, rest: Vec<usize>, out: &mut Vec<Vec<usize>>) {
            if rest.is_empty() {
                out.push(prefix);
                return;
            }
            for i in 0..rest.len() {
                let mut rest2 = rest.clone();
                let head = rest2.remove(i);
                let mut prefix2 = prefix.clone();
                prefix2.push(head);
                go(prefix2, rest2, out);
            }
        }
        let mut out = Vec::new();
        go(Vec::new(), (0..len).collect(), &mut out);
        out
    }

    /// For every interleaving of the two operations and every arrival order
    /// of the messages they dispatched, asserts the quiescence invariant:
    /// the Rust local store and the ArkTS physical registrations hold
    /// exactly the same set of ids.
    fn assert_pair_consistent(a: &[Step], b: &[Step]) {
        for_each_interleaving(a, b, |w| {
            for order in all_orders(w.queue.len()) {
                let mut drained = w.clone();
                drained.drain(&order);
                assert_eq!(
                    drained.local_ids(),
                    drained.physical_ids(),
                    "local store and physical registrations disagree \
                     (queue {:?} drained as {:?}; floor={}, watermarks={:?})",
                    w.queue,
                    order,
                    drained.floor,
                    drained.watermarks
                );
            }
        });
    }

    #[test]
    fn unregister_racing_re_register_is_bidirectionally_consistent() {
        // The forward direction (ghost hotkey): a register predating the
        // unregister must not resurrect the hotkey. The reverse direction
        // (silent dead hotkey): a register stamped after the unregister must
        // survive it. Enumerating every interleaving × arrival order covers
        // both, including the schedule where the register's insert lands
        // between the unregister's stamp and its conditional local removal.
        assert_pair_consistent(&op_unregister(7), &op_register(7));
    }

    #[test]
    fn unregister_all_racing_re_register_is_bidirectionally_consistent() {
        assert_pair_consistent(&op_unregister_all(), &op_register(7));
    }

    #[test]
    fn unregister_all_racing_unregister_is_consistent() {
        assert_pair_consistent(&op_unregister_all(), &op_unregister(7));
    }

    #[test]
    fn re_register_racing_unregister_survives_it() {
        // The ghost-race schedule an unconditional local removal breaks:
        // the unregister stamps and dispatches, the register stamps and
        // inserts between the unregister's dispatch and its local removal,
        // then the removal runs. The register is the newest operation for
        // the id and must be live on both sides, in every arrival order.
        let mut w = World::default();
        let (mut tu, mut tr) = (0, 0);
        let u = op_unregister(7);
        let r = op_register(7);
        u[0](&mut w, &mut tu); // unregister: stamp
        u[1](&mut w, &mut tu); // unregister: dispatch
        r[0](&mut w, &mut tr); // register: stamp + insert (newer)
        u[2](&mut w, &mut tu); // unregister: conditional local removal
        r[1](&mut w, &mut tr); // register: dispatch
        for order in all_orders(w.queue.len()) {
            let mut drained = w.clone();
            drained.drain(&order);
            assert!(
                drained.local_ids().contains(&7) && drained.physical_ids().contains(&7),
                "re-register that stamped after the unregister must survive it \
                 (queue {:?} drained as {:?})",
                w.queue,
                order
            );
        }
    }

    #[test]
    fn re_register_after_sweep_survives_the_sweep() {
        // Sequential register(X) → unregister_all → register(X) with every
        // arrival order: the second register is the newest operation for X
        // and must be live on both sides once the dust settles. Regression
        // test for the reverse race: an unconditional ArkTS watermark clear
        // (or a sweep without the per-id filter) would physically
        // unregister it while the Rust store keeps it.
        for order in all_orders(3) {
            let mut w = World::default();
            run_op(&op_register(7), &mut w);
            run_op(&op_unregister_all(), &mut w);
            run_op(&op_register(7), &mut w);
            let mut drained = w.clone();
            drained.drain(&order);
            assert_eq!(
                drained.local_ids(),
                BTreeSet::from([7]),
                "post-sweep re-register must stay in the local store \
                 (queue {:?} drained as {:?})",
                w.queue,
                order
            );
            assert_eq!(
                drained.physical_ids(),
                BTreeSet::from([7]),
                "post-sweep re-register must stay physically registered \
                 (queue {:?} drained as {:?})",
                w.queue,
                order
            );
        }
    }

    #[test]
    fn sweep_unregister_and_re_register_triple_is_consistent() {
        // Fixed adversarial sub-step schedule with all three racing
        // operations on one id, every arrival order: the unregister stamps
        // first, then the sweep's atomic take+stamp, then the re-register
        // stamps and inserts (newest of the three), then the unregister's
        // dispatch and conditional removal interleave with the register's
        // dispatch. The re-register is the newest operation for the id and
        // must be live on both sides in every arrival order.
        let mut w = World::default();
        let (mut tu, mut tr) = (0, 0);
        let u = op_unregister(7);
        let s = op_unregister_all();
        let r = op_register(7);
        u[0](&mut w, &mut tu); // unregister: stamp (oldest of the three)
        s[0](&mut w, &mut 0); // sweep: atomic take + stamp + dispatch
        r[0](&mut w, &mut tr); // register: stamp + insert (newest)
        u[1](&mut w, &mut tu); // unregister: dispatch
        u[2](&mut w, &mut tu); // unregister: conditional local removal
        r[1](&mut w, &mut tr); // register: dispatch
        for order in all_orders(w.queue.len()) {
            let mut drained = w.clone();
            drained.drain(&order);
            assert_eq!(
                drained.local_ids(),
                drained.physical_ids(),
                "triple race disagrees (queue {:?} drained as {:?})",
                w.queue,
                order
            );
            assert!(
                drained.local_ids().contains(&7),
                "newest re-register must survive the triple race \
                 (queue {:?} drained as {:?})",
                w.queue,
                order
            );
        }
    }
}
