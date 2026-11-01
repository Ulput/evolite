# The renderer for external pictures

NFT pictures, avatars and anything else that comes from outside are turned into RGBA frames in a
separate process. The main process (the core with the keys, and the interface) never parses
untrusted bytes.

```
core (keys, network)                  evolite.exe --render-worker
  picture bytes  ─── stdin ────────▶  format by the first bytes
                                       parsing with limits, rasterising
  answer checked ◀── stdout ────────  RGBA frames or a refusal code
```

The `crates/evolite-render` crate:

| Module | What it does |
|---|---|
| `sniff` | the format by the first bytes: PNG, JPEG, GIF, WebP, SVG; anything else is refused |
| `packing` | unpacking: raw deflate (by a request flag) and gzip (`.svgz`, by its signature), no more than `max_input` |
| `raster` | `image` with `image::Limits`: the size from the header is checked before memory is allocated; GIF, WebP and APNG animations come as frames with delays |
| `svg` | usvg/resvg with no way out (below) |
| `smil` | animated SVG as frames (below) |
| `frames` | `Art`, `Limits`, `RenderError`; `Art::check` |
| `wire` | the binary protocol: an answer is checked against the limits before memory is allocated; errors are codes from a fixed list, text from the renderer is not accepted |
| `worker` | the renderer's loop; a decoder panic is a refusal, not a crash |
| `client` | `Renderer`: one process, a time limit per picture, a restart after any failure |
| `job` | a Windows Job Object: a memory limit, no child processes, no clipboard or desktop, dies together with the wallet |
| `sandbox` | starting the renderer on Windows through `CreateProcessW` (below) |

## The sandbox on Windows

