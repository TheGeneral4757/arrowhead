### Install

**Any distro (AppImage, updates in-app):** download `arrowhead-@VERSION@-linux-x86_64.AppImage`, then `chmod +x arrowhead-@VERSION@-linux-x86_64.AppImage && ./arrowhead-@VERSION@-linux-x86_64.AppImage`. Needs `libfuse2`.

**Arch (AUR):** `yay -S arrowhead-bin` (or `paru -S arrowhead-bin`). Updates come from your package manager.

**Any distro (tarball):**

```sh
curl -LO https://github.com/TheGeneral4757/arrowhead/releases/download/@TAG@/arrowhead-@VERSION@-linux-x86_64.tar.gz
curl -LO https://github.com/TheGeneral4757/arrowhead/releases/download/@TAG@/SHA256SUMS
sha256sum -c --ignore-missing SHA256SUMS
tar xzf arrowhead-@VERSION@-linux-x86_64.tar.gz && cd arrowhead-@VERSION@-linux-x86_64 && ./arrowhead
```

**Windows 10/11:** download `arrowhead-@VERSION@-windows-x86_64.msi` (installer, updates in-app) or the `.zip` (portable) from the assets below.

Linux needs pipewire and xdg-desktop-portal (+ the KDE or Hyprland backend); the tarball also needs webkit2gtk-4.1, libsoup3 and gtk3. The `.sig` file is the in-app updater's signature.

### Setup after installing

1. **Key output (Linux):** arrowhead types through `/dev/uinput`. If `test -w /dev/uinput` fails, add the udev rule from the [guide](https://github.com/TheGeneral4757/arrowhead/blob/main/docs/GUIDE.md#let-arrowhead-press-keys-devuinput) and log in again. Without it only dry runs work.
2. **Game:** Funky Friday in Sober on a private server, 4-key mode. Note your four keybinds.
3. **Setup page**, with the game on screen: Permissions, Screen (pick the whole monitor the game is on), Calibrate (start any song), Latency, Keys (match your keybinds).
4. **Play:** Live, Start, then play songs. Dry run watches without pressing. F8 pause, F12 panic.

Full walkthrough, every setting, how it works and troubleshooting: [user guide](https://github.com/TheGeneral4757/arrowhead/blob/main/docs/GUIDE.md).

> Automating input in Roblox breaks its Terms of Use. Private servers only, at your own risk.
