//! The renderer the app ships next to `evolite.exe`. It holds nothing but the worker: no
//! window code is linked in, so on Windows it runs with the window and graphics system calls
//! (Win32k) turned off, the kernel's largest attack surface for a sandboxed process. Without
//! it, the app falls back to starting itself with `--render-worker`, Win32k on.

fn main() {
    evolite_render::worker::run();
}
