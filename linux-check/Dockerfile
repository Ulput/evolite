# Linux check: builds evolite and its installer, runs the tests, takes screenshots on a virtual X
# screen and cross-builds for 64-bit ARM. See linux-check/README.md. System fonts (DejaVu, Noto
# CJK) are there because the renderer draws text in SVGs with the system's fonts, as on a desktop.
FROM rust:1-bookworm

RUN apt-get update && apt-get install -y --no-install-recommends \
      xvfb xauth unzip clang \
      libx11-dev libxcursor-dev libxrandr-dev libxi-dev libxkbcommon-dev libxkbcommon-x11-0 \
      libgl1 libegl1 libgl1-mesa-dri libwayland-dev \
      fonts-dejavu-core fonts-noto-cjk \
      gcc-aarch64-linux-gnu g++-aarch64-linux-gnu libc6-dev-arm64-cross \
    && rm -rf /var/lib/apt/lists/* \
    && rustup target add aarch64-unknown-linux-gnu

# protoc 32.0, as on Windows and in Platform's CI: Debian's 3.21 is too old for
# tenderdash-proto. The x86_64 archive is checked against its SHA-256.
RUN arch="$(dpkg --print-architecture)"; \
    case "$arch" in \
      amd64) p=linux-x86_64; sum=7ca037bfe5e5cabd4255ccd21dd265f79eb82d3c010117994f5dc81d2140ee88 ;; \
      arm64) p=linux-aarch_64; sum= ;; \
    esac; \
    curl -sSfL -o /tmp/protoc.zip \
      "https://github.com/protocolbuffers/protobuf/releases/download/v32.0/protoc-32.0-$p.zip" \
    && if [ -n "$sum" ]; then echo "$sum  /tmp/protoc.zip" | sha256sum -c -; fi \
    && unzip -q /tmp/protoc.zip -d /opt/protoc && rm /tmp/protoc.zip

WORKDIR /work
ENV CARGO_TARGET_DIR=/work/target-linux LIBGL_ALWAYS_SOFTWARE=1 PROTOC=/opt/protoc/bin/protoc \
    CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
    CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc \
    CXX_aarch64_unknown_linux_gnu=aarch64-linux-gnu-g++
