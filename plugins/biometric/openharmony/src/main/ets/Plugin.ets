import { userAuth } from '@kit.UserAuthenticationKit';
import { BusinessError } from '@kit.BasicServicesKit';
import { Plugin, Invoke } from '@tauri/app';

// ─── Types ───

interface AuthArgs {
  reason: string;
  allowDeviceCredential: boolean;
  cancelTitle: string | null;
  fallbackTitle: string | null;
  title: string | null;
  subtitle: string | null;
  confirmationRequired: boolean | null;
}

interface StatusResult {
  isAvailable: boolean;
  biometryType: number; // 0=None, 1=TouchID (fingerprint), 2=FaceID (face)
  error: string | null;
  errorCode: string | null;
}

// ─── Plugin Implementation ───

export class BiometricPlugin extends Plugin {
  getCommands(): Map<string, (invoke: Invoke) => void> {
    const commands: Map<string, (invoke: Invoke) => void> = new Map();
    commands.set('status', (invoke: Invoke): void => { this.handleStatus(invoke); });
    commands.set('authenticate', (invoke: Invoke): void => { this.handleAuthenticate(invoke); });
    return commands;
  }

  // ─── shared availability probe ───

  /**
   * Probes which biometric type the device supports at ATL1 trust level
   * (FACE preferred, then FINGERPRINT). Returns null when neither is
   * available. Shared by doStatus (biometryType mapping) and doAuthenticate
   * (widget authType selection) so the two probes cannot drift apart.
   */
  private probeAuthType(): userAuth.UserAuthType | null {
    const isSupported = (authType: userAuth.UserAuthType): boolean => {
      try {
        userAuth.getAvailableStatus(authType, userAuth.AuthTrustLevel.ATL1);
        return true;
      } catch (e) {
        return false;
      }
    };
    if (isSupported(userAuth.UserAuthType.FACE)) {
      return userAuth.UserAuthType.FACE;
    }
    if (isSupported(userAuth.UserAuthType.FINGERPRINT)) {
      return userAuth.UserAuthType.FINGERPRINT;
    }
    return null;
  }

  // ─── status ───

  private handleStatus(invoke: Invoke): void {
    this.doStatus(invoke);
  }

  private async doStatus(invoke: Invoke): Promise<void> {
    const result: StatusResult = {
      isAvailable: false,
      biometryType: 0,
      error: null,
      errorCode: null
    };

    const authType = this.probeAuthType();
    if (authType === userAuth.UserAuthType.FACE) {
      result.isAvailable = true;
      result.biometryType = 2; // FaceID
    } else if (authType === userAuth.UserAuthType.FINGERPRINT) {
      result.isAvailable = true;
      result.biometryType = 1; // TouchID (fingerprint)
    } else {
      result.isAvailable = false;
      result.biometryType = 0;
      result.error = 'biometryNotAvailable';
      result.errorCode = 'biometryNotAvailable';
    }

    invoke.resolve(JSON.stringify(result));
  }

  // ─── authenticate ───

  private handleAuthenticate(invoke: Invoke): void {
    this.doAuthenticate(invoke);
  }

