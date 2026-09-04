import { vibrator } from '@kit.SensorServiceKit';
import { BusinessError } from '@kit.BasicServicesKit';
import { Plugin, Invoke } from '@tauri/app';

// ─── Types ───

interface VibrateArgs {
  duration: number;
}

interface ImpactFeedbackArgs {
  style: string;
}

interface NotificationFeedbackArgs {
  type: string;
}

// ─── Plugin Implementation ───

// Maps Tauri ImpactFeedbackStyle (light/medium/heavy/soft/rigid) to OHOS
// preset vibration effect IDs.
//
// The documented HapticFeedback presets (js-apis-vibrator) are only
// 'haptic.effect.soft' (long, gentle, low-frequency), 'haptic.effect.hard'
// (short, thick, strong) and 'haptic.effect.sharp' (short, sharp,
// high-frequency) — there are no light/medium/heavy/rigid ids, so styles map
// onto the closest documented preset rather than onto invented ids
// (isSupportEffect would return false for those and every call would
// silently degrade to the plain time-based fallback vibration).
function impactStyleToEffectId(style: string): string {
  switch (style) {
    case 'light':
    case 'soft':
      return 'haptic.effect.soft';
    case 'medium':
    case 'heavy':
      return 'haptic.effect.hard';
    case 'rigid':
      return 'haptic.effect.sharp';
    default:
      return 'haptic.effect.soft';
  }
}

// Maps Tauri NotificationFeedbackType (success/warning/error) to the OHOS
// notification feedback effect IDs (API 18+). On devices below API 18 — or
// without these presets — vibratePresetWithFallback falls back to a short
// time-based vibration.
function notificationTypeToEffectId(type: string): string {
  switch (type) {
    case 'success':
      return 'haptic.notice.success';
    case 'warning':
      return 'haptic.notice.warning';
    case 'error':
      return 'haptic.notice.fail';
    default:
      return 'haptic.notice.success';
  }
}

export class HapticsPlugin extends Plugin {
  getCommands(): Map<string, (invoke: Invoke) => void> {
    const commands: Map<string, (invoke: Invoke) => void> = new Map();
    commands.set('vibrate', (invoke: Invoke): void => { this.handleVibrate(invoke); });
    commands.set('impactFeedback', (invoke: Invoke): void => { this.handleImpactFeedback(invoke); });
    commands.set('notificationFeedback', (invoke: Invoke): void => { this.handleNotificationFeedback(invoke); });
    commands.set('selectionFeedback', (invoke: Invoke): void => { this.handleSelectionFeedback(invoke); });
    return commands;
  }

  // ─── vibrate (duration in ms) ───

  private handleVibrate(invoke: Invoke): void {
    this.doVibrate(invoke);
  }

  private async doVibrate(invoke: Invoke): Promise<void> {
    try {
      const args: VibrateArgs = JSON.parse(invoke.parseArgs()) as VibrateArgs;
      // 300ms default matches the Android plugin.
      const duration = args.duration > 0 ? args.duration : 300;
      await vibrator.startVibration(
        { type: 'time', duration: duration } as vibrator.VibrateEffect,
        { usage: 'unknown' } as vibrator.VibrateAttribute
      );
      invoke.resolve('');
    } catch (e) {
      const err = e as BusinessError;
      console.error('[HapticsPlugin] vibrate failed: code=' + err.code + ', message=' + err.message);
      invoke.reject('vibrate failed: ' + err.message);
    }
  }

  // ─── shared preset-vibration helper ───

  /**
   * Vibrates with a preset effect, falling back to a short time-based
   * vibration when the device does not support the effect id — preset
   * support varies per device, and the haptic.notice.* presets require
   * API 18+. Throws on startVibration failure; callers surface it via
   * invoke.reject.
   */
  private async vibratePresetWithFallback(effectId: string, intensity: number, fallbackDuration: number): Promise<void> {
    const supported = await vibrator.isSupportEffect(effectId);
    if (!supported) {
      await vibrator.startVibration(
        { type: 'time', duration: fallbackDuration } as vibrator.VibrateEffect,
        { usage: 'unknown' } as vibrator.VibrateAttribute
      );
      return;
    }
    await vibrator.startVibration(
      { type: 'preset', effectId: effectId, count: 1, intensity: intensity } as vibrator.VibrateEffect,
      { usage: 'unknown' } as vibrator.VibrateAttribute
    );
  }

  // ─── impactFeedback (style) ───

  private handleImpactFeedback(invoke: Invoke): void {
    this.doImpactFeedback(invoke);
  }

  private async doImpactFeedback(invoke: Invoke): Promise<void> {
    try {
      const args: ImpactFeedbackArgs = JSON.parse(invoke.parseArgs()) as ImpactFeedbackArgs;
      const effectId = impactStyleToEffectId(args.style);
      await this.vibratePresetWithFallback(effectId, 50, 30);
      invoke.resolve('');
    } catch (e) {
      const err = e as BusinessError;
      console.error('[HapticsPlugin] impactFeedback failed: ' + err.message);
      invoke.reject('impactFeedback failed: ' + err.message);
    }
  }

  // ─── notificationFeedback (type) ───

  private handleNotificationFeedback(invoke: Invoke): void {
    this.doNotificationFeedback(invoke);
  }

  private async doNotificationFeedback(invoke: Invoke): Promise<void> {
    try {
      const args: NotificationFeedbackArgs = JSON.parse(invoke.parseArgs()) as NotificationFeedbackArgs;
      const effectId = notificationTypeToEffectId(args.type);
      await this.vibratePresetWithFallback(effectId, 60, 50);
      invoke.resolve('');
    } catch (e) {
      const err = e as BusinessError;
      console.error('[HapticsPlugin] notificationFeedback failed: ' + err.message);
      invoke.reject('notificationFeedback failed: ' + err.message);
    }
  }

  // ─── selectionFeedback ───

  private handleSelectionFeedback(invoke: Invoke): void {
    this.doSelectionFeedback(invoke);
  }

  private async doSelectionFeedback(invoke: Invoke): Promise<void> {
    try {
      // Selection ticks use the soft preset (the lightest documented effect).
      await this.vibratePresetWithFallback('haptic.effect.soft', 40, 15);
      invoke.resolve('');
    } catch (e) {
      const err = e as BusinessError;
      console.error('[HapticsPlugin] selectionFeedback failed: ' + err.message);
      invoke.reject('selectionFeedback failed: ' + err.message);
    }
  }
}

export default HapticsPlugin;
