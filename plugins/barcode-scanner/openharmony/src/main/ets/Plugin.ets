import { scanBarcode, scanCore } from '@kit.ScanKit';
import { vibrator } from '@kit.SensorServiceKit';
import { abilityAccessCtrl, bundleManager, Permissions, Want } from '@kit.AbilityKit';
import { BusinessError } from '@kit.BasicServicesKit';
import { Plugin, Invoke } from '@tauri/app';

// ─── Types ───

interface ScanOptions {
  cameraDirection: string | null;
  formats: string[] | null;
  windowed: boolean | null;
}

interface Scanned {
  content: string;
  format: string;
  bounds: Record<string, Object> | null;
}

// ─── Plugin Implementation ───

const PERMISSION_CAMERA: Permissions = 'ohos.permission.CAMERA';

function permissionStateToString(state: number): string {
  // abilityAccessCtrl.GrantStatus / authResults: 0 = granted, -1 = denied
  if (state === 0) {
    return 'granted';
  }
  if (state === -1) {
    return 'denied';
  }
  return 'prompt';
}

// ─── Format mapping (Tauri Format string ⇄ OHOS scanCore.ScanType) ───

// Maps a Tauri guest-js `Format` enum value to the ScanType used to filter
// the system scan page. Returns null for formats Scan Kit cannot filter on
// (GS1_DATA_BAR*) — the caller skips them, mirroring the Android plugin's
// supportedFormats() lookup. 'ITF' maps to ITF14_CODE: Scan Kit has no
// plain-ITF code type.
function formatToScanType(format: string): scanCore.ScanType | null {
  switch (format) {
    case 'QR_CODE':
      return scanCore.ScanType.QR_CODE;
    case 'UPC_A':
      return scanCore.ScanType.UPC_A_CODE;
    case 'UPC_E':
      return scanCore.ScanType.UPC_E_CODE;
    case 'EAN_8':
      return scanCore.ScanType.EAN8_CODE;
    case 'EAN_13':
      return scanCore.ScanType.EAN13_CODE;
    case 'CODE_39':
      return scanCore.ScanType.CODE39_CODE;
    case 'CODE_93':
      return scanCore.ScanType.CODE93_CODE;
    case 'CODE_128':
      return scanCore.ScanType.CODE128_CODE;
    case 'CODABAR':
      return scanCore.ScanType.CODABAR_CODE;
    case 'ITF':
      return scanCore.ScanType.ITF14_CODE;
    case 'AZTEC':
      return scanCore.ScanType.AZTEC_CODE;
    case 'DATA_MATRIX':
      return scanCore.ScanType.DATAMATRIX_CODE;
    case 'PDF_417':
      return scanCore.ScanType.PDF417_CODE;
    default:
      return null;
  }
}

// Reverse direction: ScanResult.scanType → Tauri `Format` enum value, so
// Scanned.format matches the guest-js type instead of leaking the numeric
// enum value (e.g. '11' for QR_CODE). Types without a Tauri counterpart
// (MULTIFUNCTIONAL_CODE, ONE_D_CODE, …) fall back to the raw string.
function scanTypeToFormat(scanType: scanCore.ScanType): string {
  switch (scanType) {
    case scanCore.ScanType.QR_CODE:
      return 'QR_CODE';
    case scanCore.ScanType.UPC_A_CODE:
      return 'UPC_A';
    case scanCore.ScanType.UPC_E_CODE:
      return 'UPC_E';
    case scanCore.ScanType.EAN8_CODE:
      return 'EAN_8';
    case scanCore.ScanType.EAN13_CODE:
      return 'EAN_13';
    case scanCore.ScanType.CODE39_CODE:
      return 'CODE_39';
    case scanCore.ScanType.CODE93_CODE:
      return 'CODE_93';
    case scanCore.ScanType.CODE128_CODE:
      return 'CODE_128';
    case scanCore.ScanType.CODABAR_CODE:
      return 'CODABAR';
    case scanCore.ScanType.ITF14_CODE:
      return 'ITF';
    case scanCore.ScanType.AZTEC_CODE:
      return 'AZTEC';
    case scanCore.ScanType.DATAMATRIX_CODE:
      return 'DATA_MATRIX';
    case scanCore.ScanType.PDF417_CODE:
      return 'PDF_417';
    default:
      return String(scanType);
  }
}

