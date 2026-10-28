// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

import { invoke } from '@tauri-apps/api/core'

export enum BiometryType {
  None = 0,
  // Apple TouchID or Android fingerprint
  TouchID = 1,
  // Apple FaceID or Android face authentication
  FaceID = 2,
  // Android iris authentication
  Iris = 3
}

export interface Status {
  isAvailable: boolean
  biometryType: BiometryType
  error?: string
  errorCode?:
    | 'appCancel'
    | 'authenticationFailed'
    | 'invalidContext'
    | 'notInteractive'
    | 'passcodeNotSet'
    | 'systemCancel'
    | 'userCancel'
    | 'userFallback'
    | 'biometryLockout'
    | 'biometryNotAvailable'
    | 'biometryNotEnrolled'
}

export interface AuthOptions {
  /**
   * Also allow the device credential (PIN/pattern/password) as an
   * authentication factor.
   *
   * #### Platform-specific:
   * - **OpenHarmony:** supported — the lock-screen passcode is added to the
   *   system auth widget's factors; on biometric failure the widget shows a
   *   system button switching to passcode entry. Degrades to a passcode-only
   *   prompt on devices without biometric hardware; rejects with
   *   `passcodeNotSet` when no lock-screen passcode is set. Mutually
   *   exclusive with `cancelTitle` (the passcode scene cannot carry a custom
   *   navigation button).
   */
  allowDeviceCredential?: boolean
  /**
   * Title of the cancel button.
   *
   * #### Platform-specific:
   * - **OpenHarmony:** maps to the auth widget's navigation button (max 60
   *   characters); tapping it rejects with `userFallback`. Only honored when
   *   `allowDeviceCredential` is `false` — the same mutual exclusion as
   *   Android's negative button.
   */
  cancelTitle?: string

  // iOS options
  /**
   * The "Enter Password" fallback title. On OpenHarmony there is no
   * customizable fallback text: the passcode-switch button is provided
   * automatically when `allowDeviceCredential` is `true`.
   */
  fallbackTitle?: string

  // android options
  /**
   * The prompt title. On OpenHarmony it is the dialog title (falls back to
   * `reason`, then "Authentication"; max 500 characters).
   */
  title?: string
  /** Android-only; ignored on OpenHarmony (the system widget has no subtitle). */
  subtitle?: string
  /** Android-only; ignored on OpenHarmony. */
  confirmationRequired?: boolean
  /** Android-only; ignored on OpenHarmony (the system widget manages its own lockout policy). */
  maxAttemps?: number
}

/**
 * Checks if the biometric authentication is available.
 * @returns a promise resolving to an object containing all the information about the status of the biometry.
 */
export async function checkStatus(): Promise<Status> {
  return await invoke('plugin:biometric|status')
}

/**
 * Prompts the user for authentication using the system interface (touchID, faceID or Android Iris).
 * Rejects if the authentication fails.
 *
 * ```javascript
 * import { authenticate } from "@tauri-apps/plugin-biometric";
 * await authenticate('Open your wallet');
 * ```
 * @param reason
 * @param options
 * @returns
 */
export async function authenticate(
  reason: string,
  options?: AuthOptions
): Promise<void> {
  await invoke('plugin:biometric|authenticate', {
    reason,
    ...options
  })
}
