# LaserFoundry

**An open-source, clean-room laser engraver/cutter application in pure Rust, aiming at a
LightBurn-class workflow.** Lay out artwork, give each colour layer its own speed, power, passes and
fill settings, preview the toolpath, and stream GRBL G-code to your machine, or to the built-in
simulator when nothing is plugged in.

> **Status: early.** The core pipeline works end to end and is well tested, but this is a small
> fraction of what LightBurn does. See [`ROADMAP.md`](ROADMAP.md) and
> [`docs/target-app-parity.md`](docs/target-app-parity.md) for an honest list of what exists and what
> doesn't. LaserFoundry is independent of and unaffiliated with LightBurn Software.

## What works today

- **Images:** shown on the canvas as real thumbnails (rotation and flips included); brightness, contrast, gamma and negative adjustments (non-destructive, applied before dithering), crop and sharpen (undoable), and tracing of dark areas into vector outlines; a picture flush with the bed edge gets a shorter lead-in rather than a job that cannot run.
- **Arrange:** grid and circular arrays (bounded copy count), rounded rectangles, join paths, and **nesting**: pack the selection into the bed (or a region) without overlaps, with a gap between parts and optional turning, keeping clear of the shapes that stay and, if asked, putting small parts inside the holes of big ones; groups move as one part.
- **Draw and edit curves:** a Pen tool (click corners, drag out smooth nodes, click the first node to close) and a Nodes tool (drag nodes and handles, double-click a segment to insert, smooth/corner, line/curve, open/close); text converts to editable paths.
- **Combine:** boolean union / subtract / intersect / XOR of closed shapes (holes preserved), and offset (grow, shrink, kerf-style; round, miter or bevel joins; open lines and stroke text become an outline around the line).
- **Create:** rectangle, ellipse, line, regular-polygon and text tools (editable multi-line text in an original single-line stroke font, or filled outlines from a .ttf/.otf file you choose (never bundled): size, letter/line gap, alignment; click anywhere in the block to select); drag, rubber-band select, corner-handle resize, nudge, align, flip, rotate, duplicate, undo/redo (200 steps).
- **Import:** SVG (shapes, groups, transforms, units; stroke/fill colours map to cut layers), DXF (LINE, LWPOLYLINE with bulges, POLYLINE, CIRCLE, ARC; `$INSUNITS` honoured; DXF layers become cut layers), and raster images (PNG/JPEG/BMP/GIF/WebP).
- **Cut layers:** 16 layers, each with Line / Fill / Fill + Line mode, speed, min/max power, passes, fill interval and angle, kerf offset (outlines are compensated for beam width: parts grow, holes shrink), bi-directional scanning, air assist, and image dithering (threshold, Floyd–Steinberg, Atkinson, Jarvis, ordered, halftone, grayscale).
- **Toolpath:** inner shapes cut before outer ones, nearest-neighbour ordering and duplicate-line removal (each switchable), serpentine scan fill with holes, raster engraving with overscan, and "cut selected shapes only", a toolpath preview with a play/scrub timeline, a head marker and a time breakdown per layer.
- **Output:** GRBL G-code (`M4` dynamic power or `M3`, configurable `$30` S range, bottom-left or top-left origin, bed-bounds refusal), frame-the-job, save G-code.
- **Job origin:** Absolute coordinates, User origin, or Current position, with a 9-point anchor on the job bounds, so work can be placed on material wherever the head is (`G92` for current position).
- **G-code:** GRBL (tested against the simulator) plus Marlin-laser and Smoothieware dialects written from their public docs (untested on hardware).
- **Machine control:** a persistent GRBL connection with live status and position, jog pad (direction follows the machine's origin corner), home, unlock, set origin here, go to origin, pause/resume (feed hold), stop (soft reset, laser off), a console with a command line, a controller settings (`$$`) editor that flags laser mode off and S max or travel that disagree with the document, and character-counting job streaming with progress. Serial ports are behind the `serial` feature (on by default in the desktop app); a built-in simulated controller needs no hardware and checks the 128-byte RX buffer is never overrun.
- **Files:** native `.laserfoundry` (versioned JSON, atomic saves, validated on load). Unsaved work is autosaved every 20 s to a per-user recovery file and offered back after a crash; New, Open (and File > Open Recent) and closing the window ask before discarding changes; the title shows `*` while there are unsaved changes.
- **AI (desktop and browser):** Design from Prompt and Redraw Image draw vector artwork whose colours say what the laser does (red cut, blue score, black engrave); Ask About Lasering answers with your machine and layers as context; Check My Cut reads photos of a test cut and suggests speed, power and pass changes you can apply. Requests go to your own Anthropic key or to an account service that takes credits (the same account as StitchFoundry's), after a first-use question saying what is sent. Written and tested against scripted answers and a local fake service, not yet run against the real one. See [`docs/ai.md`](docs/ai.md).
- **Headless:** `laserfoundry-cli` converts files to G-code, runs JSON command scripts, and `laserfoundry-cli mcp` serves every command to AI agents as Model Context Protocol tools.

## Quick start

```sh
git clone https://github.com/Stitchwright/laserfoundry
cd laserfoundry

cargo run --release -p laserfoundry                         # desktop app
cargo run --release -p laserfoundry -- design.svg           # open an SVG
cargo run -p laserfoundry-cli -- gcode design.svg -o job.gcode --bed 300x200 --origin top_left \
    --set 2:speed=900,max_power=40                          # headless conversion
apps/laserfoundry-web/build.sh                              # browser build (design, open/save as files, preview, G-code, Web Serial in Chromium browsers)
cargo xtask ci                                              # fmt, clippy, tests, layers, clean-room
```

Linux needs the usual windowing libraries for eframe (X11 or Wayland, `libxkbcommon`, a GL driver).
Serial ports use the `serialport` crate without `libudev`; you may need to be in the `dialout` group.

> **Not yet verified on real hardware.** Everything machine-side has run against the built-in
> simulator only. Use Frame and low power first, and stay present.

> **Safety:** a laser is a fire and eye hazard. Test with the simulator and the Frame button first,
> never leave a running machine unattended, and wear appropriate eye protection. This software is
> provided without warranty (see the licences).

## Design

A layered Rust workspace, "everything is a command", no panics in shipped code, and a thin UI.
See [`docs/architecture.md`](docs/architecture.md). Contributor and agent rules are in
[`AGENTS.md`](AGENTS.md).

## Reporting bugs

In the app, **Help → Report a Bug…** opens StitchHub's report page, filled in with the version and your system.
Describe what happened and send it: it becomes an issue in this repository (a public one, so leave out anything
private). You can also [open an issue](https://github.com/Stitchwright/laserfoundry/issues) yourself. A maintainer can
have Claude work on an issue by labelling it `claude-fix`; Claude opens a pull request for review, and never merges.

## Licence

MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE`).
