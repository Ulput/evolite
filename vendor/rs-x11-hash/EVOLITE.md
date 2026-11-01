rs-x11-hash 0.1.8 from crates.io, vendored by evolite for Windows MSVC builds:
- src/x11/sph_types.h: `#include <unistd.h>` only outside _WIN32 (nothing from it is used);
- build.rs: clang only off MSVC, the GCC warning flag only where supported.
Remove once rust-dashcore moves past aca0202 (its X11 is pure Rust, dash-pow).
