# Apple Developer ID Signing Setup

This document covers exporting the Developer ID certificate and configuring GitHub Secrets so the release workflow produces a signed and notarized DMG.

## Prerequisites

- Enrolled in the Apple Developer Program (Team ID: 4JH68XUHC5)
- Xcode installed (for `xcrun notarytool`)
- The certificate "Developer ID Application: Bob Matsuoka (4JH68XUHC5)" in your login keychain

---

## Step 1: Verify the certificate is present

Open **Keychain Access**, switch to **login** keychain, select **My Certificates**.
Look for **Developer ID Application: Bob Matsuoka (4JH68XUHC5)**.

If it is missing, download it from [Apple Developer Console](https://developer.apple.com/account/resources/certificates/list) and double-click to install.

---

## Step 2: Export the certificate as .p12

1. Right-click the certificate in Keychain Access.
2. Choose **Export "Developer ID Application: Bob Matsuoka (4JH68XUHC5)"**.
3. Save as `developer-id.p12` (keep this file out of the repo).
4. Set a strong export password — you will need it as `APPLE_CERTIFICATE_PASSWORD`.

---

## Step 3: Base64-encode and copy

```bash
base64 -i developer-id.p12 | pbcopy
```

The clipboard now holds the base64-encoded certificate.

---

## Step 4: Add GitHub Secrets

Go to the repository on GitHub: **Settings > Secrets and variables > Actions > New repository secret**.

Add the following secrets:

| Secret name | Value |
|---|---|
| `APPLE_CERTIFICATE` | Base64-encoded .p12 (paste from clipboard) |
| `APPLE_CERTIFICATE_PASSWORD` | Export password you set in Step 2 |
| `KEYCHAIN_PASSWORD` | Any random string (used for the temporary CI keychain) |
| `APPLE_SIGNING_IDENTITY` | `Developer ID Application: Bob Matsuoka (4JH68XUHC5)` |

---

## Step 5: Set up notarization

Go to [appleid.apple.com](https://appleid.apple.com) > **Sign-In and Security > App-Specific Passwords** and generate a password labeled "CI notarytool".

Add three more secrets:

| Secret name | Value |
|---|---|
| `APPLE_ID` | Your Apple ID email (the one enrolled in the program) |
| `APPLE_TEAM_ID` | `4JH68XUHC5` |
| `APPLE_APP_SPECIFIC_PASSWORD` | The app-specific password generated above |

---

## How it works

**Release build** (`installer-release.yml`, triggered on `v*.*.*` tags):
- Imports the certificate into a temporary keychain if `APPLE_CERTIFICATE` is set.
- Signs the app using `APPLE_SIGNING_IDENTITY` (falls back to `-` / ad-hoc if the secret is absent).
- Notarizes and staples the DMG if `APPLE_ID` is set.
- Cleans up the temporary keychain on completion.

**Dev build** (`installer-dev.yml`, triggered on pushes to `main`):
- Uses `tauri.conf.dev.json` (identifier: `com.trusty-izzie.dev`, ad-hoc signing).
- Compiles with the `dev-instance` Cargo feature so the daemon writes to `~/.config/trusty-izzie-dev/`.
- Uploads the DMG as a GitHub Actions artifact (7-day retention, not a release).

---

## Local dev build

```bash
cd installer
npx tauri build --target universal-apple-darwin \
  --config src-tauri/tauri.conf.dev.json \
  --features dev-instance
```

This produces an ad-hoc signed DMG named **Izzie Dev** that installs alongside the prod app without conflict.
