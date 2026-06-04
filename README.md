# AI Balance Widget for New API

AI Balance Widget for New API is a frameless Windows Tauri desktop widget for
checking a New API style account balance endpoint. It polls once per minute,
shows only the numeric remaining balance, and keeps tray commands for showing
the widget, opening settings, checking updates, and exiting.

## Credentials

Credentials and provider endpoints are not stored in this repository.

Open Settings from the tray menu or the widget title bar, then enter:

- API endpoint, for example a `GET /api/user/self` compatible endpoint
- Access Token from the provider security settings
- User ID from the provider account

The app saves those values in the local Tauri app config directory on this
machine.

## Auto Update

Release builds use Tauri signed updater through GitHub Releases:

```text
https://github.com/coderDJing/ai-balance-widget-new-api/releases/latest/download/latest.json
```

The updater public key is stored in `src-tauri/tauri.conf.json`. The matching
private key is local-only:

```text
C:\Users\coder\.tauri\ai-balance-orb.key
```

Do not commit the private key or paste it into issue/release text. GitHub
Actions needs these repository secrets for signed updater artifacts:

- `TAURI_SIGNING_PRIVATE_KEY`
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`

The current key has no password, so the password secret may be empty or omitted.

## Development

```bash
pnpm install
pnpm tauri:dev
```

## Desktop Builds

Build on the target platform:

```bash
pnpm tauri:build
```

The repository includes GitHub Actions for Windows builds.

## Release

Release versions must stay synchronized in `package.json`,
`src-tauri/Cargo.toml`, and `src-tauri/tauri.conf.json`.

Run the release script from the repository root:

```powershell
.\scripts\release.ps1 0.1.1
```

The script validates a clean `master` branch, updates the three version files,
runs `pnpm build`, `pnpm check:desktop`, and a local signed debug NSIS updater
build, commits the version bump, creates the `v0.1.1` tag, and pushes `master`
plus the tag. Pushing a `v*` tag triggers `.github/workflows/release.yml`, which
builds and publishes the Windows GitHub Release with installer, signature, and
`latest.json`.

The release workflow keeps:

- `releaseDraft: false`
- `prerelease: false`
- `updaterJsonPreferNsis: true`
- `args: --ci`

## Verification

```bash
pnpm build
pnpm check:desktop
```

Signed updater artifact smoke check:

```powershell
$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content -Raw "C:/Users/coder/.tauri/ai-balance-orb.key"
pnpm tauri build --debug --bundles nsis --ci
```

Expected debug bundle outputs:

```text
src-tauri/target/debug/bundle/nsis/*.exe
src-tauri/target/debug/bundle/nsis/*.exe.sig
```
