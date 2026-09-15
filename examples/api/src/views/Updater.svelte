<script>
  import {
    check,
    checkAppGalleryUpdate,
    showAppGalleryUpdateDialog
  } from '@tauri-apps/plugin-updater'
  import { relaunch } from '@tauri-apps/plugin-process'
  import { platform } from '@tauri-apps/plugin-os'

  export let onMessage

  const isOhos = platform() === 'ohos'

  let isChecking, isInstalling, newUpdate
  let totalSize = 0,
    downloadedSize = 0

  async function checkUpdate() {
    isChecking = true
    try {
      if (isOhos) {
        const update = await checkAppGalleryUpdate()
        if (update) {
          onMessage(`Should update: ${update.version ?? 'unknown'}`)
          onMessage(update)

          newUpdate = update
        } else {
          onMessage('No update available')
        }
      } else {
        const update = await check()
        if (update) {
          onMessage(`Should update: ${update.available}`)
          onMessage(update)

          newUpdate = update
        } else {
          onMessage('No update available')
        }
      }
    } catch (e) {
      onMessage(e)
    } finally {
      isChecking = false
    }
  }

  async function install() {
    if (isOhos) {
      isInstalling = true
      try {
        await showAppGalleryUpdateDialog()
        onMessage('Update dialog shown')
      } catch (e) {
        console.error(e)
        onMessage(e)
      } finally {
        isInstalling = false
      }
      return
    }
    isInstalling = true
    downloadedSize = 0
    try {
      await newUpdate.downloadAndInstall((downloadProgress) => {
        switch (downloadProgress.event) {
          case 'Started':
            totalSize = downloadProgress.data.contentLength
            break
          case 'Progress':
            downloadedSize += downloadProgress.data.chunkLength
            break
          case 'Finished':
            break
        }
      })
      onMessage('Installation complete, restarting...')
      await new Promise((resolve) => setTimeout(resolve, 2000))
      await relaunch()
    } catch (e) {
      console.error(e)
      onMessage(e)
    } finally {
      isInstalling = false
    }
  }

  $: progress = totalSize ? Math.round((downloadedSize / totalSize) * 100) : 0
</script>

<div class="flex children:grow children:h10">
  {#if !isChecking && !newUpdate}
    <button class="btn" on:click={checkUpdate}>Check update</button>
  {:else if !isInstalling && newUpdate}
    <button class="btn" on:click={install}>
      {isOhos ? 'Show update dialog' : 'Install update'}
    </button>
  {:else if !isOhos}
    <div class="progress">
      <span>{progress}%</span>
      <div class="progress-bar" style="width: {progress}%"></div>
    </div>
  {/if}
</div>

<style>
  .progress {
    width: 100%;
    height: 50px;
    position: relative;
    margin-top: 5%;
  }

  .progress > span {
    font-size: 1.2rem;
  }

  .progress-bar {
    height: 30px;
    background-color: hsl(32, 94%, 46%);
    border: 1px solid #333;
  }
</style>
