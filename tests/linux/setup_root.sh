#!/usr/bin/env bash
# Root setup for the Ledge Grab Linux test distro: headless GNOME (Wayland + X11),
# portals, PipeWire, test tools, and build dependencies.
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive

id -u lg >/dev/null 2>&1 || useradd -m -s /bin/bash lg
printf '[user]\ndefault=lg\n[boot]\nsystemd=true\n' > /etc/wsl.conf

apt-get update -q
apt-get install -y -q --no-install-recommends \
  gnome-shell mutter gnome-session gnome-settings-daemon gsettings-desktop-schemas \
  xwayland xvfb x11-utils x11-xserver-utils xdotool wmctrl xclip \
  xdg-desktop-portal xdg-desktop-portal-gnome xdg-desktop-portal-gtk \
  pipewire pipewire-pulse wireplumber gstreamer1.0-pipewire \
  dbus dbus-x11 dbus-user-session at-spi2-core \
  wl-clipboard imagemagick gnome-screenshot python3-gi gir1.2-gtk-4.0 python3-dbus \
  fonts-inter fonts-noto-color-emoji fonts-dejavu-core \
  build-essential pkg-config curl git ca-certificates clang \
  libgtk-4-dev libgtk-3-dev libdbus-1-dev libpipewire-0.3-dev libspa-0.2-dev \
  libxcb1-dev libxcb-shm0-dev libxcb-randr0-dev libxcb-xfixes0-dev libxcb-xtest0-dev \
  libx11-dev libxtst-dev libxi-dev libxkbcommon-dev libwayland-dev wayland-protocols \
  libssl-dev libsqlite3-dev libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
  gstreamer1.0-plugins-good gstreamer1.0-plugins-ugly gstreamer1.0-x
echo "gnome-shell: $(gnome-shell --version)"
echo "mutter: $(mutter --version | head -1)"
echo "ROOT SETUP DONE"
