# Vendored crate patches

Unmodified copies of `eframe` 0.36.2 and `epaint` 0.36.2 with a few small changes, wired in via
`[patch.crates-io]` in the workspace `Cargo.toml`.

- `eframe/src/lib.rs`, `eframe/src/native/wgpu_integration.rs`: `eframe::KEEP_ROOT_UI`. eframe skips
  the root UI pass while the root window is minimized or hidden, so a minimized app could never open
  a new popup viewport. With the flag set the pass still runs (painting is still skipped).
- `eframe/src/native/{wgpu_integration,run,winit_integration}.rs`: never present to a hidden or
  minimized window. winit doesn't report `Occluded` on Windows, so a root hidden with
  `Visible(false)` (tray) was still painted, and `get_current_texture` stalls on a hidden window
  (DXGI frame-latency wait / Vulkan acquire), freezing the event loop. Also: while a child viewport
  (popup) is visible, the hidden root is not throttled to 100 ms, so popups stay at full frame rate.
- `epaint/src/text/font.rs`: `epaint::text::set_glyph_rasterizer`. egui's own rasterizer only hints
  glyphs vertically (emilk/egui#8034, #8079), which leaves CJK stems blurry on Windows. The app
  installs a GDI rasterizer that applies the font's full hinting; unknown fonts fall back to egui.

## egui-wgpu

`Renderer::update_buffers` panics when `Queue::write_buffer_with` returns `None` (seen on Windows
after a GPU device loss / long runs: emilk/egui#8265, #8450). With `panic = "abort"` that kills the
app. The patch sets `Renderer::buffers_failed` instead, and `Painter::paint_and_update_textures`
skips that frame and retries 250 ms later.