`std::process::Command` can do neither AppContainer, nor a suspended start, nor mitigation
policies, so the process is started by our own code:
- **AppContainer `evolite.renderer` with no capabilities:** no network (not even `127.0.0.1`), no
  access to the user's files and registry, low integrity level. The profile is created on the
  first run on a machine; the container is granted read and execute on `evolite.exe` itself (an
  ACE for the container's SID only). For an install into Program Files the installer must do this.
- **Created suspended**, put into the Job Object and only then resumed — not a single instruction
  runs outside the limits.
- **Inherits only two pipes** (an explicit handle list), whatever the wallet has open.
- **Mitigation policies from the first instruction:** no dynamic code (executable memory cannot
  be allocated), extension points disabled (old DLL injection routes), no images from the network
  or with low integrity, strict handle checks, termination on heap corruption, ASLR.
  **Win32k is closed** (no window and graphics system calls — the biggest attack surface on the
  Windows kernel) in the separate `evolite-renderer.exe`: it sits next to `evolite.exe`, is built
  together with it (the second `[[bin]]` of the `evolite-ui` package) and is not linked with
  window code. Test: it draws text with Win32k closed, while `evolite.exe` itself does not start in
  that mode — so the restriction really applies. If `evolite-renderer.exe` is not next to it, the
  wallet starts itself with `--render-worker`, and Win32k is then open. The installer must ship
  both files.
- System fonts are available to the container — text in SVG is drawn.

## SVG

By default usvg reads `<image href="…">` from disk: an untrusted SVG would show a user's file,
and a `\\host\share\…` path on Windows would hand an NTLM hash to someone else's server. Here:
- paths and URLs are never opened;
- `data:` — only PNG, JPEG, GIF, WebP (by their bytes), no larger than `max_source_side`; a nested
  SVG is not parsed;
- DTDs and entities are forbidden, no more than `max_svg_nodes` XML nodes;
- the canvas is no larger than `max_side`, larger art is drawn scaled down;
- system fonts are read before the first untrusted byte.

## Compressed SVG

- **Raw deflate** — how Jembe stores an NFT picture in a document (`imageFormat` 1; `imageHash`
  is the SHA-256 of the *uncompressed* SVG, which is also the picture's key in evolite). Such a
  stream has no signature, so the document says what it is: `Packing::RawDeflate` → the
  `RAW_DEFLATE` request flag. Without the flag the same bytes are "not a picture".
- **gzip** (`.svgz`) is recognised by its first bytes; only SVG may be inside, and the CRC and
  length are checked.
- Unpacking happens in the renderer, in the sandbox, like any decoding (`packing.rs`,
  `miniz_oxide` — pure Rust with no `unsafe`), no more than `max_input` (16 MB): a 60 KB "bomb"
  that would expand to 64 MB stops at the limit without memory for the rest
  (`Limit("unpacked size")`). Nothing is unpacked twice.
- In the demo the Jembe samples are stored as in the documents: raw deflate. When the real core
  checks a picture against `imageHash`, it will have to unpack too — the same way and with the
  same limit.

## Animated SVG (SMIL and CSS)

resvg draws one moment. The `anim` module (`smil` and `css`) computes the values of the animated
attributes for each frame, writes them into the SVG text and sends it down the same closed path as
a static picture.

Understands:
- `<animate>`: numbers, number lists (the path `d`, `points`, `viewBox`), colours;
  `from`/`to`/`by`/`values`, `keyTimes`, `calcMode` linear and discrete; values of different
  shapes jump at the midpoint;
- `<animateTransform>`: translate, scale, rotate, skewX, skewY, `additive="sum"`;
- `<set>`;
- `begin` (a time), `dur`, `repeatCount`, `repeatDur`, `fill="freeze"`; the target is
  `href="#id"` or the parent.
- CSS `@keyframes` in `<style>`: `animation` and its separate properties, selectors (simplecss),
  `transform` (translate, scale, rotate, skew, matrix) with `transform-origin` and
  `transform-box: fill-box`, `opacity`, `fill`, `stroke` and other number and colour properties;
  `ease*`, `cubic-bezier`, `steps`, `linear`; `infinite`, `alternate`, a delay.

Does not understand (drawn at rest): `<animateMotion>`, an event `begin` ("on click"),
CSS `:hover`; `calcMode="spline"` runs linearly. The `id`, `href`, `style` and `class` attributes
are not available to animations.

Frames: a loop is a whole number of the common period of all repeating animations (no jerk at the
seam), no longer than 10 s; a one-off animation runs to its end, and the last frame holds for 2 s.
One-off changes (`<set>` at second 1) repeat on every turn of the loop: frames have no "intro".
Up to 20 frames per second, a side of up to 512, no more than 300 frames and 192 MB; identical
frames in a row are merged into one longer frame. If frames are slow to draw, there are fewer of
them (a 3 s budget), and the tempo is kept. Frames are drawn in parallel, a thread per core, at
most four: Web Shot — 0.6 s instead of 3 s, InstantSend — 0.6 s instead of 1.6 s (and all 200
frames instead of 166), Coffin Dance — 0.14 s instead of 0.5 s (a warm renderer, 400 px). Threads
do not get in the sandbox's way: on Linux `fork`/`vfork` are forbidden, but not `clone`, which
creates threads. More than 1000 animations in a file — drawn as a static picture.

To see the result: `cargo run -p evolite-render --example preview -- file.svg out.gif
strip.png` — an animated GIF and a strip of eight frames (in this process, for your own files).

## 3D models (GLB)

Binary glTF 2 (`glTF` in the first bytes; Jembe suggests storing it as `imageFormat` 2 — GLB in
raw deflate, the same `RAW_DEFLATE` flag). It is shown "on a turntable": the model makes a turn in
6 s, frames on the same 50 ms grid as SVG — 120 frames.

**Parsing** (`glb/parse.rs`, our own, without the `gltf` crate): only the file's own BIN chunk —
a `uri` of any kind (a path, a URL, even `data:`) is rejected; no images and textures, animations
or sparse accessors; required extensions only if known (`KHR_mesh_quantization`,
`KHR_materials_*`), otherwise a refusal. Every offset and length is checked against the chunk
before reading, the node tree is walked without cycles (depth up to 32, up to 1000 nodes), no more
than 100,000 triangles. Normalised integer positions (quantisation) and `COLOR_0` are understood.

**Drawing** (`glb/draw.rs`) is on the CPU: the sandbox has no graphics (Win32k is closed), and a
few hundred triangles do not need them anyway. A Z-buffer, flat faces (Jembe files have no normals
— a low-poly style), vertex colours, a small "studio" of three lights and a sky for reflections
(otherwise metal is black), transparent faces (a gem with `transmission`) after the opaque ones,
far to near; anti-aliasing — drawing at double size and scaling down; the ACES tone curve, as in
Jembe's three.js. A 35° camera, like theirs.

It is fast because:
- lighting is computed once per face corner and spread across the face (a face is flat — one
  normal), not for each of a pixel's four samples;
- each thread has its own canvas for the whole turn; before a frame only the rectangle the
  previous frame drew is cleared;
- whether a pixel belongs to a triangle, and its depth, are computed incrementally (one addition
  per pixel);
- up to 4 threads (each holds a canvas: 8 MB at 512 px).

**Frames stay compressed from the renderer to the window.** A thread compresses a frame (LZ4) as
soon as it is drawn; compressed frames go down the pipe (`PackedArt`, each no longer than LZ4
could produce — this is checked before memory is allocated); the core passes them to the window
as they are, without recompressing; the window unpacks only the frame on screen and checks that it
is exactly its size. SVG pictures and rasters are compressed the same way, after decoding.

Measured (the Jembe ring, 436 triangles, 22 cores):

| | 256 px | 384 px | 512 px |
|---|---|---|---|
| first frame | 2 ms | 5 ms | 7 ms |
| a turn, 120 frames | 0.06 s | 0.1 s | 0.17 s (was 1.3 s) |
| renderer peak memory | 21 MB | 37 MB | 42 MB (was 149 MB) |
| frames compressed | 1.4 MB | 2.3 MB | 3.2 MB (was 120 MB down the pipe) |

Renderer CPU at 512 px: ~0.6 s per turn instead of ~5 s. The window playing "Collection" with the
ring uses ~0.1 of a core, as without it.

## Display size

The renderer starts together with the wallet (`Renderer::start` in the core's picture thread):
the process, the sandbox and the fonts are ready before "Collection" is opened, and the first
picture does not wait ~1.3 s.

`Renderer::render_at(bytes, side)`: SVG is drawn so that its long side is exactly `side` (larger
or smaller than its own size), a raster no larger than `side`. The limits on frame sides and
animation frame sides are lowered to `side` for this request (`Limits::for_side`), so a small card
does not drag 512 px frames along. `render(bytes)` — the picture's own size.

## Collection

An NFT in the API carries not the picture's bytes but its key — a SHA-256 (`Nft::art`). The
interface decodes nothing: for each card it asks for `Command::DrawArt { art, side }`, where
`side` is the card's width in screen pixels (accounting for the scale), rounded up to 32, at most
1024. The core (`evolite-core/src/art.rs`) draws pictures one at a time on a separate thread
through the sandboxed renderer (`evolite.exe --render-worker`) and answers with `Event::Art`
carrying frames (`Picture`). The interface (`evolite-ui/src/art.rs`) turns frames into textures,
runs the animation on egui's clock and asks for repaints only for visible cards. While a picture
is being drawn there is a "Drawing…" placeholder (or the same picture at its previous size); on a
refusal — "Could not show it" with the reason. The real core answers `DrawArt` with "Platform is
not connected yet".

Order in the core: first pictures from the disk cache; then the first frame of each picture
(`Renderer::render_first`, the `FIRST_FRAME` request flag) — cards on screen before the others
(`urgent`), and it is shown at once (`Event::Art { partial: true }`); then whole animations. A
single frame with `delay_ms` 0 is a static picture; nothing follows it.

The disk cache (`evolite-core/src/art_cache.rs`): the user's cache folder
(`%LOCALAPPDATA%\evolite\art\r<renderer version>`, `~/.cache/evolite/…`,
`~/Library/Caches/evolite/…`), a file per key and size, with frames compressed (LZ4) the way the
interface takes them. The core writes it, not the renderer; on reading, sizes are checked against
the limits, a damaged file is deleted and the picture is drawn again. Over 256 MB, the least
recently opened files are deleted down to 192 MB. A new renderer version (`RENDER_VERSION`) means
a new folder.

The renderer draws only the moments when something changes: if a frame's SVG text matches the
previous one (`steps()`, discrete SMIL, `<set>`, pauses), the frame is not drawn and the previous
one is held longer. Threads for frames — half the cores, from 1 to 8 (`default_threads`): 2 cores
— 1 thread, 4 — 2, 8 — 4, 16 or more — 8; the rest is left to the window, the sync and other
programs.

Playback: animation frames sit on a grid of exactly 50 ms from the start of the loop (the loop's
last frame may be shorter), and the window changes the frames of all animations on one shared
50 ms tick — no more than 20 repaints a second, with no frames lost or doubled. A frame is chosen
for the moment it is shown (`time + predicted_dt`), and `predicted_dt` is added to the repaint
delay: egui subtracts it itself, and a delay shorter than a frame turned into two immediate
repaints — the window was drawing 60 times a second. Frames come with premultiplied alpha (as the
texture takes them, without a pass over the pixels; `RENDER_VERSION` 2). In a minimised window
animations stop. The demo core wakes the window only when there is something to show.

Measured (22 cores): playing eight cards — ~0.1 of a core (was ~0.85), "Overview" at rest — 0;
the renderer without a cache — a peak of ~3 cores for ~2 s and up to 56 MB, at rest 2–7 MB; the
window on "Collection" — 113–133 MB against ~100 MB on "Overview".

The demo has eight NFTs: three of our own and five Jembe samples (Coffin Dance, Web Shot and
InstantSend — CSS-animated, Anonymous Hacker — blinks, Teddy — static). Measured (22 cores,
8 threads): without a cache the first frames of all eight take ~0.35 s and the animations are
ready in ~3.7 s; from the cache all eight take ~0.1 s, 21 MB on disk.

In "Collection", pictures that have been off screen for a minute give back their memory (textures
and frames) and come back from the disk cache when they return. Settings → Appearance →
"Collection": "NFT animation" (as the system says — on Windows "Show animations in Windows" —,
on, off; off means the first frame and not a single extra repaint) and "Ask before showing": an
NFT picture from an issuer the wallet does not trust (out of the box it trusts only `jembe`) is
not drawn, not even in the sandbox, until the person presses "Show" on the card or "Always show
from …" in the panel. In the demo such an NFT is Glitch Skull from `neon-forge`. There is no
picture in the selected NFT's panel in the Figma mockup either (the "18 · Collection" frame).

## Default limits

Input 16 MB; a source raster up to 8192²; frames up to 2048², up to 300 of them, up to 192 MB in
total; 50,000 SVG nodes; 1000 SVG animations, a loop up to 10 s, up to 20 frames/s, an animation
frame side up to 512, a 3 s budget for frames; a 5 s time limit per picture; renderer memory
256 MB on Windows (512 MB of address space on Linux and macOS).

## Covered by tests

`cargo test -p evolite-render` and `cargo test -p evolite-ui --test render_worker`:
- a PNG with a 100,000 × 100,000 header — refused before decoding;
- an SVG linking a local file, `file:///` and `\\127.0.0.1\share` — nothing is read;
- a nested SVG, entities, too many nodes, a huge canvas;
- a hung renderer is stopped by the time limit; a crashed one is reported and the next one is
  started afresh; one that claims frames over the limit is rejected; one that takes memory over the
  limit is terminated by Windows (without the limit the test would see a timeout);
- `evolite.exe --render-worker` itself answers as a renderer, and text in SVG is drawn;
- `evolite-renderer.exe` with Win32k closed draws text, `evolite.exe` with Win32k closed does not
  start;
- the whole picture path (`crates/evolite-ui/tests/art_path.rs`): the core asks the renderer, the
  first frame arrives, then the animation, and on a second run the same picture comes from disk
  without the renderer;
- fuzzing (`crates/evolite-render/tests/fuzz.rs`, stable Rust): demo pictures, SMIL, CSS, PNG and
  GIF are broken by mutations (bytes, insertions, a dictionary of values such as `steps(0)`,
  `1e309`, `keyTimes="0;2;1"`), plus random bytes into the protocol. A quick pass runs with the
  tests, a long one with `EVOLITE_FUZZ_ITERS=3000` (36,000 pictures, ~75 s): no crashes in our
  code; the renderer catches 5 crashes inside tiny-skia (`alpha_runs.rs`) and svgtypes
  (`transform-origin`) and refuses the picture. To replay a case:
  `EVOLITE_FUZZ_CASE=<file> … replay -- --ignored`;
- escape (`tests/sandbox.rs`): a test renderer tries to read a user's file, connect to a port on
  `127.0.0.1`, write a file and allocate executable memory. Without the sandbox:
  `read=ok net=ok write=ok exec=ok`; in the sandbox:
  `read=denied net=denied write=denied exec=denied`;
- the first run on a machine: `cargo test -p evolite-render forget_the_container -- --ignored`
  deletes the container profile, after which the tests run in parallel and create it again;
- SMIL: a square moves, a colour blends, a bar turns a quarter in a second, `<set>` fires on time,
  `to` starts from the element's own value, `keyTimes` in discrete mode, an event `begin` — static,
  too many animations — static, a 7 s and 11 s loop is cut to 10 s, slow frames — fewer frames, a
  value usvg rejects keeps the previous frame, values are escaped in the text; an animated SVG goes
  through `evolite.exe` in the sandbox and arrives as frames.

## Next

1. Linux and macOS: build and run the sandbox and thread tests there (the code exists, untested;
   needs Docker or a machine). The system "reduce motion" setting is not read there yet.
2. Fuzzing with libFuzzer (`cargo-fuzz`, needs nightly) — deeper than mutations on stable.
3. Report the crashes found to tiny-skia and svgtypes (the renderer catches them).
4. Repainting only the changed area of a frame (SVG).
5. SMIL: `<animateMotion>`, "an intro, then a loop".
6. GLB: textures (pictures inside the model — through the same raster decoder with limits),
   handing out frames one at a time while the rest are drawn; drawing the wallet's NFTs ahead of
   time, while idle.
7. Trusted issuers and "Show" are not kept between runs (settings are not saved at all yet).