export class BarcodeScannerPlugin extends Plugin {
  // ─── Permission request fallback state ───
  // Tracks a pending requestPermissionsFromUser call so that it can be
  // settled via one of three paths if the original Promise never resolves
  // (system permission dialog causes ability onBackground → event loop freeze).
  private pendingPermissionInvoke: Invoke | null = null;
  private permissionSettled: boolean = false;
  private permissionStateCallback: ((info: abilityAccessCtrl.PermissionStateChangeInfo) => void) | null = null;
  private permissionStateList: Array<Permissions> = [];
  private permissionTimeoutId: number = -1;
  getCommands(): Map<string, (invoke: Invoke) => void> {
    const commands: Map<string, (invoke: Invoke) => void> = new Map();
    commands.set('scan', (invoke: Invoke): void => { this.handleScan(invoke); });
    commands.set('cancel', (invoke: Invoke): void => { this.handleCancel(invoke); });
    commands.set('checkPermissions', (invoke: Invoke): void => { this.handleCheckPermissions(invoke); });
    commands.set('requestPermissions', (invoke: Invoke): void => { this.handleRequestPermissions(invoke); });
    commands.set('openAppSettings', (invoke: Invoke): void => { this.handleOpenAppSettings(invoke); });
    commands.set('vibrate', (invoke: Invoke): void => { this.handleVibrate(invoke); });
    return commands;
  }

  /**
   * Called by PluginManager.notifyForeground() when the host ability returns
   * to the foreground. If there is a pending permission request that hasn't
   * been settled yet, settle it now by reading the real permission state.
   */
  onForeground(): void {
    if (this.pendingPermissionInvoke != null && !this.permissionSettled) {
      this.settlePermissions('onForeground');
    }
  }

  // ─── scan ───

  private handleScan(invoke: Invoke): void {
    this.doScan(invoke);
  }

  private async doScan(invoke: Invoke): Promise<void> {
    try {
      // scanBarcode.startScanForResult requires a UIAbilityContext.
      // The mobile plugin Plugin base class does not reliably expose context,
      // so this may reject on configurations where context is unavailable.
      const options: ScanOptions = JSON.parse(invoke.parseArgs()) as ScanOptions;

      // Honor the caller's format filter: map known Format strings to Scan
      // Kit types and skip unknown ones (Android behavior); no formats (or
      // none that Scan Kit supports) falls back to ALL. cameraDirection and
      // windowed have no Scan Kit equivalent — the default UI is always
      // fullscreen on the back camera (documented in the README).
      const scanTypes: Array<scanCore.ScanType> = [];
      if (options.formats != null) {
        for (const format of options.formats) {
          const scanType = formatToScanType(format);
          if (scanType != null) {
            scanTypes.push(scanType);
          }
        }
      }
      if (scanTypes.length === 0) {
        scanTypes.push(scanCore.ScanType.ALL);
      }

      const scanOptions: scanBarcode.ScanOptions = {
        scanTypes: scanTypes,
        enableMultiMode: false,
        enableAlbum: true
      };

      // startScanForResult needs a context; the Plugin base class exposes it
      // via this.context (same pattern as notification's requestEnableNotification).
      if (this.context == null) {
        invoke.reject('barcode scan requires a UIAbilityContext which is not available from the plugin. Request from the Ability layer.');
        return;
      }

      const result = await scanBarcode.startScanForResult(this.context, scanOptions);
      const scanned: Scanned = {
        content: result.originalValue,
        format: scanTypeToFormat(result.scanType),
        bounds: null
      };
      invoke.resolve(JSON.stringify(scanned));
    } catch (e) {
      const err = e as BusinessError;
      console.error('[BarcodeScannerPlugin] scan failed: code=' + err.code + ', message=' + err.message);
      invoke.reject('scan failed: ' + err.message);
    }
  }

  // ─── cancel ───

  private handleCancel(invoke: Invoke): void {
    // OHOS scanBarcode default UI is a system scan page; it auto-closes on
    // scan success or back press. There is no programmatic cancel API.
    invoke.resolve('');
  }

