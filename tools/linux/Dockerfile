# Linux build environment matching the CI apt packages (see .github/workflows/build.yml).
FROM rust:1-bookworm
RUN apt-get update && apt-get install -y --no-install-recommends \
      pkg-config clang libgtk-3-dev libayatana-appindicator3-dev libudev-dev libxkbcommon-dev \
      libxcb1-dev libxrandr-dev libdbus-1-dev libpipewire-0.3-dev libwayland-dev libegl-dev libgbm-dev \
    && rm -rf /var/lib/apt/lists/*
RUN rustup component add clippy