  private async doAuthenticate(invoke: Invoke): Promise<void> {
    try {
      const args: AuthArgs = JSON.parse(invoke.parseArgs()) as AuthArgs;

      // Widget auth scenes are enumerated by the system (the
      // start-authentication guide, "unified user-authentication widget"): {PIN}, {FACE},
      // {FINGERPRINT}, {PIN,FACE}, {PIN,FINGERPRINT}, {PIN,FACE,FINGERPRINT},
      // plus navigation-button scenes {FACE+nav}, {FINGERPRINT+nav},
      // {FACE,FINGERPRINT+nav} (API 18+). A bare [FACE, FINGERPRINT] pair is
      // NOT a scene — the historical "authType check fail:0" → ret:8 (12500008)
      // rejection. PIN+biometric IS supported: on biometric failure the widget
      // shows a system button switching to the lock-screen passcode (the
      // allowDeviceCredential semantic).
      const biometricType = this.probeAuthType();
      let authTypes: userAuth.UserAuthType[];
      let pinOnly = false;
      if (args.allowDeviceCredential === true) {
        authTypes = biometricType != null
          ? [userAuth.UserAuthType.PIN, biometricType]
          : [userAuth.UserAuthType.PIN]; // no biometric hardware → passcode-only prompt
        pinOnly = biometricType == null;
      } else {
        if (biometricType == null) {
          invoke.reject('biometryNotAvailable', 'biometryNotAvailable');
          return;
        }
        authTypes = [biometricType];
      }

      const authParam: userAuth.AuthParam = {
        challenge: new Uint8Array([0]),
        authType: authTypes,
        authTrustLevel: userAuth.AuthTrustLevel.ATL1
      };

      // Widget auth (API 18+) needs widgetParam.uiContext to display the auth
      // dialog in modal-application mode bound to this Ability's window. Without
      // it the system logs "BeginWidgetAuth has context: 0 / AuthWidget fail
      // ret:8" and no popup appears. UIAbilityContext extends Context, so
      // this.context (set by PluginManager.setContext) satisfies the
      // widgetParam.uiContext?: Context field directly.
      if (!this.context) {
        invoke.reject('authenticate failed: Plugin context not initialized');
        return;
      }

      const widgetParam: userAuth.WidgetParam = {
        title: args.title != null ? args.title : (args.reason != null ? args.reason : 'Authentication'),
        uiContext: this.context
      };
      // cancelTitle → the widget's navigation button (tapping it rejects with
      // 'userFallback' via 12500011), capped at the documented 60 characters.
      // Mutually exclusive with allowDeviceCredential: the passcode scene
      // cannot carry a custom navigation button ("lock-screen passcode auth
      // and custom business auth are mutually exclusive",
      // apply-custom-authentication) — the same mutex as Android's
      // BiometricActivity, which only sets the negative button when
      // allowDeviceCredential is false.
      if (args.cancelTitle != null && args.allowDeviceCredential !== true && args.cancelTitle.length > 0) {
        widgetParam.navigationButtonText = args.cancelTitle.slice(0, 60);
      }

      const authInstance = userAuth.getUserAuthInstance(authParam, widgetParam);

      // IAuthCallback is an interface (onResult method), not a function type —
      // see @ohos.userIAM.userAuth.d.ts. There is no 'error' event; async
      // failures (cancel, lockout, …) arrive here via result.result, and
      // synchronous errors from start() are caught by the outer try/catch.
      authInstance.on('result', {
        onResult: (result: userAuth.UserAuthResult): void => {
          if (result.result === userAuth.UserAuthResultCode.SUCCESS) {
            invoke.resolve('');
          } else {
            // Machine-readable code (userCancel/biometryLockout/…) goes in the
            // reject code channel — the Rust ErrorResponse surfaces it — not
            // concatenated into the human message. In the passcode-only scene
            // NOT_ENROLLED means no lock-screen passcode is set — surface the
            // cross-platform 'passcodeNotSet' vocabulary instead of the
            // biometric-specific mapping.
            const errorCode = pinOnly && result.result === userAuth.UserAuthResultCode.NOT_ENROLLED
              ? 'passcodeNotSet'
              : this.mapResultCode(result.result);
            invoke.reject('Authentication failed', errorCode);
          }
        }
      });

      authInstance.start();
    } catch (e) {
      const err = e as BusinessError;
      console.error('[BiometricPlugin] authenticate failed: code=' + err.code + ', message=' + err.message);
      // Keep the reject-code vocabulary consistent with the onResult path for
      // userAuth-range BusinessErrors; other thrown errors pass their numeric
      // code through raw (no code at all → 'authenticationFailed').
      let code: string = 'authenticationFailed';
      if (err.code != null) {
        code = err.code >= 12500000 && err.code <= 12599999
          ? this.mapResultCode(err.code)
          : String(err.code);
      }
      invoke.reject('authenticate failed: ' + err.message, code);
    }
  }

  private mapResultCode(code: number): string {
    // Maps OHOS UserAuthResultCode (js-apis-useriam-userauth) onto the shared
    // Tauri errorCode vocabulary — the same strings the Android plugin's
    // biometryErrorCodeMap produces and the guest-js Status.errorCode union
    // declares. Numeric values per the official enum table:
    //   FAIL 12500001, GENERAL_ERROR 12500002, CANCELED 12500003,
    //   TIMEOUT 12500004, TYPE_NOT_SUPPORT 12500005,
    //   TRUST_LEVEL_NOT_SUPPORT 12500006, BUSY 12500007, LOCKED 12500009,
    //   NOT_ENROLLED 12500010, CANCELED_FROM_WIDGET 12500011.
    if (code === userAuth.UserAuthResultCode.SUCCESS) {
      return 'success';
    }
    switch (code) {
      case userAuth.UserAuthResultCode.FAIL:
      case userAuth.UserAuthResultCode.GENERAL_ERROR:
        return 'authenticationFailed';
      case userAuth.UserAuthResultCode.CANCELED:
        return 'userCancel';
      case userAuth.UserAuthResultCode.TIMEOUT:
      case userAuth.UserAuthResultCode.BUSY:
        return 'systemCancel';
      case userAuth.UserAuthResultCode.TYPE_NOT_SUPPORT:
      case userAuth.UserAuthResultCode.TRUST_LEVEL_NOT_SUPPORT:
        return 'biometryNotAvailable';
      case userAuth.UserAuthResultCode.LOCKED:
        return 'biometryLockout';
      case userAuth.UserAuthResultCode.NOT_ENROLLED:
        return 'biometryNotEnrolled';
      case userAuth.UserAuthResultCode.CANCELED_FROM_WIDGET:
        // The user tapped the widget's navigation button, choosing the app's
        // own auth flow — the semantic of iOS fallbackTitle / Android
        // cancelTitle.
        return 'userFallback';
      default:
        // INVALID_PARAMETERS (12500008, API 20+), PIN_EXPIRED (12500013,
        // API 12+), and any future codes.
        return 'authenticationFailed';
    }
  }
}

export default BiometricPlugin;
