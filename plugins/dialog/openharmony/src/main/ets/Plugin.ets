import picker from '@ohos.file.picker';
import promptAction from '@ohos.promptAction';
import deviceInfo from '@ohos.deviceInfo';
import fs from '@ohos.file.fs';
import fileUri from '@ohos.file.fileuri';
import { Plugin, Invoke } from '@tauri/app';

interface FileFilter {
  name?: string;
  extensions?: string[];
}

interface OpenArgs {
  multiple?: boolean;
  directory?: boolean;
  filters?: FileFilter[];
  defaultPath?: string;
  title?: string;
}

interface SaveArgs {
  filters?: FileFilter[];
  defaultPath?: string;
  title?: string;
  fileName?: string;
}

interface MessageArgs {
  title?: string;
  message?: string;
  kind?: string;
  okButtonLabel?: string;
  noButtonLabel?: string;
  cancelButtonLabel?: string;
}

export class DialogPlugin extends Plugin {
  getCommands(): Map<string, (invoke: Invoke) => void> {
    const commands: Map<string, (invoke: Invoke) => void> = new Map();
    commands.set('showFilePicker', (invoke: Invoke): void => { this.handleOpen(invoke); });
    commands.set('saveFileDialog', (invoke: Invoke): void => { this.handleSave(invoke); });
    commands.set('showMessageDialog', (invoke: Invoke): void => { this.handleMessage(invoke); });
    return commands;
  }

  private handleOpen(invoke: Invoke): void {
    const argsStr = invoke.parseArgs();
    console.info('[DialogPlugin] showFilePicker args: ' + argsStr);
    const args: OpenArgs = JSON.parse(argsStr) as OpenArgs;
    const multiple = args.multiple ?? false;
    const directory = args.directory ?? false;
    this.showDocumentPicker(invoke, multiple, directory, args.filters ?? []);
  }

  private handleSave(invoke: Invoke): void {
    const argsStr = invoke.parseArgs();
    console.info('[DialogPlugin] saveFileDialog args: ' + argsStr);
    const args: SaveArgs = JSON.parse(argsStr) as SaveArgs;
    const fileName = args.fileName ?? 'untitled';
    this.showSavePicker(invoke, fileName, args.filters ?? []);
  }

  private handleMessage(invoke: Invoke): void {
    const argsStr = invoke.parseArgs();
    console.info('[DialogPlugin] showMessageDialog args: ' + argsStr);
    const args: MessageArgs = JSON.parse(argsStr) as MessageArgs;
    const title = args.title ?? '';
    const message = args.message ?? '';
    const kind = args.kind ?? 'info';
    const buttons: string[] = [];
    if (args.okButtonLabel != null) {
      buttons.push(args.okButtonLabel);
    }
    if (args.noButtonLabel != null) {
      buttons.push(args.noButtonLabel);
    }
    if (args.cancelButtonLabel != null) {
      buttons.push(args.cancelButtonLabel);
    }
    if (buttons.length === 0) {
      buttons.push('OK');
    }
    this.showMessageBox(invoke, title, message, buttons, kind);
  }

