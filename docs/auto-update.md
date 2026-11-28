# Auto-update (Windows)

Release builds of Ledge Grab update themselves. Local `cargo build`s never do:
updating is compiled in only when `LEDGE_GRAB_UPDATES=1` is set at build time, which
`packaging/build-release.ps1` does.

## How it works

1. **Check.** A background thread in the app fetches
   `https://github.com/Nabin-me/ledge-grab/releases/latest/download/ledge-grab-update.txt`
   20 seconds after launch and then every 10 minutes, through WinHTTP (the system's
   proxy and TLS settings). The User-Agent is `LedgeGrab/<version>`; nothing else
   about the user is sent.
2. **Verify.** The manifest must carry a valid ed25519 signature for the public key
   in `src/update.rs`, name a newer version than the running one, and list files whose
   names carry that version (so an old signed file can't pose as a new one).
3. **Download.** The file for this copy is downloaded to
   `%LOCALAPPDATA%\Ledge Grab\updates\`, and its size and SHA-256 are checked
   against the manifest. A mismatch deletes it.
   - Installed for this user (Inno Setup record in HKCU points at the exe's folder):
     `LedgeGrab-Setup-<v>-x64.exe`.
   - Portable ZIP or standalone exe: `LedgeGrab-<v>-win-x64.exe`.
   - Installed for all users (HKLM): no silent update, since it would need an admin.
4. **Install when idle.** The main window checks every 15 seconds and installs only
   when no recording, Sequential Copy, Scroll to PDF, interval capture, region
   picker or background save is running, and the shelf is closed.
   - Installed: a hidden PowerShell waits for the app to exit, runs the installer
     with `/VERYSILENT /CURRENTUSER`, then starts the installed exe. If the install
     fails, the old version comes back.
   - Portable: the running exe is renamed to `.old`, the new one is copied in its
     place and started; the next launch deletes `.old`.
5. **Confirm.** The relaunched app gets `--updated=<v>`. On the new version it shows
   "Ledge Grab updated"; if it came back as the old version, it records that and
   skips that release for a day.

## Release key

The private key is the `UPDATE_SIGNING_KEY` repository secret (base64 32-byte
ed25519 seed). The release workflow signs `ledge-grab-update.txt` with
`packaging/update-manifest.py` after code signing, and fails if the secret is
missing, so no release ships that installed copies can't verify.

Losing the private key means installed copies can't be updated automatically any
more (users would have to download a release with a new key by hand). Keep an
offline backup. To rotate it: `python packaging/update-manifest.py keygen NEW_FILE`,
put the printed public key in `src/update.rs`, ship one release signed with the
**old** key that contains the new public key, then switch the secret.

## Turning it off

Set `LEDGE_GRAB_NO_AUTO_UPDATE=1` in the user's environment.

## Testing

`tests/update_e2e.ps1` runs real updates against a local feed: a portable copy
swapping its exe, an installed copy upgrading through the installer, a tampered
manifest, and a download that doesn't match its checksum. The payload is built with a
throwaway key: compile with `LEDGE_GRAB_UPDATES=1` and
`LEDGE_GRAB_UPDATE_PUBKEY=<test public key>` (the newer copy also with
`LEDGE_GRAB_VERSION_OVERRIDE`), build both installers with ISCC, and sign the manifest
with the test key. `LEDGE_GRAB_UPDATE_FEED` points the app at the local server (HTTPS,
or HTTP to localhost only); signatures are checked the same way.

Not covered yet: the wait while the app is busy (checked in code, not by the test),
Linux (no Linux releases yet), and admin installs.
