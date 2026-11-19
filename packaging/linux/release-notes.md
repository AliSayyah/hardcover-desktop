## Install

Download `hardcover-desktop-<version>-x86_64-linux.tar.gz`, check it against its `.sha256`, and unpack it into `~/.local`:

```sh
sha256sum -c hardcover-desktop-*-x86_64-linux.tar.gz.sha256
tar -xzf hardcover-desktop-*-x86_64-linux.tar.gz -C ~/.local --strip-components=1
```

This installs `~/.local/bin/hardcover-desktop` with its desktop entry, icon and metainfo, so the app appears in your launcher.

Requires x86_64 Linux with glibc 2.35 or newer (Ubuntu 22.04, Debian 12, Fedora 36 and later), Vulkan or OpenGL graphics drivers, and a Secret Service keyring such as GNOME Keyring or KWallet to store your sign-in.
