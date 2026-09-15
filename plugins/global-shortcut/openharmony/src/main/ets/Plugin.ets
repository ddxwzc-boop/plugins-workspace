import { Plugin, Invoke } from '@tauri/app';

/**
 * GlobalShortcutPlugin for OHOS.
 *
 * Shortcut registration and event handling is done entirely on the Rust side:
 * IPC commands (register, unregister, unregisterAll, isRegistered) are served
 * by the plugin's Rust invoke_handler, and triggered shortcut events flow back
 * through the openharmony-ability bridge plugin (ohos.global-shortcut). This
 * class only satisfies the ohpm package structure expected by the plugin
 * framework.
 */
export class GlobalShortcutPlugin extends Plugin {
  getCommands(): Map<string, (invoke: Invoke) => void> {
    // No ArkTS command handlers: every command is handled by the Rust side.
    const commands: Map<string, (invoke: Invoke) => void> = new Map();
    return commands;
  }
}

export default GlobalShortcutPlugin;
