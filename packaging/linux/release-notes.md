## Install on Linux (x86_64)

- **Tarball installer.** Download `install.sh` and run `bash install.sh` to install the latest stable release under `~/.local`. For this exact release, download its `.tar.gz` and matching `.tar.gz.minisig` and pass the archive to `bash install.sh ./<filename>.tar.gz`. The installer verifies the signature. The installed app can update itself.
- **AppImage.** Download the `.AppImage`, run `chmod +x <filename>.AppImage`, and launch `./<filename>.AppImage`. If FUSE is unavailable, add `--appimage-extract-and-run`. Replace the file to update.
- **Flatpak.** Download the `.flatpak`, install it with `flatpak install --user ./<filename>.flatpak`, and launch `flatpak run io.github.alisayyah.HardcoverDesktop`. Install each new bundle the same way. Flatpak gets the Freedesktop runtime from Flathub.

All formats need Wayland or X11, Vulkan or OpenGL graphics, and a Secret Service keyring to store your sign-in. The tarball and AppImage require host glibc 2.35 or newer. The Flatpak runtime supplies glibc.

Each download has a matching `.minisig`. To verify a file, save its signature in the same directory, then run:

```sh
minisign -Vm FILENAME -P RWQmH1HvOAVP+8piXBk3ZC+nqzBVFZFcynEg8NUsgQDCSYYAkrSkPRXW
```