  // ─── checkPermissions ───

  private handleCheckPermissions(invoke: Invoke): void {
    this.doCheckPermissions(invoke);
  }

  private async doCheckPermissions(invoke: Invoke): Promise<void> {
    let state = 'prompt';
    try {
      const info = bundleManager.getBundleInfoForSelfSync(
        bundleManager.BundleFlag.GET_BUNDLE_INFO_WITH_APPLICATION
      );
      const token = info.appInfo.accessTokenId;
      const mgr = abilityAccessCtrl.createAtManager();
      state = permissionStateToString(mgr.checkAccessTokenSync(token, PERMISSION_CAMERA));
    } catch (e) {
      console.warn('[BarcodeScannerPlugin] checkPermissions failed: ' + (e as BusinessError).message);
    }
    invoke.resolve(JSON.stringify({ camera: state }));
  }

  // ─── requestPermissions ───

  private handleRequestPermissions(invoke: Invoke): void {
    this.doRequestPermissions(invoke);
  }

  private async doRequestPermissions(invoke: Invoke): Promise<void> {
    // requestPermissionsFromUser requires a UIAbilityContext.
    if (this.context == null) {
      invoke.resolve(JSON.stringify({ camera: 'prompt' }));
      return;
    }
    this.requestPermissionsWithFallback(invoke);
  }

  /**
   * Three-path permission request with fallback settlement:
   *   Path 1 (onForeground): ability returns to foreground after dialog → settle.
   *   Path 2 (selfPermissionStateChange event): system fires permission state
   *       change callback → settle.
   *   Path 3 (setTimeout 60s): safety net for foreground-only scenarios → settle.
   *   Original Promise .then/.catch also settles (if it ever resolves).
   * All settlement paths read the real permission state via
   * queryCameraPermissionStatus(), not the possibly-stale authResults array.
   */
  private requestPermissionsWithFallback(invoke: Invoke): void {
    // Initialize pending state
    this.pendingPermissionInvoke = invoke;
    this.permissionSettled = false;
    this.permissionStateList = [PERMISSION_CAMERA];

    // Path 2: Register selfPermissionStateChange listener (API 18+).
    // Wrap in try/catch — duplicate registration throws 12100004.
    try {
      const mgr = abilityAccessCtrl.createAtManager();
      const callback = (info: abilityAccessCtrl.PermissionStateChangeInfo): void => {
        this.settlePermissions('selfPermissionStateChange');
      };
      this.permissionStateCallback = callback;
      mgr.on('selfPermissionStateChange', this.permissionStateList, callback);
    } catch (e) {
      console.warn('[BARCODE] on selfPermissionStateChange failed: ' + (e as BusinessError).message);
      this.permissionStateCallback = null;
    }

    // Path 3: setTimeout 60s safety net (only effective while in foreground).
    this.permissionTimeoutId = setTimeout((): void => {
      console.warn('[BARCODE] permission request safety-net timeout (60s) fired; settling with polled state');
      this.settlePermissions('timeout');
    }, 60000);

    // Original path: fire requestPermissionsFromUser as fire-and-forget.
    // If the Promise never resolves (event loop frozen during dialog),
    // the other paths handle settlement.
    this.firePermissionRequest();
  }

  private async firePermissionRequest(): Promise<void> {
    try {
      const mgr = abilityAccessCtrl.createAtManager();
      await mgr.requestPermissionsFromUser(this.context!, [PERMISSION_CAMERA]);
      this.settlePermissions('promise-resolved');
    } catch (e) {
      console.warn('[BARCODE] promise rejected: ' + (e as BusinessError).message);
      this.settlePermissions('promise-rejected');
    }
  }

  /**
   * Settles the pending permission request (if any) using the real permission
   * state read via queryCameraPermissionStatus(). Idempotent — the `settled`
   * flag ensures only the first path to call this actually resolves the invoke.
   */
  private settlePermissions(source: string): void {
    if (this.permissionSettled) {
      return;
    }
    this.permissionSettled = true;

    // Cleanup listeners and timeout
    this.cleanupPermissionListeners();

    // Resolve with current permission status (reads real state, not cached authResults).
    // selfPermissionStateChange can fire BEFORE the grant is committed and visible
    // to checkAccessTokenSync, so poll for a short window instead of a single read.
    const invoke = this.pendingPermissionInvoke;
    this.pendingPermissionInvoke = null;
    if (invoke != null) {
      this.resolveWithPolledStatus(invoke, 0);
    }
  }

