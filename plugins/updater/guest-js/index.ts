// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

import { invoke, Channel, Resource } from '@tauri-apps/api/core'

/** Options used when checking for updates */
interface CheckOptions {
  /**
   * Request headers
   */
  headers?: HeadersInit
  /**
   * Timeout in milliseconds
   */
  timeout?: number
  /**
   * A proxy url to be used when checking and downloading updates.
   */
  proxy?: string
  /**
   * Target identifier for the running application. This is sent to the backend.
   */
  target?: string
  /**
   * Allow downgrades to previous versions by not checking if the current version is greater than the available version.
   */
  allowDowngrades?: boolean
}

/** Options used when downloading an update */
interface DownloadOptions {
  /**
   * Request headers
   */
  headers?: HeadersInit
  /**
   * Timeout in milliseconds
   */
  timeout?: number
}

interface UpdateMetadata {
  rid: number
  currentVersion: string
  version: string
  date?: string
  body?: string
  rawJson: Record<string, unknown>
}

/** Updater download event */
type DownloadEvent =
  | { event: 'Started'; data: { contentLength?: number } }
  | { event: 'Progress'; data: { chunkLength: number } }
  | { event: 'Finished' }

class Update extends Resource {
  // TODO: remove this field in v3
  /** @deprecated This is always true, check if the return value is `null` instead when using {@linkcode check} */
  available: boolean
  currentVersion: string
  version: string
  date?: string
  body?: string
  rawJson: Record<string, unknown>
  private downloadedBytes?: Resource

  constructor(metadata: UpdateMetadata) {
    super(metadata.rid)
    this.available = true
    this.currentVersion = metadata.currentVersion
    this.version = metadata.version
    this.date = metadata.date
    this.body = metadata.body
    this.rawJson = metadata.rawJson
  }

  /** Download the updater package */
  async download(
    onEvent?: (progress: DownloadEvent) => void,
    options?: DownloadOptions
  ): Promise<void> {
    convertToRustHeaders(options)
    const channel = new Channel<DownloadEvent>()
    if (onEvent) {
      channel.onmessage = onEvent
    }
    const downloadedBytesRid = await invoke<number>('plugin:updater|download', {
      onEvent: channel,
      rid: this.rid,
      ...options
    })
    this.downloadedBytes = new Resource(downloadedBytesRid)
  }

  /** Install downloaded updater package */
  async install(): Promise<void> {
    if (!this.downloadedBytes) {
      throw new Error('Update.install called before Update.download')
    }

    await invoke('plugin:updater|install', {
      updateRid: this.rid,
      bytesRid: this.downloadedBytes.rid
    })

    // Don't need to call close, we did it in rust side already
    this.downloadedBytes = undefined
  }

  /** Downloads the updater package and installs it */
  async downloadAndInstall(
    onEvent?: (progress: DownloadEvent) => void,
    options?: DownloadOptions
  ): Promise<void> {
    convertToRustHeaders(options)
    const channel = new Channel<DownloadEvent>()
    if (onEvent) {
      channel.onmessage = onEvent
    }
    await invoke('plugin:updater|download_and_install', {
      onEvent: channel,
      rid: this.rid,
      ...options
    })
  }

  async close(): Promise<void> {
    await this.downloadedBytes?.close()
    await super.close()
  }
}

/** Check for updates, resolves to `null` if no updates are available */
async function check(options?: CheckOptions): Promise<Update | null> {
  convertToRustHeaders(options)

  const metadata = await invoke<UpdateMetadata | null>('plugin:updater|check', {
    ...options
  })
  return metadata ? new Update(metadata) : null
}

/** AppGallery update info (OpenHarmony only) */
interface AppGalleryUpdate {
  /**
   * Always `true` when this object is returned — a `null` result means no
   * update is available. Kept for API-explicitness.
   */
  available: boolean
  /** The currently installed version */
  currentVersion: string
  /** The available update version, or `null` on API levels below 20 */
  version: string | null
}

/**
 * Check for updates via AppGallery (OpenHarmony only).
 *
 * The update source is AppGallery — the desktop `check` options (headers,
 * timeout, proxy, target, allowDowngrades) do not apply. `version` is `null`
 * on API levels below 20 (`versionName` unavailable).
 *
 * Resolves to `null` when no update is available.
 *
 * @platform openharmony
 */
async function checkAppGalleryUpdate(): Promise<AppGalleryUpdate | null> {
  return await invoke<AppGalleryUpdate | null>(
    'plugin:updater|check_app_gallery_update'
  )
}

/**
 * Show the system AppGallery update dialog (OpenHarmony only).
 *
 * The system dialog drives the entire download + installation flow and
 * whether to update is the user's decision; the promise resolves once the
 * dialog request is dispatched (it does NOT mean the update was installed).
 *
 * @platform openharmony
 */
async function showAppGalleryUpdateDialog(): Promise<void> {
  await invoke('plugin:updater|show_app_gallery_update_dialog')
}

/**
 * Converts the headers in options to be an {@linkcode Array<[string, string]>} which is what the Rust side expects
 */
function convertToRustHeaders(options?: { headers?: HeadersInit }) {
  if (options?.headers) {
    options.headers = Array.from(new Headers(options.headers).entries())
  }
}

export type { CheckOptions, DownloadOptions, DownloadEvent, AppGalleryUpdate }
export {
  check,
  checkAppGalleryUpdate,
  showAppGalleryUpdateDialog,
  Update
}
