## Install

Two forms for x86_64 Linux, the same app in each. Either way it updates itself, in one click, after verifying the new release's signature.

- **Installer:** download `install.sh` and run `bash install.sh`. It installs the latest release into `~/.local`, after checking its signature. For this release's tarball instead, put its `.tar.gz.minisig` next to it and run `bash install.sh hardcover-desktop-<version>-x86_64-linux.tar.gz`.
- **AppImage:** download the `.AppImage`, `chmod +x` it and run it (without FUSE, add `--appimage-extract-and-run`). The app replaces the file in place when it updates; AppImageUpdate and similar tools can update it too.

Both need glibc 2.35 or newer (Ubuntu 22.04, Debian 12, Fedora 36 and later), Wayland or X11, Vulkan or OpenGL graphics drivers, and a Secret Service keyring, such as GNOME Keyring or KWallet, to store your sign-in.

Every download has a `.minisig` signature. To check one with [minisign](https://jedisct1.github.io/minisign/), download both and run:

```sh
minisign -Vm <file> -P RWQmH1HvOAVP+8piXBk3ZC+nqzBVFZFcynEg8NUsgQDCSYYAkrSkPRXW
```
