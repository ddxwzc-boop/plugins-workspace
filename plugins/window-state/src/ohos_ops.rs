// Copyright 2019-2024 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! Platform-dispatched operations for the window-state plugin.
//!
//! Each function comes in two fn-level-`cfg` variants — OHOS and non-OHOS —
//! so the call sites in `lib.rs` stay free of `cfg` attributes. The OHOS
//! variants carry the OHOS lock-discipline workarounds (documented inline);
//! the non-OHOS variants keep the original desktop behavior verbatim.

use std::fs::create_dir_all;

use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, RunEvent, Runtime, Window};

use crate::{MonitorExt, PluginState, Result, StateFlags, WindowState, WindowStateCache};

#[cfg(target_env = "ohos")]
use std::collections::HashMap;

#[cfg(target_env = "ohos")]
use crate::WindowExt;

#[cfg(not(target_env = "ohos"))]
use crate::{AppHandleExt, WindowExtInternal};

// `AppHandleExt::save_window_state`, OHOS variant.
//
// The OHOS path below saves all tracked state and ignores `flags`.
#[cfg(target_env = "ohos")]
pub(crate) fn save_window_state<R: Runtime>(app: &AppHandle<R>, flags: StateFlags) -> Result<()> {
    let _ = &flags;
    let app_dir = app.path().app_config_dir()?;
    let plugin_state = app.state::<PluginState>();
    let state_path = app_dir.join(&plugin_state.filename);
    let windows = app.webview_windows();
    let cache = app.state::<WindowStateCache>();

    // OHOS: collect fresh geometry BEFORE taking the cache lock.
    //
    // outer_position()/inner_size() on OHOS are NOT lock-free reads: the
    // WryWindowDispatcher transports each query to the main thread and
    // blocks on mpmc recv() for the reply. Holding the cache lock across
    // that round-trip deadlocks when the main thread is concurrently in
    // the Resized/Moved handler waiting for this same lock — worker holds
    // cache + waits for main, main waits for cache (faultlog 2026-08-25
    // ×3: save_window_state→outer_position→recv vs on_window_event→
    // cache.lock, permanent THREAD_BLOCK_6S). Collect unlocked first:
    // blocking recv is then harmless because the main thread is free to
    // answer.
    let fresh_geometry: Vec<(String, Option<(i32, i32)>, Option<(u32, u32)>)> = {
        // Short lock #1: snapshot tracked labels only (no dispatcher calls).
        let tracked: Vec<String> = cache.0.lock().unwrap().keys().cloned().collect();
        // Unlocked collection: dispatcher round-trips may block on the
        // main thread, but no lock is held while they do.
        let mut fresh = Vec::with_capacity(tracked.len());
        for (window_label, window) in &windows {
            let label: String = plugin_state
                .map_label
                .as_ref()
                .map(|map| map(window_label).to_string())
                .unwrap_or_else(|| window_label.clone());
            if !tracked.contains(&label) {
                continue;
            }
            let position = window.outer_position().ok().map(|pos| (pos.x, pos.y));
            let size = window
                .inner_size()
                .ok()
                .and_then(|size| (size.width > 0 && size.height > 0).then_some((size.width, size.height)));
            fresh.push((label, position, size));
        }
        fresh
    };

    let mut state = cache.0.lock().unwrap();

    // OHOS: the desktop update_state() loop is not run here. update_state()
    // calls is_maximized()/is_minimized(), which on tao OHOS are synchronous
    // NAPI calls (is_window_maximized/is_window_minimized) that block the
    // main thread during window transitions — from a tokio worker thread
    // (save_window_state) this risks appfreeze / "failed to receive message
    // from webview". The other queries in update_state (inner_size,
    // outer_position, is_fullscreen, is_decorated, is_visible) are also NOT
    // free on OHOS: the tao layer reads the window_rect cache, but the
    // runtime-wry dispatcher transports each query to the main thread with
    // a blocking recv() — so update_state runs them as one batch under the
    // cache lock, which is why this path collects geometry unlocked (see
    // fresh_geometry) instead. size+position are refreshed in the loop below
    // instead of relying on Moved/Resized event handlers alone (see comment
    // there for why event-driven caching is insufficient on OHOS).
    //
    // OHOS save branch: unconditionally refresh size + position for every
    // tracked window (no longer gated on `flags`, and no longer temporarily
    // gated to the main window only).
    //
    // Reason 1 (why flags gating was removed): serde serializes the entire
    // WindowState struct (no skip_serializing_if), so even when the caller
    // passes only the SIZE flag, stale x/y are persisted together; on
    // restore with state_flags=all, a stale (0,0) would be applied to the
    // window. Thus size and position must be refreshed together at save
    // time, independent of `flags`.
    //
    // Reason 2 (why getters are used instead of relying on the
    // Moved/Resized event cache): tao OHOS dispatches windowRectChange
    // (MOVE/DRAG) as Event::ContentRectChange rather than
    // WindowEvent::Moved, so the Moved handler never fires and position
    // stays at the creation default; size is refreshed by the Resized
    // handler, but a race between save and the event may persist a stale
    // value. Note: outer_position()/inner_size() read the per-window rect
    // cache at the tao layer, but the runtime-wry dispatcher transport is a
    // blocking round-trip ("send message to main thread + recv for the
    // reply") — therefore geometry collection MUST happen outside the cache
    // lock (see the fresh_geometry block above); the collected results are
    // written back inside the lock.
    //
    // Reason 3 (why the main-window gate was dropped): after Phase 2, tao's
    // inner_size/outer_position go through window_rect_for(window_id)
    // (design.md D5); each window reads its own rect with no interference.
    // The Phase 1 `if window.label() != "main" { continue; }` temporary gate
    // has been removed — all tracked windows are refreshed.
    for (label, s) in state.iter_mut() {
        if let Some((_, position, size)) = fresh_geometry.iter().find(|(l, _, _)| l == label) {
            if let Some((x, y)) = position {
                s.x = *x;
                s.y = *y;
            }
            if let Some((width, height)) = size {
                s.width = *width;
                s.height = *height;
            }
        }
    }

    // OHOS: serialize while holding the cache lock, then release it before
    // disk I/O. save_window_state runs on a tokio worker; the main thread's
    // Resized handler locks this same cache, and holding it across fs::write
    // blocks that handler for the duration of a synchronous disk write —
    // observed as a transient appfreeze (THREAD_BLOCK_6S) during
    // high-frequency window churn on OHOS. Concurrent saves may now
    // interleave their file writes (last writer wins), which is acceptable
    // for a best-effort state snapshot.
    {
        let serialized = serde_json::to_vec_pretty(&*state)?;
        drop(state);
        create_dir_all(app_dir)?;
        std::fs::write(state_path, serialized)?;
    }

    Ok(())
}

