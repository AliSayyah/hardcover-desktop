## Install

Three forms for x86_64 Linux, all the same app:

- **Installer:** download `install.sh` and run `bash install.sh`. It installs the latest release into `~/.local`, after checking its signature, and the app then updates itself. For this release's tarball instead, put its `.tar.gz.minisig` next to it and run `bash install.sh hardcover-desktop-<version>-x86_64-linux.tar.gz`.
- **AppImage:** download the `.AppImage`, `chmod +x` it and run it (without FUSE, add `--appimage-extract-and-run`). To update, replace the file.
- **Flatpak:** download the `.flatpak` and run `flatpak install --user <file>.flatpak`, then `flatpak run io.github.alisayyah.HardcoverDesktop`. The Freedesktop runtime comes from Flathub. To update, install the next bundle the same way.

The installer and the AppImage need glibc 2.35 or newer (Ubuntu 22.04, Debian 12, Fedora 36 and later); the Flatpak's runtime brings its own. All three need Wayland or X11, Vulkan or OpenGL graphics drivers, and a Secret Service keyring, such as GNOME Keyring or KWallet, to store your sign-in.

Every file has a `.minisig` signature. To check one with [minisign](https://jedisct1.github.io/minisign/), download both and run:

```sh
minisign -Vm <file> -P RWQmH1HvOAVP+8piXBk3ZC+nqzBVFZFcynEg8NUsgQDCSYYAkrSkPRXW
```
