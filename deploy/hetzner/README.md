# Hetzner voxl desktop

Headless XFCE on Xvfb, streamed by voxl over the tailnet. This is the
deployment used on `hetzner-ash` (`hetzner-ash.taile209c0.ts.net`).

## What runs

- Systemd slice `desk.slice` with `MemoryMax=3G`
- `desk-xvfb.service`: `Xvfb :1 -screen 0 2560x1440x24 -shmem` as user `desk`
- `desk-session.service`: `dbus-run-session startxfce4` on `DISPLAY=:1`
- `desk-voxl.service`: voxl on `127.0.0.1:6901`, files under `/home/desk`,
  `--enable-resize` on (the desktop follows the viewer's window, up to 2560×1440),
  audio off (no PulseAudio yet). Do not enable resize on the Mac mini.
- `tailscale serve --https=8445` proxies to that port (tailnet only).
  Port 8443 is a different service and must be left alone.

Open https://hetzner-ash.taile209c0.ts.net:8445/ from a browser or Voxl
Pocket (`wss://hetzner-ash.taile209c0.ts.net:8445/ws`). Any username
works; the password is the 1Password item **voxl Hetzner** in vault
Grokbot. It is also in `/etc/voxl/env` on the server (`chmod 600`,
`VOXL_PASSWORD=...`), which systemd loads as `EnvironmentFile`.

## Build notes

Ubuntu's `libyuv-dev` has no `libyuv.pc`. Drop a small pkg-config file
pointing at `/usr/lib/x86_64-linux-gnu` and `/usr/include`, and install
`libxcb1-dev` as well as `libx264-dev`, `libopus-dev`, `libpulse-dev`,
`clang`, `pkg-config`, and `esbuild`. Then:

```sh
cargo build --release --locked
install -m 755 target/release/voxl /usr/local/bin/voxl
cargo clean   # the release target is large; disk is tight
```

Firefox is the official Linux tarball in `/opt/firefox` (not the Ubuntu
snap). `xfce4-terminal` is the terminal.

Start Xvfb with `-shmem` so voxl can map the framebuffer in place
instead of using `GetImage`. Xvfb can only shrink from the size it started
at, so it is started at 2560×1440: a 1080p or 1440p monitor can then be a
1:1 desktop, and a 4K monitor is clamped to 2560×1440 and letterboxed.

## Browser

Google Chrome stable is installed from Google's official apt repo
(`/etc/apt/sources.list.d/google-chrome.sources`, signed by
`/usr/share/keyrings/google-chrome.gpg`). It is the default browser for
the `desk` session. The wrapper `/usr/local/bin/desk-chrome` starts it
with software rendering (`--disable-gpu`, SwiftShader) because this VM
has no usable GPU. A desktop icon and the dock Web Browser launcher
both call that wrapper.