// `AppHandleExt::save_window_state`, non-OHOS variant: original desktop
// behavior.
#[cfg(not(target_env = "ohos"))]
pub(crate) fn save_window_state<R: Runtime>(app: &AppHandle<R>, flags: StateFlags) -> Result<()> {
    let app_dir = app.path().app_config_dir()?;
    let plugin_state = app.state::<PluginState>();
    let state_path = app_dir.join(&plugin_state.filename);
    let windows = app.webview_windows();
    let cache = app.state::<WindowStateCache>();
    let mut state = cache.0.lock().unwrap();

    for (label, s) in state.iter_mut() {
        let window = if let Some(map) = &plugin_state.map_label {
            windows
                .iter()
                .find_map(|(l, window)| (map(l) == label).then_some(window))
        } else {
            windows.get(label)
        };

        if let Some(window) = window {
            window.update_state(s, flags)?;
        }
    }

    // Non-OHOS: keep the original desktop behavior — the cache lock is held
    // across the disk write (no deadlock risk on desktop where save runs on
    // the event loop and getters execute inline).
    create_dir_all(app_dir)?;
    std::fs::write(state_path, serde_json::to_vec_pretty(&*state)?)?;

    Ok(())
}

// `WindowExt::restore_state` (for `Window`), OHOS variant.
//
// Same lock discipline as the OHOS save path — the cache lock must never be
// held across a window_getter! round-trip (available_monitors does rx.recv()
// on the main thread) or disk I/O when the caller is a tokio worker (the
// cmd.rs async command, no main-thread short-circuit): the main thread's
// Resized/Moved handlers take cache.lock() and would mutually block
// (THREAD_BLOCK_6S). Triggered by the default StateFlags::all() which
// includes POSITION. Three phases: unlocked file read → short locked
// snapshot/writeback → unlocked window operations (all setters are
// fire-and-forget dispatches, safe from any thread).
//
// Desktop platforms are unaffected (tao controls window creation, the event
// loop executes getters inline); their original code path is kept
// byte-for-byte in the non-OHOS variant below.
#[cfg(target_env = "ohos")]
pub(crate) fn restore_state<R: Runtime>(
    window: &Window<R>,
    plugin_state: &PluginState,
    label: &str,
    flags: StateFlags,
) -> tauri::Result<()> {
    // Phase 0 (no lock): re-read the saved state from the file. The
    // window is created by the OS ability (at a default position)
    // before this plugin's on_window_ready fires; the Moved event from
    // window creation overwrites the cache (populated from file during
    // setup) with the default position, so the file is the source of
    // truth here. Reading it inside the cache lock would be lock-held
    // disk I/O (same class as the fixed fs::write-in-lock on save).
    let mut file_cache: Option<HashMap<String, WindowState>> = None;
    if let Ok(app_dir) = window.app_handle().path().app_config_dir() {
        let state_path = app_dir.join(&plugin_state.filename);
        if state_path.exists() {
            if let Ok(data) = std::fs::read(&state_path) {
                file_cache =
                    serde_json::from_slice::<HashMap<String, WindowState>>(&data).ok();
            }
        }
    }

    // Phase 1 (short lock): write the file value back into the cache,
    // then snapshot the state to restore (or seed defaults when there
    // is none — the non-OHOS else-branch queries the window here via
    // window_getter! calls, which must stay off worker threads on OHOS;
    // the Resized/Moved handlers populate real values on subsequent
    // events).
    let cache = window.state::<WindowStateCache>();
    let saved = {
        let mut c = cache.0.lock().unwrap();
        if let Some(saved) = file_cache.as_ref().and_then(|fc| fc.get(label)) {
            c.insert(label.to_string(), saved.clone());
        }
        let saved = c
            .get(label)
            .filter(|state| state != &&WindowState::default())
            .cloned();
        if saved.is_none() {
            c.insert(label.to_string(), WindowState::default());
        }
        saved
    }; // cache lock released

    // Phase 2 (no lock): apply the state to the window.
    //
    // KNOWN LIMITATION (OHOS): maximize, fullscreen, and decorations state
    // are silently dropped on OHOS — the setters are guarded by cfg(desktop)
    // because these states have no window-management meaning on the
    // phone/tablet form factor, where the app surface is the window itself
    // (tao's OHOS backend does implement set_maximized()/set_fullscreen()/
    // set_decorations(), so this is a deliberate semantic choice, not a
    // missing capability). Their saved values are still persisted to disk
    // but have no effect on restore. Position and size are restored.
    let mut should_show = true;

    if let Some(state) = saved {
        if flags.contains(StateFlags::DECORATIONS) {
            #[cfg(desktop)]
            window.set_decorations(state.decorated)?;
        }

        if flags.contains(StateFlags::POSITION) {
            let position = (state.x, state.y).into();
            let size = (state.width, state.height).into();
            for m in window.available_monitors()? {
                if m.intersects(position, size) {
                    window.set_position(PhysicalPosition {
                        x: if state.maximized { state.prev_x } else { state.x },
                        y: if state.maximized { state.prev_y } else { state.y },
                    })?;
                }
            }
        }

        if flags.contains(StateFlags::SIZE) {
            window.set_size(PhysicalSize {
                width: state.width,
                height: state.height,
            })?;
        }

        if flags.contains(StateFlags::MAXIMIZED) && state.maximized {
            #[cfg(desktop)]
            window.maximize()?;
        }

        if flags.contains(StateFlags::FULLSCREEN) {
            #[cfg(desktop)]
            window.set_fullscreen(state.fullscreen)?;
        }

        should_show = state.visible;
    }

    if flags.contains(StateFlags::VISIBLE) && should_show {
        window.show()?;
        window.set_focus()?;
    }

    Ok(())
}

