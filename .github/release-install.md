### Install

**Arch (AUR):** `yay -S arrowhead-bin` (or `paru -S arrowhead-bin`). Updates come from your package manager.

**Any distro (tarball):**

```sh
curl -LO https://github.com/TheGeneral4757/arrowhead/releases/download/@TAG@/arrowhead-@VERSION@-linux-x86_64.tar.gz
curl -LO https://github.com/TheGeneral4757/arrowhead/releases/download/@TAG@/SHA256SUMS
sha256sum -c --ignore-missing SHA256SUMS
tar xzf arrowhead-@VERSION@-linux-x86_64.tar.gz && cd arrowhead-@VERSION@-linux-x86_64 && ./arrowhead
```

Needs webkit2gtk-4.1, libsoup3, gtk3, pipewire, xdg-desktop-portal (+ the KDE or Hyprland backend) and a writable `/dev/uinput`. See the [README](https://github.com/TheGeneral4757/arrowhead#install) for first-run setup. The `.sig` file is the in-app updater's signature.

> Automating input in Roblox breaks its Terms of Use. Private servers only, at your own risk.

