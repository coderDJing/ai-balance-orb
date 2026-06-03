# AI Balance Orb

AI Balance Orb is a frameless Tauri desktop widget for checking a New API style
account balance endpoint. It supports Windows and macOS, polls once per minute,
shows only the numeric remaining balance, and keeps a tray icon with a tray Exit
command.

## Credentials

Credentials and provider endpoints are not stored in this repository.

On first launch, enter:

- API endpoint, for example a `GET /api/user/self` compatible endpoint
- Access Token from the provider security settings
- User ID from the provider account

The app saves those values in the local Tauri app config directory on this
machine.

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

The repository includes GitHub Actions for Windows and macOS builds.

## Verification

```bash
pnpm build
pnpm check:desktop
```