// `WindowExt::restore_state` (for `Window`), non-OHOS variant: original
// desktop behavior.
#[cfg(not(target_env = "ohos"))]
pub(crate) fn restore_state<R: Runtime>(
    window: &Window<R>,
    _plugin_state: &PluginState,
    label: &str,
    flags: StateFlags,
) -> tauri::Result<()> {
    let cache = window.state::<WindowStateCache>();
    let mut c = cache.0.lock().unwrap();

    let mut should_show = true;

    if let Some(state) = c
        .get(label)
        .filter(|state| state != &&WindowState::default())
    {
        if flags.contains(StateFlags::DECORATIONS) {
            window.set_decorations(state.decorated)?;
        }

        if flags.contains(StateFlags::POSITION) {
            let position = (state.x, state.y).into();
            let size = (state.width, state.height).into();
            for m in window.available_monitors()? {
                if m.intersects(position, size) {
                    window.set_position(PhysicalPosition {
                        x: if state.maximized { state.prev_x } else { state.x },
                        y: if state.maximized { state.prev_y } else { state.y },
                    })?;
                }
            }
        }

        if flags.contains(StateFlags::SIZE) {
            window.set_size(PhysicalSize {
                width: state.width,
                height: state.height,
            })?;
        }

        if flags.contains(StateFlags::MAXIMIZED) && state.maximized {
            window.maximize()?;
        }

        if flags.contains(StateFlags::FULLSCREEN) {
            window.set_fullscreen(state.fullscreen)?;
        }

        should_show = state.visible;
    } else {
        let mut metadata = WindowState::default();

        if flags.contains(StateFlags::SIZE) {
            let size = window.inner_size()?;
            metadata.width = size.width;
            metadata.height = size.height;
        }

        if flags.contains(StateFlags::POSITION) {
            let pos = window.outer_position()?;
            metadata.x = pos.x;
            metadata.y = pos.y;
        }

        if flags.contains(StateFlags::MAXIMIZED) {
            metadata.maximized = window.is_maximized()?;
        }

        if flags.contains(StateFlags::VISIBLE) {
            metadata.visible = window.is_visible()?;
        }

        if flags.contains(StateFlags::DECORATIONS) {
            metadata.decorated = window.is_decorated()?;
        }

        if flags.contains(StateFlags::FULLSCREEN) {
            metadata.fullscreen = window.is_fullscreen()?;
        }

        c.insert(label.into(), metadata);
    }

    if flags.contains(StateFlags::VISIBLE) && should_show {
        window.show()?;
        window.set_focus()?;
    }

    Ok(())
}

