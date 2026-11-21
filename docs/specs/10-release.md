---
created: 2026-11-21
status: draft
milestone: M5
---

# 10 Release and updates

## Versioning
- SemVer, one version for the whole workspace (`version.workspace = true`) and `ui/package.json`.
- release-please (`.github/workflows/release-please.yml`, `release-type: rust`, extra-files for `ui/package.json` and `crates/app/tauri.conf.json`). Merging its PR creates the tag `vX.Y.Z` and the GitHub release notes.

## Release build (`.github/workflows/release.yml`, on tag)
| Runner | Build | Assets |
|---|---|---|
| ubuntu-24.04 | `tauri build --no-bundle` for the app; `cargo build --release` for cli and overlay | `arrowhead-X.Y.Z-linux-x86_64.tar.gz` (app, cli, overlay, LICENSE, README, `arrowhead.desktop`, icon) |
| windows-latest (v0.2) | `tauri build` with the MSI bundler | `arrowhead-X.Y.Z-windows-x86_64.msi`, portable `arrowhead-X.Y.Z-windows-x86_64.zip` |

Plus `SHA256SUMS` and the updater manifest `latest.json` signed with the Tauri updater key (secret `TAURI_SIGNING_PRIVATE_KEY` in repo secrets; the public key is in `tauri.conf.json`).

Linux runtime deps (documented in README): webkit2gtk-4.1, libsoup3, gtk3, pipewire, xdg-desktop-portal + a backend (kde or hyprland).

## AUR (`packaging/aur/PKGBUILD`, package `arrowhead-bin`)
- Installs from the release tarball: `/usr/bin/arrowhead`, `/usr/bin/arrowhead-cli`, `/usr/lib/arrowhead/arrowhead-overlay`, desktop file, icon, license.
- `depends=(webkit2gtk-4.1 libsoup3 gtk3 pipewire xdg-desktop-portal)`.
- A CI job bumps `pkgver` and checksums on release; publishing to the AUR via an action is optional and off until the user enables it (needs an AUR SSH key secret).
- The AUR build sets `ARROWHEAD_PACKAGED=aur` at compile time; the app then only notifies about updates and says "update with your package manager".

## In-app updates
- On launch (if `updates.check`) and every 24 h: fetch `latest.json` from the GitHub release. This is the only network call the app makes.
- Newer version: a pill in the top bar; Settings shows notes and an "Update and restart" button (tauri-plugin-updater: download, verify signature, replace, relaunch).
- Never updates while `Playing`.

## Repo setup (when the user approves going public)
1. `gh repo create TheGeneral4757/arrowhead --public --source . --push`.
2. Branch protection on `main`: CI required, squash merge.
3. Secrets: `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`; optional `AUR_SSH_KEY`.
4. Archive fnf-thing-v2: push `d600e1a`, add a README pointer to arrowhead, `gh repo archive`.
