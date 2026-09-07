## Default Permission

This permission set configures which kind of
updater functions are exposed to the frontend.

#### Granted Permissions

The full workflow from checking for updates to installing them
is enabled. On OpenHarmony the AppGallery-specific commands
(`checkAppGalleryUpdate` / `showAppGalleryUpdateDialog`) are also
enabled; they only resolve on OHOS builds.

#### This default permission set includes the following:

- `allow-check`
- `allow-download`
- `allow-install`
- `allow-download-and-install`
- `allow-check-app-gallery-update`
- `allow-show-app-gallery-update-dialog`

## Permission Table

<table>
<tr>
<th>Identifier</th>
<th>Description</th>
</tr>


<tr>
<td>

`updater:allow-check`

</td>
<td>

Enables the check command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`updater:deny-check`

</td>
<td>

Denies the check command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`updater:allow-check-app-gallery-update`

</td>
<td>

Enables the check_app_gallery_update command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`updater:deny-check-app-gallery-update`

</td>
<td>

Denies the check_app_gallery_update command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`updater:allow-download`

</td>
<td>

Enables the download command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`updater:deny-download`

</td>
<td>

Denies the download command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`updater:allow-download-and-install`

</td>
<td>

Enables the download_and_install command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`updater:deny-download-and-install`

</td>
<td>

Denies the download_and_install command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`updater:allow-install`

</td>
<td>

Enables the install command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`updater:deny-install`

</td>
<td>

Denies the install command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`updater:allow-show-app-gallery-update-dialog`

</td>
<td>

Enables the show_app_gallery_update_dialog command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`updater:deny-show-app-gallery-update-dialog`

</td>
<td>

Denies the show_app_gallery_update_dialog command without any pre-configured scope.

</td>
</tr>
</table>