// The plugin's `on_event` handler, OHOS variant.
#[cfg(target_env = "ohos")]
pub(crate) fn on_event<R: Runtime>(app: &AppHandle<R>, event: &RunEvent, state_flags: StateFlags) {
    if let RunEvent::Ready = &event {
        // Restore on Ready because on_window_ready does not fire for the
        // main window on OHOS (created before the plugin registers).
        // Apply the same denylist / filter / skip_initial_state gating as
        // on_window_ready so excluded windows (e.g. splash) are not restored.
        let windows_to_restore: Vec<_> = {
            let plugin_state = app.state::<PluginState>();
            app.webview_windows()
                .into_iter()
                .filter(|(_, window)| {
                    let label = plugin_state
                        .map_label
                        .as_ref()
                        .map(|map| map(window.label()))
                        .unwrap_or_else(|| window.label());
                    if plugin_state.denylist.contains(label) {
                        return false;
                    }
                    if let Some(filter_callback) = &plugin_state.filter_callback {
                        if !filter_callback(label) {
                            return false;
                        }
                    }
                    !plugin_state.skip_initial_state.contains(label)
                })
                .map(|(_, w)| w)
                .collect()
        };
        for window in windows_to_restore {
            let _ = window.restore_state(state_flags);
        }
    }
    // OHOS: skip auto-save on Exit. The user controls persistence
    // via the explicit save_window_state command (Save/Restore button).
    // Auto-save on Exit would overwrite the user's explicit save with
    // the current (possibly moved) position.
}

