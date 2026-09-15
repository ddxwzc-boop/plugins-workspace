// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

// Note: `check_app_gallery_update` and `show_app_gallery_update_dialog` are not
// part of the upstream (v2) command surface, but they are the only updater
// commands registered on OpenHarmony, where updates are hosted by AppGallery -
// the desktop commands (`check`/`download`/`install`/`download_and_install`)
// are not available on that platform.
const COMMANDS: &[&str] = &[
    "check",
    "download",
    "install",
    "download_and_install",
    "check_app_gallery_update",
    "show_app_gallery_update_dialog",
];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .global_api_script_path("./api-iife.js")
        .build();

    tauri_plugin::cfg_aliases();
}
