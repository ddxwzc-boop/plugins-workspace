![biometric](https://github.com/tauri-apps/plugins-workspace/raw/v2/plugins/biometric/banner.png)

Prompt the user for biometric authentication on Android, iOS and OpenHarmony.

| Platform | Supported |
| -------- | --------- |
| Linux    | x         |
| Windows  | x         |
| macOS    | x         |
| Android  | ✓         |
| iOS      | ✓         |
| OpenHarmony | ✓      |

## Install

_This plugin requires a Rust version of at least **1.65**_

There are three general methods of installation that we can recommend.

1. Use crates.io and npm (easiest, and requires you to trust that our publishing pipeline worked)
2. Pull sources directly from Github using git tags / revision hashes (most secure)
3. Git submodule install this repo in your tauri project and then use file protocol to ingest the source (most secure, but inconvenient to use)

Install the Core plugin by adding the following to your `Cargo.toml` file:

`src-tauri/Cargo.toml`

```toml
[dependencies]
tauri-plugin-biometric = "2.0.0"
# alternatively with Git:
tauri-plugin-biometric = { git = "https://github.com/tauri-apps/plugins-workspace", branch = "v2" }
```

You can install the JavaScript Guest bindings using your preferred JavaScript package manager:

<!-- Add the branch for installations using git! -->

```sh
pnpm add @tauri-apps/plugin-biometric
# or
npm add @tauri-apps/plugin-biometric
# or
yarn add @tauri-apps/plugin-biometric
```

## Usage

First you need to register the core plugin with Tauri:

`src-tauri/src/lib.rs`

```rust
fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_biometric::init())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

Afterwards all the plugin's APIs are available through the JavaScript guest bindings:

```javascript
import { authenticate } from '@tauri-apps/plugin-biometric'
await authenticate('Open your wallet')
```

### OpenHarmony

`authenticate` opens the system user authentication widget (face or fingerprint, whichever the device supports). A subset of `AuthOptions` is honored:

- `reason` — fallback for the dialog title.
- `title` — the dialog title (falls back to `reason`, then "Authentication"; max 500 characters).
- `cancelTitle` — maps to the widget's navigation button (max 60 characters); tapping it rejects with `userFallback`. Only honored when `allowDeviceCredential` is `false` — the passcode scene cannot carry a custom navigation button (the same mutual exclusion as Android's negative button).
- `allowDeviceCredential` — adds the lock-screen passcode to the widget's authentication factors: on biometric failure the widget shows a system button switching to passcode entry. On devices without biometric hardware this degrades to a passcode-only prompt; when no lock-screen passcode is set the call rejects with `passcodeNotSet`.

`fallbackTitle`, `subtitle`, `confirmationRequired` and `maxAttemps` have no counterpart in the OHOS widget (`WidgetParam` exposes only `title`, `navigationButtonText` and `uiContext`) and are ignored. `checkStatus` reports `isAvailable` plus `biometryType` (`1` fingerprint, `2` face).

## Contributing

PRs accepted. Please make sure to read the Contributing Guide before making a pull request.

## Contributed By

<table>
  <tbody>
    <tr>
      <td align="center" valign="middle">
        <a href="https://crabnebula.dev" target="_blank">
          <img src="contributors/crabnebula.svg" alt="CrabNebula" width="283">
        </a>
      </td>
      <td align="center" valign="middle">
        <a href="https://impierce.com" target="_blank">
            <img src="contributors/impierce.svg" alt="Impierce" width="283" height="90">
        </a>
      </td>
    </tr>
  </tbody>
</table>

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
