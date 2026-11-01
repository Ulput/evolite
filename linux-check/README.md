# Checking on Linux

Debian 12 in Docker, an Xvfb virtual display, Mesa software rendering, protoc 32.0, clang (the
X11 hash), system fonts (DejaVu, Noto CJK) and a cross compiler for 64-bit ARM. Run the commands
from the repository root; `<folder>` is where screenshots and the packed installer go. Cargo takes
Platform and rust-dashcore from GitHub into the `evolite-cargo-git` volume.

```
docker build -t evolite-linux-check linux-check
docker run --rm -v "$PWD":/work -v <folder>:/out \
  -v evolite-cargo-registry:/usr/local/cargo/registry -v evolite-cargo-git:/usr/local/cargo/git \
  -v evolite-target-linux:/work/target-linux evolite-linux-check bash -c '
    cargo test --workspace &&
    cargo build --release --bin evolite --bin evolite-renderer --bin evolite-setup &&
    cargo test --release -p evolite-ui --bin evolite-setup -- --include-ignored &&
    (cd target-linux/release && ./evolite-setup --pack /out/evolite-setup-linux-x86_64) &&
    xvfb-run -a target-linux/release/evolite-setup --screen space --lang ru --screenshot /out/setup-space.png &&
    cargo build --release --features evolite-ui/dev-tools --bin evolite &&
    xvfb-run -a target-linux/release/evolite --screen probe --screenshot /out/probe.png'
```

- The build folder is a Docker volume (`evolite-target-linux`): a Windows folder mounted into
  the container is many times slower to build in.
- `--include-ignored` runs the installer's live install cycle too: inside the container it only
  writes to a temporary folder.
- The installer is packed from the release build without `dev-tools`. `--screen`, `--screenshot`
  and the `probe` screen of evolite are in the `dev-tools` build only (`docs/development.md`,
  "Screenshots and checks"), so evolite is built once more with it for the last screenshot; the installer has its
  own `--screen` for its pages.
- 64-bit ARM: `cargo build --release --target aarch64-unknown-linux-gnu --bin evolite …` in the
  same container (the linker and C compilers are set in the image).
- The `probe` screen checks writing systems: Hindi, Arabic right to left, numbers.
