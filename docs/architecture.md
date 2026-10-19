# Architecture

> **Last updated:** 2026-10-13 · **Stage:** M0, the vertical slice (import → layers → toolpath → G-code → stream)

How LaserFoundry is built today. Rules (clean-room, never-crash, quality gates) are in
[`AGENTS.md`](../AGENTS.md).

## Shape

A Cargo workspace of 13 library crates under `crates/`, two apps under `apps/`, and `xtask`. Pure
Rust. The same engine runs the desktop app and the headless CLI.

## Layers

`cargo xtask layers` (part of `cargo xtask ci`) enforces that a crate depends only on lower layers
and that nothing below L7 depends on a UI toolkit. The table lives in `xtask/src/layers.rs`.

| Layer | Crates     | Role                                                                                             |
|-------|------------|--------------------------------------------------------------------------------------------------|
| L0    | `geom`     | polylines, flattening, bounds and hit-testing on `kurbo`                                         |
| L0    | `i18n`     | translation catalogs (`tr`, `tr_fill`, engine-message templates), shared by the UI and the CLI    |
| L1    | `doc`      | the document: cut layers, shapes (paths and rasters), the machine profile; data + serde only     |
| L2    | `toolpath` | document to `Job`: ordering, scan fill, raster dithering, time estimate                          |
| L2    | `text`     | an original single-line stroke font (glyph table + corner-cutting smoothing) and text layout    |
| L1    | `pathops`  | boolean combine and offset on flattened shapes (wraps `i_overlay`: even-odd regions with holes, validated and bounded inputs) |
| L3    | `svg`      | SVG import (`usvg` resolves shapes, groups, transforms, units)                                   |
| L3    | `dxf`      | ASCII DXF import written from the public reference                                               |
| L3    | `gcode`    | `Job` to GRBL G-code with a `Placement` (absolute, or relative to a job anchor); bed-bounds and finiteness checks; framing |
| L3    | `format`   | the native `.laserfoundry` file: versioned JSON, atomic save, validation on load                 |
| L3    | `device`   | `Transport` trait, GRBL character-counting streamer (`JobStream`), status parsing, the persistent `Link` (worker thread: status polling, jog/home/unlock/zero, job streaming, pause/resume/stop, console), simulated controller, optional serial port (`serial` feature) |
| L3    | `fontlib`  | downloads the font library (stitchfoundry-fonts) into the data folder: tree listing + raw files, caps, safe paths. Desktop app and CLI only (xtask layers enforces it) |
| L5    | `engine`   | `Session` (document, selection, undo/redo), the command registry, the panic guard                |
| L6    | `mcp`      | a Model Context Protocol server over stdio: every engine command is a tool                       |
| L7    | `ui-egui`  | menus, tools, canvas, panels; talks to the engine only through `execute`                         |
| L8    | `apps/laserfoundry`, `apps/laserfoundry-cli`, `xtask` | the eframe desktop app (native dialogs, serial), the CLI, build tooling |

## Everything is a command

Every user-visible action is a `CommandSpec` in `crates/engine/src/cmd/*`: id, label, menu path,
shortcut, a parameter description, whether it mutates, and `run`. Mutating commands are one undo
step; if they return an error or panic, the document is restored. `command.batch` runs several as
one atomic undo step. The UI, the CLI's `run` script and any future control channel or MCP server
reach the same registry, so anything a person can do, a script or agent can do.

`cargo run -p laserfoundry-cli -- commands` prints the live list.

## Coordinates and units

Everything is millimetres, **y down from the bed's top-left** in the document and in the UI. The
machine profile (`Origin::BottomLeft` or `TopLeft`) is applied only when G-code is written. Shapes
store local geometry plus an affine transform; world-space outlines are flattened on demand
(tolerance 0.02 mm).

## Planning and output

`toolpath::plan` walks output-enabled layers in id order. Per layer: scan fills first, then outlines (closed outlines are first offset by the layer's kerf compensation via `pathops`; open paths are left as drawn)
(inner shapes first, then nearest neighbour), then images (sampled on a world grid at the layer's
interval, dithered, emitted as scan rows with zero-power overscan). The result is a `Job` of
machine-independent `Move`s (`Travel`, `Cut { power % }`). `gcode::generate` maps power to
`S` (0–`max_s`), flips Y for a bottom-left origin, drops redundant words, repeats each op `passes`
times, and refuses out-of-bed or non-finite coordinates.

## Job origin

`Document::job_origin` (`start_from` + a 9-point `anchor`) decides how design coordinates become
machine coordinates. *Absolute*: workspace = machine coordinates, bed-checked point by point.
*User origin* / *Current position*: coordinates are relative to the anchor point of the job's
bounds, which sits at work-zero; the job's extent must fit the bed (it cannot be checked point by
point because zero is wherever the user put it). *Current position* also emits `G92 X0 Y0`. The
command is `job.set_origin`; the setting is saved in the project file (older files load with
defaults).

## Talking to a machine

`device::Link` owns a transport on a worker thread. It polls status (`?`, a real-time byte that does
not use the RX buffer), sends immediate lines (`$J=` jogs, `$H`, `$X`, `G92`), and streams jobs with
GRBL's character-counting protocol (`JobStream`): keep as many lines in flight as fit in the 128-byte
RX buffer and free space as `ok` arrives. `error:N` and `ALARM:N` abort the job with the offending
line and soft-reset the controller; stop sends a soft reset (0x18) so the laser goes off; pause and
resume are feed hold (`!`) and cycle start (`~`). A job is not "finished" until every line is
accepted *and* the controller reports Idle. Immediate commands are refused while a job runs, lines
are validated (single line, bounded length, no embedded reset byte), jogs are bounded, and dropping
the link mid-job soft-resets. Jog/home/zero are UI-level machine actions, not document edits, so
they are `Link` methods rather than engine commands; the document-affecting parts (`job.*`) are
commands. `SimulatedGrbl` models the RX buffer, jogging, homing, `G92` and feed hold.

## Robustness

No panics in shipped code (workspace clippy lints). `engine::guard` catches a panic at every
command and rolls the document back. Importers and parsers cap input sizes and counts; the engine's
`command_sweep_never_panics` test fires hostile parameters at every registered command and checks
document invariants afterwards.

## Autosave and recovery

While the document has unsaved changes the UI writes a recovery copy (`file.autosave`, through the
same atomic writer as Save) at most every 20 s, and schedules a repaint so an idle window still gets
there. The copy is removed once the work is saved or knowingly discarded. A leftover copy found at
startup is offered back (`file.recover` reopens it untitled and unsaved) and is not overwritten
while the question is open. The location comes from the `Platform` (the desktop app uses the
per-user data folder).

## UI

`ui-egui` is a thin front end: tools turn gestures into commands (`shape.*`, `edit.move`,
`command.batch` for corner-handle resize), panels read the session and call `layer.set`,
`machine.set`, `job.*`. Derived data (flattened outlines, the preview) is cached per
`Session::revision`. File dialogs and serial ports come in through the `Platform` trait, so the UI
crate has no OS dependencies beyond egui. Headless frame tests in `crates/ui-egui/src/tests.rs`
feed real pointer events through the real layout.