  private async showDocumentPicker(invoke: Invoke, multiple: boolean, directory: boolean, filters: FileFilter[]): Promise<void> {
    try {
      console.info('[DialogPlugin] showDocumentPicker multiple=' + multiple + ' directory=' + directory);
      const documentPicker = new picker.DocumentViewPicker();
      // Documented range is 1-500 (@ohos.file.picker d.ts; the online
      // reference caps at 500 for API <= 20 and lifts the file-count limit
      // from API 21) — 10 was an arbitrary cap that silently truncated
      // multi-select. Mirrors the ohos.files bridge (allowMany ? 500 : 1).
      const options: picker.DocumentSelectOptions = {
        maxSelectNumber: multiple ? 500 : 1
      };
      // Suffix filters apply to file picking only: folder mode must keep its
      // #99/#124-verified behavior, and the fileSuffixFilters x
      // selectMode(FOLDER/MIXED) combination is undocumented — the ohos.files
      // bridge likewise leaves fileSuffixFilters unset for folder picking.
      if (!directory) {
        const suffixFilters = this.pickerSuffixChoices(filters);
        if (suffixFilters !== undefined) {
          options.fileSuffixFilters = suffixFilters;
        }
      }

      // Folder selection (Eulogizethesun/tauri#99): DocumentViewPicker has no
      // dedicated folder mode — selectMode decides what can be picked.
      // - 2in1 (PC): MIXED is the only mode that works there (files and
      //   folders mixed; per official docs MIXED has no effect on other
      //   devices).
      // - Phone: FOLDER requires API 26+; older versions get an explicit
      //   error instead of a silent empty result.
      if (directory) {
        if (deviceInfo.deviceType === '2in1') {
          options.selectMode = picker.DocumentSelectMode.MIXED;
        } else {
          if (deviceInfo.sdkApiVersion < 26) {
            console.error('[DialogPlugin] folder picking requires API 26+ on non-2in1 devices, current ' + deviceInfo.sdkApiVersion);
            invoke.reject('Folder picking is not supported on this device (FOLDER selectMode requires API 26+, current ' + deviceInfo.sdkApiVersion + ')');
            return;
          }
          options.selectMode = picker.DocumentSelectMode.FOLDER;
        }
      }

      const result = await documentPicker.select(options);
      console.info('[DialogPlugin] documentPicker result: ' + JSON.stringify(result));
      if (result && result.length > 0) {
        // MIXED mode on 2in1 lets the user select files alongside folders
        // (Eulogizethesun/tauri#124): drop file URIs instead of returning
        // them as picked folders. Idempotent for Phone's FOLDER mode, which
        // cannot select files in the first place.
        let selected: string[] = result;
        if (directory) {
          selected = [];
          for (const uri of result) {
            if (this.isDirectoryUri(uri)) {
              selected.push(uri);
            } else {
              console.warn('[DialogPlugin] directory picker dropping non-directory URI: ' + uri);
            }
          }
        }
        if (selected.length > 0) {
          invoke.resolve(JSON.stringify({ files: selected }));
        } else {
          // Mirror the cancel branch below: never resolve an empty files
          // array — the Rust side unwraps the first element and would panic.
          invoke.reject('File picker cancelled');
        }
      } else {
        invoke.reject('File picker cancelled');
      }
    } catch (e) {
      const err = e as Error;
      console.error('[DialogPlugin] showDocumentPicker error: ' + err.message);
      invoke.reject('Document picker failed: ' + err.message);
    }
  }

  // Converts Tauri filters ({ name, extensions }) to the picker's
  // "description|.ext1,.ext2" choice strings — the grammar of
  // DocumentSelectOptions.fileSuffixFilters and
  // DocumentSaveOptions.fileSuffixChoices (@ohos.file.picker d.ts:
  // each element is "description|suffixes", comma-separated suffixes,
  // max 100 elements; default is no filtering). Mirrors the ohos.files
  // bridge (FilesPlugin.ets pickerSuffixFilters).
  private pickerSuffixChoices(filters: FileFilter[]): string[] | undefined {
    const choices: string[] = [];
    for (const filter of filters) {
      const name = (filter.name ?? 'Files').split('|').join(' ');
      const suffixes: string[] = [];
      for (const extension of filter.extensions ?? []) {
        const trimmed = extension.trim();
        if (trimmed.length === 0) {
          continue;
        }
        if (trimmed.indexOf('/') >= 0) {
          // MIME types (allowed in `extensions` by the JS API on mobile)
          // cannot be expressed in the suffix grammar — skip instead of
          // producing a filter that matches nothing.
          console.warn('[DialogPlugin] skipping MIME-type filter entry: ' + trimmed);
          continue;
        }
        if (trimmed === '*' || trimmed === '*.*') {
          // "All files" wildcard (2in1; API 17+ on phone).
          suffixes.push('.*');
        } else {
          suffixes.push(trimmed.startsWith('.') ? trimmed : '.' + trimmed);
        }
      }
      if (suffixes.length > 0) {
        choices.push(name + '|' + suffixes.join(','));
      }
    }
    if (choices.length === 0) {
      return undefined;
    }
    return choices.slice(0, 100);
  }