// The plugin's `on_event` handler, non-OHOS variant: original desktop
// behavior.
#[cfg(not(target_env = "ohos"))]
pub(crate) fn on_event<R: Runtime>(app: &AppHandle<R>, event: &RunEvent, state_flags: StateFlags) {
    if let RunEvent::Exit = event {
        let _ = app.save_window_state(state_flags);
    }
}

// update_state: is_maximized()/is_minimized() queries, platform-dispatched.
//
// OHOS: is_maximized()/is_minimized() use synchronous NAPI calls
// (is_window_maximized/is_window_minimized) that block the main thread
// during window transitions (CloseRequested etc.), causing appfreeze /
// test timeouts (see the Resized/Moved guards below). Skip on OHOS
// (set false); the other queries in update_state (is_fullscreen/
// is_decorated/is_visible/inner_size/outer_position) are default values
// or cached, non-blocking. Non-OHOS keeps the original synchronous
// queries.
#[cfg(target_env = "ohos")]
pub(crate) fn update_state_is_maximized<R: Runtime>(
    _window: &Window<R>,
    _flags: StateFlags,
) -> tauri::Result<bool> {
    Ok(false)
}

#[cfg(not(target_env = "ohos"))]
pub(crate) fn update_state_is_maximized<R: Runtime>(
    window: &Window<R>,
    flags: StateFlags,
) -> tauri::Result<bool> {
    Ok(
        flags.intersects(StateFlags::MAXIMIZED | StateFlags::POSITION | StateFlags::SIZE)
            && window.is_maximized()?,
    )
}

#[cfg(target_env = "ohos")]
pub(crate) fn update_state_is_minimized<R: Runtime>(
    _window: &Window<R>,
    _flags: StateFlags,
) -> tauri::Result<bool> {
    Ok(false)
}

#[cfg(not(target_env = "ohos"))]
pub(crate) fn update_state_is_minimized<R: Runtime>(
    window: &Window<R>,
    flags: StateFlags,
) -> tauri::Result<bool> {
    Ok(flags.intersects(StateFlags::POSITION | StateFlags::SIZE) && window.is_minimized()?)
}

// Moved-event handler minimized guard.
//
// OHOS: is_minimized() 经 window_getter! → is_window_minimized 的同步
// NAPI 调用,在窗口过渡期(Moved/Resized 事件回调中)阻塞主线程致 appfreeze
// (run_on_main_thread + recv 重入死锁)。OHOS 上跳过该守卫——最小化窗口不
// 触发 Moved。
#[cfg(target_env = "ohos")]
pub(crate) fn moved_is_minimized<R: Runtime>(_window: &Window<R>) -> bool {
    false
}

// 其他平台保留原同步查询(其 send_user_message 主线程短路后直接
// handle_user_message,非阻塞)。
#[cfg(not(target_env = "ohos"))]
pub(crate) fn moved_is_minimized<R: Runtime>(window: &Window<R>) -> bool {
    window.is_minimized().unwrap_or_default()
}

// Resized-event handler save guard.
//
// OHOS: is_minimized()/is_maximized() 同步查询在 resize 过渡期阻塞主线程
// 致 appfreeze(见 moved_is_minimized 注释)。OHOS 上跳过守卫无条件保存——
// 最小化不触发 Resized;最大化由 close 时 update_state 捕获 state.maximized
// 驱动恢复,保存的尺寸不影响恢复。
#[cfg(target_env = "ohos")]
pub(crate) fn resized_should_save<R: Runtime>(_window: &Window<R>) -> bool {
    true
}

// 其他平台保留原同步查询。
#[cfg(not(target_env = "ohos"))]
pub(crate) fn resized_should_save<R: Runtime>(window: &Window<R>) -> bool {
    // TODO: Remove once https://github.com/tauri-apps/tauri/issues/5812 is resolved.
    let is_maximized = if cfg!(target_os = "macos")
        && (!window.is_decorated().unwrap_or_default()
            || !window.is_resizable().unwrap_or_default())
    {
        false
    } else {
        window.is_maximized().unwrap_or_default()
    };
    !window.is_minimized().unwrap_or_default() && !is_maximized
}
