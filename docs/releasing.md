# Releasing

A release is a published GitHub release. Publishing one runs `.github/workflows/desktop.yml`, which builds the desktop app for
Windows, Linux and macOS, signs the Windows and macOS builds and attaches all three to the release.

1. Write the notes in `docs/releases/<tag>.md` (see [v0.1.0](releases/v0.1.0.md)) and publish a release on a `v*` tag (a pre-release such as `v0.0.1-rc.1` works the same way).
2. `build` makes the three unsigned archives. `sign-windows` and `sign-macos` then replace the Windows and macOS ones with
   signed builds. `release` attaches the results.

## Signing

Both signing jobs run in the `release` environment, which allows only the `release` branch and `v*` tags and holds these
secrets (no repository variables are used):

| Platform | Secrets                                                                                                                       |
|----------|-------------------------------------------------------------------------------------------------------------------------------|
| Windows  | `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`, `AZURE_SIGNING_ENDPOINT`, `AZURE_SIGNING_ACCOUNT`, `AZURE_CERT_PROFILE` |
| macOS    | `APPLE_CERTIFICATE` (base64 .p12), `APPLE_CERTIFICATE_PASSWORD`, `APPLE_ID`, `APPLE_PASSWORD` (app-specific), `APPLE_TEAM_ID`, `KEYCHAIN_PASSWORD` |

- Windows: Azure Trusted Signing signs `laserfoundry.exe` and `laserfoundry-cli.exe`, and the job fails unless both signatures
  verify as valid.
- macOS: `packaging/macos/import-cert.sh` imports the certificate into a temporary keychain and `packaging/macos/package.sh`
  builds `LaserFoundry.app`, signs it and the command-line tool with "Developer ID Application", the hardened runtime and a
  timestamp, notarizes with `notarytool` (a rejection fails the job and prints Apple's log) and staples the app. The keychain is
  deleted at the end of the job.
- With any secret of a platform missing, that platform's job warns and the release carries the unsigned build; the run does not fail.
- A manual run (`workflow_dispatch`) from `main` builds all three but the signing jobs are refused by the environment. Run it from
  the `release` branch or a `v*` tag to sign.

To try the macOS packaging without secrets, run `packaging/macos/package.sh <dir with laserfoundry and laserfoundry-cli> <out>` on
a Mac; without `KEYCHAIN` set it builds an unsigned app.