  // Checks whether a picker-returned URI points to a directory
  // (Eulogizethesun/tauri#124).
  // - API 22+: fs.statSync accepts URIs directly, so Stat.isDirectory()
  //   decides.
  // - API < 22: FileUri.getFullDirectoryUri() (since 11) returns the
  //   containing directory URI when the URI points to a file and the URI
  //   itself unchanged when it points to a directory, so equality means the
  //   URI is a directory.
  // A failed check counts as a non-directory: the caller drops the entry
  // rather than returning a file as a picked folder.
  private isDirectoryUri(uri: string): boolean {
    if (deviceInfo.sdkApiVersion >= 22) {
      try {
        return fs.statSync(uri).isDirectory();
      } catch {
        return false;
      }
    }
    try {
      return new fileUri.FileUri(uri).getFullDirectoryUri() === uri;
    } catch {
      return false;
    }
  }

  private async showSavePicker(invoke: Invoke, fileName: string, filters: FileFilter[]): Promise<void> {
    try {
      console.info('[DialogPlugin] showSavePicker fileName=' + fileName);
      const documentPicker = new picker.DocumentViewPicker();
      const options: picker.DocumentSaveOptions = {
        newFileNames: [fileName]
      };
      const suffixChoices = this.pickerSuffixChoices(filters);
      if (suffixChoices !== undefined) {
        options.fileSuffixChoices = suffixChoices;
      }

      const result = await documentPicker.save(options);
      console.info('[DialogPlugin] savePicker result: ' + JSON.stringify(result));
      if (result && result.length > 0) {
        invoke.resolve(JSON.stringify({ file: result[0] }));
      } else {
        invoke.reject('Save cancelled');
      }
    } catch (e) {
      const err = e as Error;
      console.error('[DialogPlugin] showSavePicker error: ' + err.message);
      invoke.reject('Save picker failed: ' + err.message);
    }
  }

  private async showMessageBox(invoke: Invoke, title: string, message: string, buttons: string[], kind: string): Promise<void> {
    try {
      console.info('[DialogPlugin] showMessageBox title=' + title + ', message=' + message + ', kind=' + kind);
      const buttonOptions: promptAction.Button[] = buttons.map((text: string): promptAction.Button => {
        return { text: text, color: '#1890FF' };
      });

      // promptAction.ShowDialogOptions has no icon/severity parameter (nor
      // does ArkUI AlertDialog) — the info/warning/error level is expressed
      // as a glyph prefix on the title (on the message when there is no
      // title), the only channel the system dialog offers.
      let dialogTitle: string = title;
      let dialogMessage: string = message;
      let prefix: string = '';
      if (kind === 'warning') {
        prefix = '⚠️ '; // warning sign
      } else if (kind === 'error') {
        prefix = '❌ '; // cross mark
      }
      if (prefix !== '') {
        if (dialogTitle.length > 0) {
          dialogTitle = prefix + dialogTitle;
        } else {
          dialogMessage = prefix + dialogMessage;
        }
      }

      const result = await promptAction.showDialog({
        title: dialogTitle,
        message: dialogMessage,
        buttons: buttonOptions
      });

      const buttonText = buttons[result.index];
      console.info('[DialogPlugin] messageBox result: ' + buttonText);
      invoke.resolve(JSON.stringify({ value: buttonText }));
    } catch (e) {
      const err = e as Error;
      console.error('[DialogPlugin] showMessageBox error: ' + err.message);
      invoke.reject('Message box failed: ' + err.message);
    }
  }
}

export default DialogPlugin;