  /**
   * Resolves the pending permission invoke with a polled permission read.
   * selfPermissionStateChange can fire BEFORE the grant is committed and
   * visible to checkAccessTokenSync (observed on device: event change=1 but
   * sync query still 'denied'). First read is immediate (fast path: already
   * granted resolves with no delay); if the camera permission is not yet
   * granted, re-read every 300ms up to 6 retries (~1.8s), then resolve with
   * the last read regardless.
   */
  private resolveWithPolledStatus(invoke: Invoke, attempt: number): void {
    const state = this.queryCameraPermissionStatus();
    const allGranted: boolean = state === 'granted';
    if (allGranted || attempt >= 6) {
      invoke.resolve(JSON.stringify({ camera: state }));
      return;
    }
    setTimeout((): void => {
      this.resolveWithPolledStatus(invoke, attempt + 1);
    }, 300);
  }

  private cleanupPermissionListeners(): void {
    // off the selfPermissionStateChange listener (must pass same callback + permissionList)
    if (this.permissionStateCallback != null) {
      try {
        const mgr = abilityAccessCtrl.createAtManager();
        mgr.off('selfPermissionStateChange', this.permissionStateList, this.permissionStateCallback);
      } catch (e) {
        console.warn('[BARCODE] off selfPermissionStateChange failed: ' + (e as BusinessError).message);
      }
      this.permissionStateCallback = null;
    }
    // clear the timeout
    if (this.permissionTimeoutId !== -1) {
      clearTimeout(this.permissionTimeoutId);
      this.permissionTimeoutId = -1;
    }
  }

  /**
   * Reads the real camera permission state from the system. Used by both
   * checkPermissions and the fallback settlement paths.
   */
  private queryCameraPermissionStatus(): string {
    let state = 'prompt';
    try {
      const info = bundleManager.getBundleInfoForSelfSync(
        bundleManager.BundleFlag.GET_BUNDLE_INFO_WITH_APPLICATION
      );
      const token = info.appInfo.accessTokenId;
      const mgr = abilityAccessCtrl.createAtManager();
      state = permissionStateToString(mgr.checkAccessTokenSync(token, PERMISSION_CAMERA));
    } catch (e) {
      console.warn('[BarcodeScannerPlugin] camera permission query failed: ' + (e as BusinessError).message);
    }
    return state;
  }

  // ─── openAppSettings ───

  private handleOpenAppSettings(invoke: Invoke): void {
    this.doOpenAppSettings(invoke);
  }

  private async doOpenAppSettings(invoke: Invoke): Promise<void> {
    if (this.context == null) {
      invoke.reject('openAppSettings requires a UIAbilityContext which is not available from the plugin.');
      return;
    }
    try {
      const want: Want = {
        action: 'action.settings.applications.info',
        parameters: {}
      };
      await this.context.startAbility(want);
      invoke.resolve('');
    } catch (e) {
      console.error('[BarcodeScannerPlugin] openAppSettings failed: ' + (e as BusinessError).message);
      invoke.reject('openAppSettings failed: ' + (e as BusinessError).message);
    }
  }

  // ─── vibrate ───

  private handleVibrate(invoke: Invoke): void {
    this.doVibrate(invoke);
  }

  private async doVibrate(invoke: Invoke): Promise<void> {
    try {
      await vibrator.startVibration(
        { type: 'time', duration: 100 } as vibrator.VibrateEffect,
        { usage: 'unknown' } as vibrator.VibrateAttribute
      );
      invoke.resolve('');
    } catch (e) {
      const err = e as BusinessError;
      console.error('[BarcodeScannerPlugin] vibrate failed: ' + err.message);
      invoke.reject('vibrate failed: ' + err.message);
    }
  }
}

export default BarcodeScannerPlugin;
