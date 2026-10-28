![plugin-shell](https://github.com/tauri-apps/plugins-workspace/raw/v2/plugins/shell/banner.png)

Access the system shell. Allows you to spawn child processes and manage files and URLs using their default application.

| Platform | Supported |
| -------- | --------- |
| Linux    | ✓         |
| Windows  | ✓         |
| macOS    | ✓         |
| Android  | ✓         |
| iOS      | ✓         |
| OpenHarmony | ✓ (open is limited, see below) |

## Install

_This plugin requires a Rust version of at least **1.77.2**_

There are three general methods of installation that we can recommend.

1. Use crates.io and npm (easiest, and requires you to trust that our publishing pipeline worked)
2. Pull sources directly from Github using git tags / revision hashes (most secure)
3. Git submodule install this repo in your tauri project and then use file protocol to ingest the source (most secure, but inconvenient to use)

Install the Core plugin by adding the following to your `Cargo.toml` file:

`src-tauri/Cargo.toml`

```toml
[dependencies]
tauri-plugin-shell = "2.0.0"
# alternatively with Git:
tauri-plugin-shell = { git = "https://github.com/tauri-apps/plugins-workspace", branch = "v2" }
```

You can install the JavaScript Guest bindings using your preferred JavaScript package manager:

```sh
pnpm add @tauri-apps/plugin-shell
# or
npm add @tauri-apps/plugin-shell
# or
yarn add @tauri-apps/plugin-shell
```

## Usage

First you need to register the core plugin with Tauri:

`src-tauri/src/lib.rs`

```rust
fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

Afterwards all the plugin's APIs are available through the JavaScript guest bindings:

```javascript
import { Command } from '@tauri-apps/plugin-shell'
Command.create('git', ['commit', '-m', 'the commit message'])
```

## OpenHarmony (OHOS) Platform Notes

On OpenHarmony, the deprecated `open` API (JS `open()`, Rust `Shell::open`,
and the `_blank` link interception installed by this plugin's init script) is
routed through the **openharmony-ability URL bridge** — the same system link
opener used by `tauri-plugin-opener` — instead of the desktop Linux helpers
(`xdg-open`, `gio`, ...) which do not exist on OHOS. Consequences:

- `open('https://…')` resolves once the system link opener accepts the request;
  the browser (or other default handler) is brought to the foreground.
- `'open with'` (the `with` program argument) is unsupported and ignored —
  the system default application is always used.
- Absolute URLs are forwarded to the opener as-is. Non-URL arguments are
  treated as filesystem paths and canonicalized into `file://` URIs; a
  missing relative path fails with an IO error.
- The bridge requires a `://` scheme, so `mailto:`/`tel:` URLs — accepted by
  the default `plugins > shell > open` validation regex — are rejected by the
  bridge on OHOS with a platform error. Use `https://` URLs.
- The Rust `Shell::open` / `tauri_plugin_shell::open::open` functions are
  `async` on OHOS (bridge round-trip), unlike the synchronous desktop
  variants.

## Contributing

PRs accepted. Please make sure to read the Contributing Guide before making a pull request.

## Partners

<table>
  <tbody>
    <tr>
      <td align="center" valign="middle">
        <a href="https://crabnebula.dev" target="_blank">
          <img src="https://github.com/tauri-apps/plugins-workspace/raw/v2/.github/sponsors/crabnebula.svg" alt="CrabNebula" width="283">
        </a>
      </td>
    </tr>
  </tbody>
</table>

For the complete list of sponsors please visit our [website](https://tauri.app#sponsors) and [Open Collective](https://opencollective.com/tauri).

## License

Code: (c) 2015 - Present - The Tauri Programme within The Commons Conservancy.

MIT or MIT/Apache 2.0 where applicable.
