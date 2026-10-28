![haptics](https://github.com/tauri-apps/plugins-workspace/raw/v2/plugins/haptics/banner.png)

Haptic feedback and vibrations on Android, iOS and OpenHarmony.

There are no standards/requirements for vibration support on Android, so the `feedback` APIs may not work correctly on more affordable phones, including recently released ones.

| Platform | Supported |
| -------- | --------- |
| Linux    | x         |
| Windows  | x         |
| macOS    | x         |
| Android  | ✓         |
| iOS      | ✓         |
| OpenHarmony | ✓      |

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
tauri-plugin-haptics = "2.0.0"
# alternatively with Git:
tauri-plugin-haptics = { git = "https://github.com/tauri-apps/plugins-workspace", branch = "v2" }
```

You can install the JavaScript Guest bindings using your preferred JavaScript package manager:

<!-- Add the branch for installations using git! -->

```sh
pnpm add @tauri-apps/plugin-haptics
# or
npm add @tauri-apps/plugin-haptics
# or
yarn add @tauri-apps/plugin-haptics
```

## Usage

First you need to register the core plugin with Tauri:

`src-tauri/src/lib.rs`

```rust
fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_haptics::init())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

Second, add the required permissions in the project:

`src-tauri/capabilities/default.json`

```json
  "permissions": [
    "haptics:allow-impact-feedback",
    "haptics:allow-notification-feedback",
    "haptics:allow-selection-feedback",
    "haptics:allow-vibrate"
  ]
```

Afterwards all the plugin's APIs are available through the JavaScript guest bindings:

```javascript
import {
  vibrate,
  impactFeedback,
  notificationFeedback,
  selectionFeedback
} from '@tauri-apps/plugin-haptics'

await vibrate(1)
await impactFeedback('medium')
await notificationFeedback('warning')
await selectionFeedback()
```

## Platform Notes

### OpenHarmony

Feedback APIs are mapped onto the system preset vibration effects of `@ohos.vibrator` (`VibratePreset`):

| API | Argument | Effect ID | Intensity |
| --- | --- | --- | --- |
| `impactFeedback` | `light`, `soft` | `haptic.effect.soft` | 50 |
| `impactFeedback` | `medium` | `haptic.effect.hard` | 50 |
| `impactFeedback` | `heavy` | `haptic.effect.hard` | 100 |
| `impactFeedback` | `rigid` | `haptic.effect.sharp` | 50 |
| `notificationFeedback` | `success` | `haptic.notice.success` | 60 |
| `notificationFeedback` | `warning` | `haptic.notice.warning` | 60 |
| `notificationFeedback` | `error` | `haptic.notice.fail` | 60 |
| `selectionFeedback` | - | `haptic.effect.soft` | 40 |
| `vibrate` | duration (ms) | time-based (`VibrateTime`) | - |

- `intensity` is the OpenHarmony preset vibration strength (`VibratePreset.intensity`, API 12+). Its documented value range is all integers in `(0, 100]` with a default of `100`. `medium` and `heavy` share the `haptic.effect.hard` preset and are separated only by this strength value (50 vs 100). According to the official documentation, when the vibration effect or the device does not support intensity adjustment the motor vibrates at the default strength — the call still succeeds, but the two styles may feel identical on such devices.
- The `haptic.notice.*` presets are only available from API 18+. When a device does not support a preset (checked via `vibrator.isSupportEffect`), the plugin falls back to a short time-based vibration (30 ms for impact, 50 ms for notification, 15 ms for selection).
- Devices without a vibration motor reject feedback calls: `vibrator.startVibration` throws `BusinessError` 801 ("Capability not supported."), which surfaces as a rejected promise (e.g. `impactFeedback failed: ...`).

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
        <a href="https://rescue.co" target="_blank">
            <img src="contributors/rescue.png" alt="Rescue.co" width="283" height="90">
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
