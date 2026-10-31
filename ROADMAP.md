# Roadmap

> **Stage:** M1 software complete, hardware verification pending · **Next:** run on a real GRBL machine (gap 1)

| Milestone | Goal                                                                                       | Status |
|-----------|--------------------------------------------------------------------------------------------|--------|
| M0        | Import, layers, toolpath, G-code, simulated streaming, desktop app, CLI, quality gates     | done   |
| M1        | Jog/home/position, job origin, pause/resume (done, simulator-tested); real-hardware verification (gap 1) | in progress |
| M2        | Design tools: text, node edit, offset, booleans, snapping (gaps 4–7)                       |        |
| M3        | Planning quality: cut-order options, lead-ins, tabs, image adjustments, timeline preview   |        |
| M4        | Workflow: groups, autosave/recovery, material test grids, agent control (MCP)              |        |

See [`docs/gaps.md`](docs/gaps.md) for the ranked work list and
[`docs/target-app-parity.md`](docs/target-app-parity.md) for the self-assessment.

## Progress log

- **2026-10-13 — M0.** Workspace scaffolded on the VectorCraft/CADCraft conventions. Geometry,
  document model, toolpath planner (ordering, scan fill, dithered raster), GRBL G-code generator,
  SVG/DXF/image import, native file format, GRBL streamer with simulator and optional serial
  transport, engine with 31 commands, undo/redo and panic guard, egui desktop app, CLI, `xtask`
  (fmt, clippy, tests, layers, clean-room, assets). 118 tests. Not yet run on real hardware.
- **2026-10-14 — M1.** Persistent GRBL `Link` (status polling, jog, home, unlock, set/go to origin,
  pause/resume/stop, console) with a shared `JobStream` protocol core; simulator learned `$J=`, `$H`,
  `G92`, G90/G91 and feed hold. Job origin modes (`job.set_origin`: absolute / user origin / current
  position, 9-point anchor) through the document, G-code placement, file format, CLI and UI. UI jog pad
  and anchor picker drawn in code (the default font has no arrow glyphs). 143 tests. Still not run on
  real hardware.
- **2026-10-14 — text.** New `text` crate: an original single-line stroke font (ASCII, upper and
  lower case, digits, punctuation) drawn from a glyph table with corner-cutting smoothing, plus
  layout (multi-line, alignment, letter/line gap). `ShapeKind::Text` keeps content and style next to
  the generated path; `text.create` / `text.edit` commands; Text tool and inspector editor in the UI.
  Clicking anywhere inside a text block selects it. Fixed a drag-select bug found while testing (a
  click that switched tools was re-handled as a selection click).
- **2026-10-14 — path operations.** New `pathops` crate on `i_overlay`: boolean union / difference /
  intersect / xor with even-odd holes, and offset (grow, shrink, round/miter/bevel joins; open paths
  become an outline). `path.boolean` and `path.offset` commands, Arrange menu and Selection-panel
  controls. Found and fixed, by using the app, that ellipses and circles were never closed shapes
  (no fill, no booleans, no inner-first ordering); regression tests added at the geometry, planner
  and engine layers.
- **2026-10-14 — kerf.** Per-layer kerf offset (`layer.set kerf_offset`): closed outlines are offset
  before cutting so parts grow and holes shrink by the beam compensation; fills and open paths are
  untouched. `pathops` moved down to layer 1 so the planner can use it.
- **2026-10-14 — images.** `image.adjust` (brightness, contrast, gamma, negative; non-destructive,
  applied before dithering and never to the white around a rotated picture), real thumbnails on the
  canvas through the shape transform, an Image section in the inspector. Found by streaming a
  120k-pixel picture to the simulator: an image flush against the bed edge made the job unrunnable
  because of the 2 mm lead-in, so absolute placements now shorten the lead-in to the room available.
- **2026-10-14 — never lose work.** Autosave to a per-user recovery file (`file.autosave`), a
  startup offer to recover it (`file.recover`: reopens as untitled and unsaved), prompts before New,
  Open and closing discard changes (a save that does not complete keeps the question open), and a
  title with an unsaved marker. Found by killing the app with `kill -9`: egui only repaints on
  input, so an idle window never autosaved; a repaint is now scheduled while there are unsaved
  changes (with a regression test).
- **2026-10-14 — join.** `pathops::join` chains open polylines whose ends meet (grid-hashed, closes
  loops, bounded) and `path.join` applies it per layer. DXF import joins loose segments by default
  (`join: false` to keep them), so outlines drawn as separate LINEs become closed, fillable shapes.
- **2026-10-14 — arrays.** `arrange.grid` and `arrange.circular` (pitch = size + gap; arc or full
  circle; bounded to 10,000 copies, one undo step) and a corner radius on rectangles
  (`shape.rectangle radius`, Rectangle tool option); Array section in the Selection panel.
- **2026-10-14 — tabs and leads.** Per-layer tabs (`tab_count`, `tab_width`: laser-off gaps spread
  evenly on outer outlines only, never on holes, declined when they do not fit) and lead-in/out
  (`lead_in`, `lead_out`: approach from the scrap side — outside a part, inside a hole — starting
  mid-edge so the lead never runs along the cut itself; shortened to stay on the bed for absolute
  placement). Found by test: a lead at a corner ran along the adjacent edge.
- **2026-10-14 — overrides and test grids.** `Shape::cut` (`CutOverride`: speed, power, passes)
  replaces the layer's settings for one shape; the planner groups shapes by override and plans each
  group with the effective settings. `shape.cut_override` and `generate.test_grid` (speed across,
  power down, one square per cell with its own settings, labels on the next layer; bounded) with a
  Material test grid section in the Laser panel and an Own cut settings section in the Selection
  panel. Process note: a gate failure in the previous commit slipped through a shell pipeline; the
  gate script now stops on failure.
- **2026-10-14 — groups and layers.** Shapes carry an optional group id: selecting a member selects
  the group, copies and array copies form groups of their own (never sharing the original's),
  `edit.group` / `edit.ungroup` (Ctrl+G, Ctrl+Shift+G, merging existing groups). Layers can be
  renamed, recoloured (#rrggbb) and reordered (`layer.move`; list order is cut order).
- **2026-10-14 — snapping and rulers.** Pure snap maths (`snap.rs`: axis snapping to edges, centres,
  bed and guides with grid fallback; translation snapping that locks the closest selection edge;
  Shift constraints for squares/circles and 15-degree lines; 1-2-5 ruler steps) wired into creation,
  resize, move and text placement, with alignment lines while dragging. Rulers on the canvas edges
  with a cursor marker, guides pulled out of the rulers (drag back to delete), View menu options.
  Egui 0.36 delivers modifier keys as events, which the test harness now emits.
- **2026-10-14 — node editing and pen tool.** `geom::nodes` is an editable node model (corner/smooth
  nodes with handles, shape-preserving insert, delete, line/curve segments, open/close) with
  commands `node.move`, `node.move_handle`, `node.insert`, `node.delete`, `node.set_smooth`,
  `node.set_segment`, `node.set_closed`, `path.create` and `edit.convert_to_path` (text to path).
  Nodes tool (drag nodes and handles, double-click a segment to insert, Delete) and Pen tool
  (click corners, drag out smooth nodes, click the first node to close, Enter/double-click/right
  click to finish, Escape to cancel), both snapping. 319 tests.
- **2026-10-14 — timeline preview.** `toolpath::Timeline` (per-operation start, per-pass time,
  per-layer totals, `at(t)` to find operation/pass/offset; `estimate` is now derived from it and
  carries the head position across operations). `job.estimate` also returns per-layer times. The
  preview has a bottom bar with Play/Pause, Show all, a scrubber, playback speed (auto = 30 s per
  job) and a per-layer time breakdown; done moves are drawn bright, the rest faintly, with a head
  marker. Tests: timeline maths, reveal/head logic, playback to the end.
- **2026-10-14 — recent files.** File > Open Recent (ten entries, most recent first, kept in
  `recent.json` in the app data folder through `Platform::recent_file`); any change of the
  document's path (open, save as) records it, picking one asks about unsaved changes first, and an
  unreadable entry is dropped from the list.
- **2026-10-14 — controller settings.** `device::settings` (parser for `$n=value` reports, validated
  `$n=v` writer, table of GRBL 1.1 settings with source URL); the link remembers what the controller
  reports (bounded) and offers `request_settings` / `write_setting` (refused while a job runs);
  the simulator answers `$$` and stores writes. The Laser panel has a Controller settings editor
  (flags as checkboxes, numbers with units) and warns when laser mode ($32) is off or $30 / $130-131
  differ from the document, with one-click fixes.
- **2026-10-14 — cut-order options.** `Document::optimize` (inner shapes first, reduce travel,
  remove duplicate lines; older files load with all three on) and `job.set_optimize`. Duplicate
  removal compares outlines after rounding to 0.1 µm, either direction and any start vertex, and
  applies to fills too (two identical shapes would otherwise cancel under even-odd). Laser panel
  gets a Cut order section.
- **2026-10-14 — image crop, sharpen and more dithers.** `image.crop` (fractions per side; the
  remaining picture stays where it was) and `image.sharpen` (3x3 unsharp mask) edit the stored
  pixels and are undoable; Atkinson, Jarvis, ordered (8x8 Bayer) and halftone (45 degree dot
  screen mapped through the spot function's quantiles so tone is preserved) join threshold,
  Floyd-Steinberg and grayscale. Image inspector gets a Crop and sharpen section.
- **2026-10-14 — G-code dialects.** `Machine::dialect` (GRBL, Marlin laser with `M3 I` inline
  power, Smoothieware with 0-1 `S` on each G1 and no spindle commands), chosen in the Device
  section and `machine.set`; written from the public firmware docs, flagged as untested on
  hardware; out-of-bed refusal applies to all. The serial link features stay GRBL-only.


- **2026-10-14 — presets.** `Document::presets` (named snapshots of a layer's cut settings, capped at
  200, saved with the project) and `preset.save` / `preset.apply` / `preset.delete`; applying keeps
  the layer's name, colour and output switch. Presets section in the layer panel.
- **2026-10-14 — head on the canvas.** With absolute start the connected machine's position is drawn
  on the bed (a ring and cross), and "Click the canvas to move the laser there" sends a bounded
  absolute `$J=G90 G21` jog (`Link::jog_to`); points off the bed are refused, Escape or
  disconnecting leaves the mode, and nothing is sent while a job runs.
- **2026-10-14 — precision layout.** Snapping now also locks onto path corners, segment ends and
  the midpoints of straight segments (capped at 200 per shape, 5000 overall; text excluded), and
  the Selection panel offers "Create by numbers" (x, y, width, height) for rectangles and ellipses
  on the active layer.
- **2026-10-14 — perforation.** Layers have `perf_cut` / `perf_gap` (mm; both > 0): outlines, open
  or closed, are cut as dashes with rapid moves over the gaps (laser off), replacing tabs when
  both are set. Lengths below 0.05 mm, non-finite values and absurd piece counts fall back to a
  solid line (or a plan error for bad settings).
- **2026-10-14 — MCP server.** New crate `laserfoundry-mcp` (layer 6; the UI layer moved to 7): a
  newline-delimited JSON-RPC 2.0 server implementing initialize, ping, tools/list and tools/call,
  where every engine command is a tool (`shape.rectangle` becomes `shape_rectangle`) plus a
  read-only `document_summary`. Engine failures are tool errors; messages over 64 MB are skipped
  without being buffered. `laserfoundry-cli mcp` serves it on stdin/stdout. Note for operators: the
  file.* tools read and write paths the agent names, so only attach it to agents you trust.
- **2026-10-14 — trace to vectors.** `toolpath::trace` follows the boundaries of dark pixels along
  pixel edges (outer loops clockwise, holes the other way, diagonal neighbours joined), then
  simplifies with Douglas-Peucker (bounded: 16 Mpixel, 4 M edges, 50 000 loops). `image.trace`
  applies the picture's tone adjustments, scales to the picture's size and position, and adds the
  outlines as a path (optionally replacing the picture). Image inspector gets a Trace section.
- **2026-10-14 — cut selected shapes only.** `Session::job_doc()` (the document, or just the
  selection) feeds generate, estimate, frame and the preview, so job origins anchor to the selection
  too; `job.set_scope` and a Laser-panel checkbox switch it (a session setting, not saved).
- **2026-10-14 — distribute and star.** `edit.distribute` (even gaps between three or more shapes
  along an axis, or a fixed gap from the first) with Arrange menu entries, and `shape.star`
  (3-100 points, inner and outer radius).
- **2026-10-14 — cross-hatch fill and a Star button.** `crosshatch` on a layer plans the fill a
  second time at right angles (older files load with it off); the Create by numbers section gets a
  Star button.
- **2026-10-14 — fuzz harnesses, and two bugs they found.** `crates/fuzzkit` has one entry point
  per untrusted-input path (SVG, DXF, project files, controller lines, G-code programs, MCP
  messages, text, commands with junk parameters), driven by deterministic mutation tests on stable
  and by `cargo-fuzz` targets in `/fuzz` (nightly; not run here). Found and fixed: the simulated
  controller panicked on a non-ASCII G-code word, and a program containing GRBL's real-time bytes
  (0x18 soft reset, `?`, `!`, `~`) would have been streamed as is; the streamer and the console
  now refuse such lines, comments included, before anything is sent.
- **2026-10-14 — auto-reconnect and run-from-line.** A lost serial connection is retried with
  doubling waits (up to 8 tries, 30 s apart at most); Disconnect cancels it and nothing is ever
  resumed by itself. `gcode::resume_from` rebuilds position, feed, power, spindle mode and air from
  the skipped lines and never repeats `G92`; `job.resume` and a "Run from line" control in the
  G-code section start a job part-way through, with the beam off for the approach move.
- **2026-10-14 — Z axis and rotary.** Layers have `z_offset` and `z_per_pass`: the program moves Z
  (`G0 Z`, beam off) before each pass, brings it back to 0 for layers that do not use Z, and a
  resumed job restores it. `Machine::rotary` (reference and object diameter) scales Y by
  reference/object, refuses unusable diameters, and the Device section can set the bed height to
  one turn of the object. Galvo needs controller-specific protocols and stays out of scope.
- **2026-10-14 — outline fonts.** `text::layout_outline` draws filled glyph outlines (holes
  included) from a TrueType/OpenType file the person supplies, via `skrifa`; sizes are set by cap
  height, advances come from the font (no kerning or shaping). `text.create` / `text.edit` take
  `font` (a path; "" returns to the stroke font); the laid-out shapes are stored, so projects open
  without the font. No font is in the repository: tests use a font built in code
  (`text::testfont`, behind the `testfont` feature).
- **2026-10-14 — print-and-cut alignment.** `geom::register::two_point` finds the move and turn
  (and optionally size) that carries two marks onto where they really are, sharing the error
  between them and refusing coincident marks and absurd scales; `align.two_point` applies it to
  every shape or a chosen few in one undo step. The Laser panel's "Align to marks" section sets
  the design marks (by clicking, or the centres of two selected shapes) and the real ones (by
  clicking on a photo of the bed, or From laser using the head position) and draws them on the
  canvas. Live camera capture is OS- and hardware-specific and is not built.
- **2026-10-14 — browser build.** `apps/laserfoundry-web` compiles the engine and the egui
  interface to WebAssembly (`apps/laserfoundry-web/build.sh`, then serve the folder and open
  `index.html`); checked in headless Chromium: it draws, creates shapes and handles drags. It has
  no file dialogs or machine link yet (`Platform::machine_available` is false there, since the
  link needs threads). Clocks use `web-time`, which is `std::time` on desktop and works in the
  browser.
- **2026-10-14 — browser files.** Hosts without file paths now work through `Platform`
  (`browser_files`, `request_upload` / `take_upload`, `download`); the engine's `file.open` and
  `file.import_image` accept `data_base64` and `file.project_data` returns the project as data
  (shared `doc::base64`). The web build opens and imports through the browser's file chooser and
  saves projects and G-code as downloads; verified in headless Chromium (upload of an SVG, download
  of a project).
- **2026-10-14 — browser machine link (Web Serial).** `Link::connect_inline` runs the same worker
  on the caller's thread, driven by `Link::tick` once per frame (30 ms while connected); the
  transport only has to be non-blocking. `Platform` gained `inline_link`, `serial_async`,
  `request_serial` and `take_serial` (the person picks the device in the browser's prompt). The web
  build's `WebSerial` transport wraps the Web Serial streams (queued ordered writes, a reader task,
  EOF on close); the simulator also works there (`set_non_blocking`). Verified in headless Chromium
  against a scripted fake device (connect, status polling, G-code generated and streamed to the
  end). Real hardware through a browser is untested, like the desktop link.
- **2026-10-14 — overscan and power ramp.** Layers have `overscan` (laser-off run-up and run-out
  beyond fill lines, kept inside the bed with absolute placement; also overrides the machine's
  setting for pictures) and `ramp_length` (outline power rises from the layer's minimum to full
  over that distance in half-millimetre steps, so the pierce point does not scorch).
- **2026-10-14 — autosave generations.** The autosave keeps the three newest copies (`recovery.ext`,
  `recovery.1.ext`, `recovery.2.ext`), every write is read back to confirm it parses (the status bar
  says so when it does not), startup offers the newest copy that still reads back, and saving or
  discarding removes them all.
- **2026-10-14 — crop handles and resample.** With "Drag the edges on the canvas" on, the selected
  picture's own edges (so rotated and flipped pictures work) can be dragged; the removed bands are
  shaded and the percentages feed the Crop button. `image.resample` changes the pixel count (area
  average when shrinking, bilinear when enlarging; 0.05x to 4x, 100 Mpixel cap) without changing the
  size in mm.
- **2026-10-14 — starter presets.** `preset.add_starters` adds six original, generic first guesses
  (light mark, engrave, deep engrave, photo, thin cut, multi-pass cut), named for the operation
  rather than a material because results depend on the laser and sheet; the UI says to check them
  with a test grid.
- **2026-10-14 — text shaping and text on a path.** Outline text is now shaped with `harfrust`
  (kerning, ligatures, contextual forms; checked with a legacy `kern` table in the in-code test
  font). `text.on_path` bends a text shape along another shape's longest outline (letters keep
  their size and stand on the line, long edges are cut into 0.5 mm steps so they follow curves,
  closed paths wrap), producing plain geometry; Arrange > Text on Path applies it to a selected text
  and shape. New `geom::along` measures polylines by arc length.
- **2026-10-14 — cut order by group and start point.** `Optimize::by_group` finishes each group of
  shapes (Ctrl+G groups) before starting the next, within each layer; `Optimize::start` sets where
  ordering begins instead of the bed's origin. Both are in the Cut order section and
  `job.set_optimize`.
- **2026-10-14 — MCP resources and prompts.** The server now answers `resources/list` and
  `resources/read` (document summary, the G-code the job would run now, the command reference) and
  `prompts/list` / `prompts/get` (design from a description, prepare for cutting, explain the job).
- **2026-10-14 — shared preset library.** `preset.export` / `preset.import` (validated, names
  already present kept) and a per-user `presets.json` (via `Platform::preset_library_file`): the
  Presets section can save the project's presets to the library and add the library's to a project.
- **2026-10-14 — smooth tracing.** `image.trace` takes `smooth`: `geom::nodes::smooth_polyline`
  turns bends gentler than 60 degrees into smooth nodes (handles a third of the way to each
  neighbour) and keeps sharper ones as corners, so traced discs and lettering come out as editable
  curves with the node tool.
- **2026-10-14 — more DXF.** ELLIPSE (full and partial, any rotation), SPLINE (de Boor with
  optional weights, falling back to fit points or the control polygon when knots are unusable, work
  capped), and INSERT of BLOCK definitions (translate, rotate, non-uniform scale, base point; nested
  four deep; a shared entity budget stops self-inserting blocks).
- **2026-10-14 — SVG pictures and text.** Embedded PNG/JPEG/GIF/WebP pictures in an SVG become
  picture shapes at the right place and size (capped at 64 pictures and 64 MB; an undecodable one is
  counted, not fatal). Text is converted to outlines by `usvg` using the machine's installed fonts,
  which are only loaded when the file contains text (and not in the browser).
- **2026-10-14 — acceleration-aware times.** `Machine::acceleration` (mm/s², from `$120`/`$121`
  with a one-click "use controller's") times each move as a trapezoid starting and ending at rest,
  so short segments cost realistically; used by `job.estimate` and the preview timeline.
- **2026-10-14 — preview colours.** View > Preview colours: by layer (as before), by power, or by
  speed (blue to red; slow is hot), scaled to the ranges present in the job.
- **2026-10-14 — project templates.** File > New from Template lists `.laserfoundry` files in the
  per-user templates folder (`Platform::templates_dir`), opens one as a new untitled unsaved project
  (`file.open` with `template: true`), and saves the current project there under a sanitised name
  without touching its own saved state.
- **2026-10-14 — themes.** View > Theme: follow the system, light or dark. The canvas has matching
  light and dark token sets, and layer colours are lightened for the dark canvas or darkened (hue
  kept) for the light one so white and pale layers stay visible.

- **2026-10-14 — review fixes (device, gcode).** Stop while finishing now soft-resets; an alarm during
  finishing is reported; the serial reader caps its line buffer (4 KiB) and resyncs after overflow;
  a stream that aborts soft-resets the controller; stray `ok`s are ignored; errors carry source line
  numbers; jogs use G53 and a work-position flag picks `WPos` versus `MPos`; run-from-line restores
  `G92` and the spindle line; G-code refuses bad feeds, NaN power and out-of-extent relative moves.
- **2026-10-14 — overlapping edges.** With "remove duplicates" on, straight edges lying on top of an
  earlier edge (shared sides of touching shapes, a line drawn twice) are cut once; the trimmed shape
  becomes open pieces. Shapes that overlap nothing are untouched; huge or non-finite jobs are skipped.
- **2026-10-14 — keyboard shortcut editor.** Edit > Keyboard shortcuts… rebinds undo, redo, select
  all, duplicate, group and ungroup (click, press the new keys, Esc cancels). Chords need Ctrl or Alt
  (or an F-key); clashes are refused with the name of the other action; bindings persist in
  `shortcuts.json` (`Platform::shortcuts_file`) and menus show the current keys. Arrows, Delete,
  Enter and Escape stay fixed.
- **2026-10-14 — screen-reader support.** The desktop app now builds eframe with AccessKit, and the
  controls drawn in code (canvas, layer chips, jog arrows, job-origin cells) carry names, roles and
  selected state so assistive technology can read and operate them; a headless test reads the
  accessibility tree. Not yet tried with a real screen reader.
- **2026-10-14 — overscan from speed.** Machine > Overscan "from speed" (`machine.set`
  `overscan_by_speed`) sizes the run-up and run-out of fills and pictures as v²/2a from the layer's
  speed and the machine acceleration, wherever a layer has no overscan of its own; capped at 50 mm.
- **2026-10-14 — vertical text.** Text objects can be stacked top to bottom (`vertical` on
  `text.create`/`text.edit`, "Vertical" in the text inspector): one column per line, first line
  rightmost, each letter centred in its column; works with the stroke font and outline fonts.
- **2026-10-14 — font fallback.** An outline text object can name a second font file
  (`fallback_font` on `text.create`/`text.edit`, "Fallback font" in the inspector); characters the
  first font lacks are drawn from it, scaled to the same cap height. Only characters in neither font
  are reported as unsupported.
- **2026-10-14 — resume after a power cut.** While a job runs the desktop app writes its position
  (`job-position.json` in the data folder) every 5 s, backed up 20 lines because the controller
  acknowledges lines when buffered, not when run. A clean finish removes it; otherwise the next start
  shows "A job was interrupted… restart from line N" in the Laser panel, and "Use this line" fills
  Run from line once the same design's G-code is generated (matched by a fingerprint).
- **2026-10-14 — per-shape cut priority.** `shape.set_order` (Laser menu; "Cut order" in the shape
  inspector) gives shapes a priority within their layer: lower numbers are finished first, ties keep
  the drawing order; saved in the project (older files read as 0).
- **2026-10-14 — tonal-band tracing.** `image.trace` takes `levels` (1–8): that many outlines, each
  around the pixels darker than a share of the threshold, lightest first, so one picture can be
  engraved or cut in nested tones (put the shapes on different layers to give each its own power).
- **2026-10-14 — pictures keep their colour.** Imported coloured pictures now store RGB beside the
  grey (older files and grey pictures are unchanged; crop, resample and sharpen keep both in step;
  the project file validates the sizes). `image.trace` `colors` (2–8) groups the colours (k-means,
  paper-white left out) and traces each onto the layer whose colour is nearest. Pictures imported
  before this change must be imported again to trace by colour.
- **2026-10-14 — curve fitting for traces.** `image.trace` `fit` (0.05–50 px) fits cubic Béziers by
  least squares (Schneider's method) through each traced outline, splitting where the fit is worst and
  keeping sharp bends as corners: a traced disc becomes four curves instead of dozens of nodes.
  Use `tolerance` of 1 or more so single-pixel steps are not read as corners. Gap 9 is closed.
- **2026-10-14 — translations.** All fixed interface text (about 275 call sites: menus, panels,
  tooltips, headings, combo boxes, the status bar) goes through `i18n::tr`, which looks the English
  text up in a JSON catalog the person supplies (View > Language: load, save a template of every text
  shown so far, back to English), optionally with a font for scripts egui's does not cover. Nothing
  ships by the assets rule. See `docs/translations.md` for the format, limits and what is not yet covered.
- **2026-10-14 — translations, interpolated texts.** `trf!("Job finished: {lines} lines", lines = n)`
  translates a template with named places and fills in the values (single pass, values are never
  expanded again); about 30 status and label texts that were built with `format!` use it, and the
  static status messages are translated where they are written.
- **2026-10-14 — translated engine errors.** Messages from the engine are matched by template
  (`"shape {id} does not exist"`), their values picked out and put where the translation says, nested
  messages translated too (`i18n::tr_message`). `cargo xtask messages` scans the sources for
  `#[error]` texts and `EngineError` constructors into `crates/i18n/engine-messages.txt` (about
  135 templates), `cargo xtask ci` fails if it is stale, and "Save translation template" includes them.
- **2026-10-14 — message list verified.** A test runs every command with hostile parameters and checks
  each error against the shipped template list (all covered; serde's own parameter messages are
  covered by the wrapper entry `bad parameters: {0}`). The catalog is now per thread (the interface
  has one), so tests that load different catalogs cannot disturb each other.
- **2026-10-14 — mixed-direction text.** Outline text now follows the Unicode bidirectional algorithm
  (UAX #9, via the `unicode-bidi` crate): each line is split into left-to-right and right-to-left runs
  that are put in visual order and shaped in their own direction, with the fallback-font split applied
  inside each run (reversed within right-to-left runs). The stroke font has no right-to-left letters, so
  it is unchanged. Gap 4 is closed; gap 10 (ramp along fills and images) was dropped as unnecessary
  given overscan, speed-based overscan and `M4` dynamic power.
- **2026-10-14 — SVG export.** `file.export_svg` (File > Save SVG…; `include: true` returns the text, which
  is what the browser build downloads) writes the drawing in millimetres with one group per layer
  coloured like the layer: line layers as hairline strokes, fill layers as even-odd fills, text as
  outlines, pictures embedded as PNG with their tone adjustments. `selection_only` and
  `include_hidden` narrow or widen it. Written so work designed here can go into software that takes SVG
  but not G-code (such as a cloud-driven laser's own app); reading it back gives the same size.
- **2026-10-14 — live MCP link.** `laserfoundry --agent-link` (or `LASERFOUNDRY_AGENT_LINK=1`; off by
  default) opens a loopback-only port and writes `agent-link.json` (port + random token, owner-only
  permissions) to the per-user data folder; `laserfoundry-cli mcp --connect` then bridges an agent's
  stdio to it, so the agent edits the drawing in the open window and the person watches it change.
  Decisions: loopback TCP with a token (portable, no new dependencies); opt-in per launch with a
  warning logged; commands run on the app's thread once per frame (no shared locking), at most 16
  per frame and 4 agents at once; only engine commands cross the link, so a machine job cannot be
  started by an agent, and `file.new`/`file.open`/`file.recover` are refused while there are unsaved
  changes. Hostile-handshake, wrong-token, file-permission, stale-file and end-to-end tests are in
  `crates/mcp/src/link.rs`.
- **2026-10-14 — MCP resource subscriptions.** The server advertises `resources.subscribe` and
  answers `resources/subscribe` / `resources/unsubscribe`, then sends `notifications/resources/updated`
  (after the reply that caused it) when a watched resource's content may have changed: the document
  summary on any edit or selection change, the G-code on edits, the command reference never. On the
  live link the check runs every frame, so the person's own edits in the window are announced too.
  Unknown URIs are errors and a connection may watch at most 16. Writes to each agent now go through
  one writer thread per connection, so answers and notifications keep their order.
- **2026-10-15 — CAD-style alignment cues.** Snap targets now know what they are (bed edge, bed centre,
  guide, shape edge, shape centre, path endpoint, midpoint) and how far their owner extends. The canvas
  draws a line only from the moving thing to what it locked onto (guides and the bed's own lines
  run the length of the view), with an X on edges and corners, a + at centres, a square at endpoints
  and a triangle at midpoints, plus a small label naming the target. Where several targets are
  equally near, the more specific wins (endpoint over midpoint over centre over edge). With a
  drawing tool chosen and no button down, the same cues preview what a click would lock onto.
  Pure geometry in `snap::alignment`, tested for every mark and both axes; frame tests check a
  moving shape reports an endpoint and the bed centre. Not yet: equal-spacing hints between three
  shapes, and CAD-style tracking (hover a point to acquire it, then align to it).
- **2026-10-15 — equal-spacing hints.** While a selection moves, if nothing lined up on an axis, it
  snaps to an equal gap with the neighbours in its row (those overlapping it on the other axis):
  centred between the two nearest, or repeating the gap of the two nearest on either side. The
  gaps are drawn as measured lines with end ticks and a size tag in mm (new `Tokens::spacing`
  colour). Pure maths in `spacing.rs` (both axes, row filtering, touching gaps ignored, bad
  numbers and 20 000-shape inputs survived) with a frame test; the alignment label now sits below
  the cursor so the two do not collide. Still not done: CAD-style tracking (hover a point to
  acquire it, then align to it).
- **2026-10-15 — object tracking.** With a drawing tool, resting the cursor half a second on a vertex,
  midpoint or shape centre (a ring shows it arriving) acquires it: a + marks it, up to six at once.
  From acquired points the cursor is pulled onto the dashed row and column through them (drawn across
  the whole view) and onto 45-degree lines in both directions, including the exact crossing of a
  diagonal with a locked column or row (an object line, guide or another tracking line). Objects on
  an axis keep priority; right on the acquired point no angle is offered. Escape, leaving the drawing
  tools, or finishing a shape forgets them (`Snapped::diagonal`, `snap_point_tracked`,
  `acquirable`; frame tests for acquiring, passing over, diagonal start and forgetting). Note:
  every vertex's row and column already attract the cursor without acquiring, so the row/column
  part mostly adds the full-length dashed line; the diagonals are the new capability. Tracking
  applies to creating, resizing and pen/click points, not to moving shapes.
- **2026-10-15 — tracking: more angles, and while moving.** Tracking lines now run at 30, 45 and 60
  degrees and their mirror images (`snap::TRACK_ANGLES`; `Diagonal` carries its angle). Moving a
  selection uses acquired points too: whichever of its box's corners, edge middles or centre needs
  the smallest nudge is pulled onto a row, column or angled line (`snap::track_translation`);
  objects keep priority, and angles only apply when neither axis is locked. In the Select tool,
  where resting the cursor is how things are picked, pressing T over a vertex, midpoint or centre
  acquires it and pressing T again removes it (any tool). Points persist across Select and the
  drawing tools; Escape, finishing a shape or move, or the Nodes tool forgets them.
- **2026-10-15 — tracking angle choice; spacing while creating and resizing.** View > "Tracking lines"
  picks which angled lines tracking offers: row and column only, 45° diagonals, 30/45/60° (default),
  or every 15° (`snap::TrackAngles`, part of `SnapSettings`). Sizing a new shape or resizing the
  selection now also snaps the dragged edge to repeat a gap already in the row (between two shapes,
  or between the fixed edge and the shape behind it), with the gaps drawn as when moving
  (`spacing::edge_spacing`, both axes and both directions, hostile-input tests, plus a frame test).
  Checked by the headless frame tests and by eye under Xvfb (gap tags while sizing, the 45° line, the menu).
- **2026-10-15 — CLI translations.** The catalog code moved out of `ui-egui` into a new layer-0 crate,
  `laserfoundry-i18n` (the UI re-exports it, so nothing there changed), so the command line can use it.
  `laserfoundry-cli` now translates its usage text, `info` labels, option errors and engine errors with
  `--translation FILE`, `$LASERFOUNDRY_TRANSLATION` or the desktop app's catalog, and
  `translation-template` lists every text it shows. Process-level tests cover each way of choosing a
  catalog, the template, and broken or missing catalogs. Also: a Windows build workflow
  (`.github/workflows/desktop.yml`, now a Windows/Linux/macOS matrix with serial ports, attached to `v*` releases).
- **2026-10-15 — recent projects in the browser.** A browser has no file paths, so "Open Recent" keeps the
  projects' contents instead: up to 8, at most 1 MB (base64) each and 3 MB together, in the page's local
  storage under `laserfoundry.recent` (`browser_recent.rs`; the oldest entries go first when the budget is
  exceeded, a project too large to keep is simply not remembered). A project is remembered when it is
  opened from an upload or saved (downloaded); choosing an entry opens it, and a damaged one is dropped from
  the list. The host provides two new `Platform` hooks, `storage_get` / `storage_set` (the web build wraps
  `localStorage` and tolerates it being blocked or full). Unit tests cover ordering, budget, round trip and
  hostile stored text; a frame test covers save, restart, reopen and a damaged entry. The wasm build compiles; not
  yet tried in a real browser.
- **2026-10-15 — browser favicon.** `apps/laserfoundry-web/index.html` carries an inline SVG icon (a beam
  hitting a cut line on a dark tile), drawn here, so there is no asset file and no ATTRIBUTION row.
- **2026-10-15 — font library download (desktop and CLI only).** New layer-3 crate `laserfoundry-fontlib`
  fetches `Stitchwright/stitchfoundry-fonts` into `LaserFoundry/fonts` in the data folder: the file list
  from GitHub's tree API, each file from its raw address, previews skipped (the repository is ~290 MB,
  almost all of it the Ink/Stitch collection), files kept when already present at the listed size, paths
  validated, counts/sizes capped, writes atomic, cancellable. `laserfoundry-cli fonts download [--all]
  [--to DIR]` and `fonts path`; the desktop app's View > Font library offers the Foundry fonts (~1 MB) or
  everything. Checked against the real raw host (15 files, identical to the repository) and against a
  local server in tests. **The hosted browser build never downloads:** `Platform::font_library_available`
  defaults to false (only the desktop host turns it on), the menu is absent there, and `cargo xtask layers`
  now fails if anything but `laserfoundry` and `laserfoundry-cli` depends on `fontlib` or `ureq`. Caveat
  at that point: the library holds embroidery fonts, not TTF/OTF (see the next entry).
- **2026-10-15 — library fonts usable for text.** The Foundry fonts' `glyphs.stitchfoundry` turn out to hold
  ready-drawn vector glyphs (anchors with absolute in/out Bézier handles, y down, baseline y = 0) and
  `font.json` the advances and leading. `text::GlyphFont` reads them (every size, count, depth and number
  bounded; hostile-input tests), works out the cap height from H/E/I/T, winds counters against their outer
  shapes so holes survive either fill rule, and `layout_glyph_font` lays text out left to right (no shaping,
  bidi or kerning: these are Latin lettering fonts) in the same coordinates as outline text. `text.create` /
  `text.edit` take a font folder (or its `font.json`/`glyphs.stitchfoundry`) as `font`, anywhere a TTF path
  was accepted; a library folder cannot be a fallback font. The text inspector has a **Library fonts** drop
  down listing the downloaded folders by name. Looked at with the real fonts (all five, including holes in
  A/B/O and Script's swashes) and in the running app. Not covered: the 131 Ink/Stitch fonts, which are SVG
  glyph fonts (`ltr.svg`, up to 14 MB each) in a different format; the web build has no font folder.
- **2026-10-15 — font library in the hosted browser build.** The site serves the Foundry fonts itself: the
  Pages workflow checks out `stitchfoundry-fonts` (sparse, `fonts/` only) and `cargo xtask site-fonts` copies
  each complete folder's `font.json`, `glyphs.stitchfoundry` and licence into `site/fonts/` with an
  `index.json`; nothing is committed here and nothing is fetched from a third party at run time. View >
  Font library > "Download the Foundry fonts" fetches them from the site into the engine's new in-memory
  font registry (`engine::fontlib`, fonts named `library:<folder>`, accepted wherever a font path was), the
  browser remembers which are installed (`laserfoundry.fonts` in local storage) and re-fetches them on later
  visits (the HTTP cache makes that cheap), and the text inspector's Library fonts list shows them.
  The index and the remembered list are parsed with bounds (`fontindex.rs`, native tests); the registry is
  capped. The large Ink/Stitch set is not offered on the web. Tried in headless Chromium against a local copy
  of the site: download, list, switch font, reload and restore. Note: a project that uses a library font
  keeps its drawn letters, but editing the text needs the font loaded again.
- **2026-10-15 — tablet mode.** View > Tablet mode (Automatic by default: on at the first touch; or On/Off).
  When on: the interface is drawn at 1.3x with roomier controls and a wider tool column; grab reach for
  shapes and nodes grows from 6 to 16 points (corner handles keep a smaller reach and never swallow the
  middle of a small shape); two fingers pan and zoom the canvas together and drop any half-made one-finger
  edit; a **Pan** tool drags the view with one finger; an action bar carries Undo, Redo, Delete,
  Duplicate, Fit, Pan, a sticky **Shift** (holds the modifier for axis-locked moves, square shapes and
  add-to-selection) and a **Panel** toggle that folds the inspector away (it starts folded below 1100
  points of width). The page (`index.html`) now stops the browser taking touches (no page scroll, pinch
  zoom, text selection or long-press menu). Code in `tablet.rs`; frame tests cover auto-detection and the
  override, two-finger pan/zoom, dropping a drag when a second finger lands, the Pan tool, the sticky
  Shift, fingertip reach and the action bar. Tried in headless Chromium at 1024x768 with real touch
  events (CDP): first touch switches the mode on, finger-drawn rectangle, two-finger pan and spread zoom,
  Shift and Undo. Not done: long-press as right-click, hover-based features (object tracking) have no
  touch equivalent yet, the on-screen keyboard for text editing is the browser's, and nothing has been
  tried on a physical iPad or Android tablet.
- **2026-10-15 — fuzzing the font code.** Three new fuzz entry points in `fuzzkit` (`glyph_font`: parse, lay
  out in four styles, register with the engine and use through `text.create`; `font_index`: the hosted
  site's index and the remembered list; `font_listing`: the repository tree listing and path checks), each
  with stable mutation tests (run by `cargo xtask ci`) and a cargo-fuzz target. Coverage-guided runs of 90 s
  each (nightly): 1.2 M, 2.3 M and 2.1 M executions, nothing found. `fuzzkit` may depend on the downloader
  (the layering rule that keeps the network out of the browser build exempts it, as it ships nowhere).
- **2026-10-16 — canvas context menu; tracking without hover.** Right-click, or a long press on a touch
  screen, opens a context menu on the canvas (first selecting the shape under the pointer): Undo, Redo, Select
  All, Duplicate, Delete, **Track this point / Stop tracking this point**, vertical and horizontal guides at the
  pressed point, and the Arrange commands. "Track this point" is the hover-free way to acquire a tracking
  point on a touch screen. Fixes found on the way: acquirable points are now looked up among the targets
  themselves (`snap::acquirable_near`) instead of read back from the snapped lines, which failed whenever two
  vertices shared a row or column (so hovering on a rectangle's corner never worked, on desktop either); the
  corners, side middles and centre of every shape's box are acquirable, not only drawn-path vertices; and
  acquired points are drawn with no pointer or hover (they vanished on touch screens). Frame tests for the
  menu (selection, Duplicate, Delete, tracking on a path vertex and a plain rectangle's corner, guides) and
  unit tests for `acquirable_near` (shared rows, guides, hostile numbers); tried with a real long press in
  headless Chromium.
- **2026-10-16 — Ink/Stitch fonts for text.** `GlyphFont::parse_inkstitch` reads the collection's folders
  (`font.json` + `ltr.svg`, one `GlyphLayer-<char>` layer per glyph; `text/src/inkstitch.rs`, using
  `roxmltree` and kurbo's SVG path parser): transforms are composed through groups, a satin column
  (`inkstitch:satin_column`, first two subpaths = rails) becomes the outline between its rails, filled areas
  stay closed shapes and running-stitch lines stay open lines; the baseline and capital height come from a
  flat-bottomed capital rather than from guide conventions, advances from `font.json`. All bounded (24 MB
  drawing, 3 M XML nodes, nesting, paths per glyph, every number); `font.json` may be 16 MB (kerning tables).
  `text.create`/`text.edit` accept such a folder like a Foundry folder, parsed fonts are cached by folder
  (size and time of the two files), and the Library fonts list covers `fonts/` and `inkstitch/` (scrolling).
  Tried with the real collection: 121 of 131 fonts lay out text in at most 0.27 s (the other 10 are
  right-to-left-only or split into one file per letter and get a clear message); looked at 8 of them (the
  plain satin and script fonts read well, ornament fonts draw their ornaments, `bluenesia_satin`, built of
  many short columns, is fragmentary). Satin outlines overlap where columns cross (right for Fill, doubled
  lines for Line). Fuzzed: stable mutation test plus 5.2 M coverage-guided executions, nothing found.
  Not done: right-to-left fonts, the one-file-per-letter fonts, kerning, and the web build (Ink/Stitch is
  hundreds of MB, so it is desktop only).
- **2026-10-16 — reconciled with VectorCraft and CADCraft.** Read the latest `storytold/vectorcraft` and
  `storytold/cadcraft` (both public; `craftrules` is still not reachable) and took over their standards, not
  their code (their rule: the crafting apps share standards, never code). New gates in `cargo xtask ci`:
  **wasm** ("never break wasm": every crate of the browser build is checked for `wasm32-unknown-unknown`; new
  crates must be classified in `xtask/src/wasm.rs`) and **brands** (no other vendor's names in user-visible
  text: Rust string literals outside tests, crate descriptions, the hosted page); plus `cargo xtask stats`
  (tests and lines per crate) and a GitHub **CI workflow** that runs the whole gate on every push and pull
  request (we had only build and deploy workflows). `CLAUDE.md`/`AGENTS.md` gained: share standards never
  code, bound recursion and nesting, a regression test per crash fix, a fuzz entry point per new parser, the
  progress-docs update rules, and the shell and parallel-agent notes. Not taken over (they need more than a
  rule): the `--control` JSON-lines channel with `ui.screenshot` (our agent link is MCP), the `cli perf`
  budgets, shared test-corpus repos, and the About-window contributor credits.
- **2026-10-16 — first CI run.** The new GitHub CI workflow failed on its first run: CI's stable Rust (1.99)
  has a clippy lint (`chunks_exact_to_as_chunks`) that the older local toolchain (1.97) lacked, flagging three
  `chunks_exact` calls with a constant size (`doc`, `toolpath`). Fixed with `as_chunks` (stable since 1.88,
  below our minimum 1.95); the full gate now passes locally on 1.99.1. Lesson: run the gate on current
  stable before relying on a green local result.
- **2026-10-16 — browser translations.** The browser build's View > Language menu now works: *Load translation…*
  picks a JSON catalog through the upload path and keeps its text in local storage (`laserfoundry.translation`),
  so it is used again at the next visit; *Use English* clears it; *Save translation template…* downloads the
  template. A catalog's `font` is ignored in the browser (it names a file the page cannot read). Checked by a
  headless frame test (load, restart, damaged catalog, English, template download); not yet clicked through in a
  real browser. Not done: a font for non-Latin scripts in the browser, and shipping any translation.
- **2026-10-16 — translations, composed labels.** Texts that were assembled with `format!` now go through the
  catalog with named places: the canvas preview error and tracking-angle label, the layer button names and
  tooltips (including "output off"), the job-origin names, the two-mark alignment result, the unset mark, GRBL
  setting names and their "not documented" hint, and the shortcut editor's refusals. Checked by a test that a
  translation reaches the shortcut conflict message, and the full gate. Not done: successful command results
  in the status bar, and any shipped catalog.
- **2026-10-16 — translations, catalog errors.** The errors the catalog loader reports (not JSON, no `strings`,
  too many or too long strings, unreadable or oversize files) now go through the active catalog too. A review of
  the status bar found no successful command result still shown in English: what remains are the font-library
  download summaries and failures, which the hosts build. Checked by a test that a loader error is translated,
  and the full gate. Not done: those download messages, a non-Latin font in the browser, any shipped catalog.
- **2026-10-17 — security audit.** Dependency advisories (`cargo audit`: clean), pattern scans, and two independent
  code reviews of the file/command/agent-link surface and of G-code, the machine link, networking, the browser
  build and the workflows; findings were verified before acting. Fixed (each with a regression test where code
  could test it): uncapped reads of device paths (`/dev/zero`), the link token's weak fallback, a handshake that
  could be held open for ever, array copies multiplying into millions, no soft-reset attempt after a failed write
  mid-job, an unbounded browser serial buffer, a symlink-followed temp file when saving; plus a CSP for the
  browser page, read-only build tokens, an audit workflow and Dependabot. Open items for a decision are in
  `docs/security-audit.md` (the main one: the bug-fix workflow runs `cargo test` while holding an API key).
  Checked by the full gate. Not done: hardware, third-party review, SHA-pinned actions.
- **2026-10-17 — bug-fix workflow without builds.** `claude-fix.yml` no longer installs a toolchain or lets Claude
  run `cargo`: the job holds the API key and reads untrusted issue text, so running repository code there could
  leak the key. The pull request's own CI is the gate. Checked: workflow parses; not yet run on a real issue.
- **2026-10-17 — security audit, the rest.** Every action pinned to a commit hash; agents in the open window may
  only write beside the open project or in the temporary folder (never onto a link); undo history also bounded by
  memory (about 256 MB estimated); bounded request/reply queues on the agent link with a write timeout; the data
  folder owner-only; a confirmation before starting a job placed from the head's position or a user origin (the
  bed cannot be checked); the browser asks before the page closes while a job runs or work is unsaved; one app
  instance no longer removes another's link file. Tests for each except the reply-queue limit. Open: refuse or
  confirm a start when the controller is not in laser mode (`$32=0`); see `docs/security-audit.md`.
- **2026-10-17 — laser mode check.** The app now asks the controller for its settings as soon as it answers, and
  refuses to start (or frame, or resume) a job while `$32=0`: without laser mode GRBL does not turn the beam off
  for travel moves. Controllers that report no `$32` (other dialects) are not blocked. Checked by a frame test
  against the simulator (refused with laser mode off, accepted when turned on). That closes the audit's last
  code item; `docs/security-audit.md` lists what is left (a reply-queue test, hardware, outside review).
- **2026-10-17 — browser checks in real Chromium.** A fresh browser build, driven with Playwright (software
  WebGL, real mouse events): the page loads under its content security policy with no violations; View >
  Language > Load translation reads a catalog through the file chooser and the interface switches (Auswahl,
  Rechteck, Bereit); a reload brings it back from local storage; Save translation template downloads
  `translation-template.json`; and closing the tab after drawing a shape raises the browser's "leave site?"
  prompt. That closes the "not yet clicked through in a real browser" caveat on the browser translations and
  confirms the audit's CSP and tab-close warning. Not checked: Web Serial (needs a device), Firefox/Safari
  (no Web Serial there; the rest is untested).
- **2026-10-17 — reply-queue test.** The agent link's limit on queued replies now has a test: an agent that
  asks for the large command reference over and over without reading any answer is hung up on once the socket
  buffers and the (test-sized) queue fill, while another agent keeps working; it fails if the drop is removed.
  Closes the audit's last test gap. Left in `docs/security-audit.md`: very large hostile documents are slow, and
  hardware, outside review and a licence review were out of scope.
- **2026-10-18 — AI and accounts, the core (`crates/ai`).** A new layer-0 crate, written for laser work and
  checked for the browser build (no network code): requests and reply parsers for Design from Prompt and Redraw
  Image (millimetre SVG whose colours say cut, score or engrave), Ask About Lasering (document as labelled
  context, bounded conversation), Check My Cut (photos in; setting suggestions read from one JSON object and kept
  only when in range) and the account service (sign up/in/out, account, checkout, requests paid in credits, with
  a key never sent to the service and a session token never to Anthropic). Same service as StitchFoundry's, so
  one account and balance; the code is new, not copied. 19 tests plus a fuzz entry (`ai_reply`, stable
  mutation test and a `fuzz/` target). Not done: the account window and dialogs, the hosts' network clients,
  token storage. See `docs/ai.md`.
- **2026-10-18 — AI and accounts, the interface and the desktop client.** An AI menu (Design from Prompt, Redraw
  Image, Ask About Lasering, Check My Cut, Account) shown where the host can send requests (the desktop app).
  Engine commands: `ai.context` (the machine and layers in use, in words), `ai.place_art` (the model's SVG onto
  the layers its colours name, sized, centred, grouped, one undo step; an untouched layer of filled art becomes a
  fill layer), `ai.prepare_picture` (shrunk to 1568 px, transparency onto white, JPEG). Check My Cut's
  suggestions are applied one at a time as `layer.set`. The desktop host sends with `ureq` (no redirects) and
  keeps the session token and any API key in an owner-only file. Checked by 7 interface tests against a scripted
  host, 5 engine tests (placing, sizing, hostile artwork, context, pictures), 3 host tests (a local server,
  redirects, the private file) and the full gate. Not done: any run against a real service or key, the browser
  build, a privacy/consent screen. See `docs/ai.md`.
- **2026-10-18 — AI first-use consent.** The first AI request (not signing in) now waits behind a question naming
  where it goes and what it carries, with Send, Send and do not ask again (kept with the account settings) and
  Cancel; cancelling sends nothing and remembers nothing. Found by its own test: confirming sent the request
  back through the same question; fixed by handing it straight to the host. Checked by a frame test and the full
  gate.
- **2026-10-18 — AI in the browser.** The hosted page can now use the service it was built for: a `fetch` client
  (no redirects, time limit, answers capped), the content security policy gets the service's origin when the
  Pages build has the `LF_CLOUD_URL` variable (nothing otherwise, and then the AI menu is hidden), and requests
  to any other address are refused with a reason. Pictures need a file chooser the browser does not have yet, so
  there the menu offers Design from Prompt, Ask About Lasering and the account. Checked end to end in Chromium
  against a local fake service with CORS: sign in, credits read, Design from Prompt asks first, sends, and the
  artwork lands sized at 100 mm with the black layer made a fill layer; 2 native tests for the origin rules and
  the full gate. Not done: Redraw Image and Check My Cut in the browser; a run against the real service.
- **2026-10-18 — AI pictures in the browser.** Redraw Image and Check My Cut now work in the hosted page: a
  picture comes through the browser's file chooser (the existing upload path, a new kind), is prepared by
  `ai.prepare_picture` and kept for the window that asked. Checked in Chromium against the local fake service
  (a PNG chosen, sent as a JPEG with the session token, artwork placed) and by a frame test (chooser request,
  arrival, junk refused). A stale note from one AI window no longer shows in the next. Not done: a run against
  the real service.
- **2026-10-18 — docs in step.** README and the parity table now describe the AI features and accounts (a new
  "AI & accounts" row, self-assessed at 35%, estimated: nothing has run against the real service).
- **2026-10-18 — AI: the Assistant principles.** After reading StitchHub's draft terms (which exclude the Foundry
  apps), the app adopts the principles of their Assistant section without claiming the text: a switch per feature
  in the Account window (kept with the settings; a switched-off feature's menu item is disabled), a local log of
  every request and answer (`ai-log.jsonl`, owner-only, rotated at 5 MB, pictures replaced by their size, no
  headers so no tokens or keys), notes that answers can be wrong, and a lawful-use reminder with Anthropic's
  usage policy in the consent screen. Checked by 4 tests (log contents and secrets, rotation, switches kept)
  and the full gate. Not done: terms for the Foundry apps; the browser keeps no log (no file to put it in).
- **2026-10-19 — a `/debug` skill.** Added `.claude/skills/debug/SKILL.md` and a Debugging section in
  `CLAUDE.md`/`AGENTS.md` (now identical again; `AGENTS.md` had lagged on commit authorship): a Reproduce, Probe,
  Examine, Fix workflow adapted to this project (command scripts and frame tests, the simulator, per-crate test
  loops, stale `target/` and browser `pkg`, no panics or `println!`, regression tests that fail without the fix,
  laser-safety bugs first, saying what could not be checked). It condenses working notes on Kuhlmann's *The Book
  of Debugging* (No Starch Press) kept in the `autolayout` repository, in new words; the notes themselves stay
  there.
- **2026-10-19 — ready for the first real laser.** Added **Laser → Save machine report…** (the program's version
  and system, the connection, what the document assumes, the controller's `$$` settings and the console, as
  plain text with nothing private) and `docs/hardware-bringup.md`: safety, reading and fixing the settings the
  program checks, a laser-off motion test, first marks on scrap, and exactly what to bring back for a fix.
  Checked by a frame test against the simulator. Not done: any of it on real hardware (that is the next step).
- **2026-10-19 — the `laserfoundry.app` domain, prepared.** `docs/domain.md`: registering through Cloudflare, the
  DNS records for GitHub Pages (DNS only), verifying the domain with GitHub, setting it in the repository, and
  what to change in the repository afterwards. The program needs no change (the page uses relative paths). Not
  done: the registration and the DNS (the owner's), and the links in the README and docs, which wait for the
  domain to work.
- **2026-10-19 — real-GRBL readiness, round 2.** Reading the machine link against the public GRBL 1.1 interface
  notes for what a simulator cannot show: GRBL's `error:`/`ALARM:` numbers are now explained in words in the
  console and in job errors (from the public GRBL documentation; the number is always kept); a job will not
  start while the controller reports an alarm, hold, open door, homing, movement, check mode or sleep (it
  names which); a serial port that will not open says what usually fixes it (Linux `dialout` group, another
  program holding it, a loose cable). Checked by unit tests (code table, messages, the console line against the
  simulator, the port hints) and a frame test for the idle check; the first-run checklist is updated. Not done:
  any of it on real hardware.
- **2026-10-19 — Renamed LightFoundry → LaserFoundry.** The name sits too close to another vendor's product, so
  every crate, app folder, binary, `LASERFOUNDRY_*` environment variable, the `.laserfoundry` project extension,
  the data folder and all docs now say LaserFoundry; the domain `laserfoundry.app` is bought (setup steps in
  `docs/domain.md`). Checked with `cargo xtask ci` (all checks passed). Not done: the GitHub repository is still
  `Stitchwright/lightfoundry` (renaming it is a settings change; URLs in `Cargo.toml`, the README and
  `help.rs` keep the old name until then, and GitHub redirects the old one); files saved by an earlier build with
  the `.lightfoundry` extension are not recognised by name.
- **2026-10-19 — Repository renamed to `Stitchwright/laserfoundry`.** The repository URLs in `Cargo.toml`, the README,
  the Help menu and `docs/domain.md` now use the new name. Checked with `cargo xtask ci`. Not done: the custom
  domain (DNS, GitHub verification) is still the owner's to set up.
- **2026-10-19 — Shared Foundry account service by default.** The desktop and browser apps now use
  `https://cloud.stitchfoundry.app` (the service's `/v1/health` answered ok, ai, payments; its CORS answer allows any
  origin and our request fields are all on its pass-through list) unless `LF_CLOUD_URL` names another; the Pages
  build falls back to it for the content security policy, so the browser AI menu is no longer hidden. The window
  is now "Foundry Account" and the credits "Foundry credits". Checked with `cargo xtask ci` and a test of the
  default address; the service's health and CORS preflight were read with curl. Not done: a signed-in request
  against the real service (that creates an account and spends credits, so it is the owner's to try), and prompt
  tuning from a real run.
- **2026-10-19 — Landing-page copy.** `docs/marketing.md`: names and links, taglines, short and long
  descriptions, feature blocks, audience, an honest-status paragraph, FAQ and keywords, for the StitchFoundry team
  to build the LaserFoundry page from. Every claim was checked against the README and the parity assessment. Not
  done: screenshots (to be captured from the real app) and the page itself.
- **2026-10-19 — A first screenshot for the landing page.** `docs/screenshots/canvas.png`, taken from the desktop
  app under Xvfb with an original demo design; `docs/marketing.md` says what else the page needs. Not done: the
  preview, dithering and AI-window screenshots.
- **2026-10-19 — Dependabot's ten updates, applied together.** Rust: `i_overlay` 9.1, `getrandom` 0.4 (live-link
  tokens), `skrifa` 0.48 and `harfrust` 0.14 (its shaping API was redone: `crates/text/src/outline.rs` now builds a
  `harfrust::Font` and `ShaperFont` and calls `shape`, and a font it cannot shape is a `BadFont` error, not a panic
  or a silent skip). Workflows (all still pinned by commit): `checkout` 7.0.1, `upload-artifact` 7.0.2,
  `download-artifact` 8.0.2, `upload-pages-artifact` 5.0.0, `deploy-pages` 5.0.1, `action-gh-release` 3.0.3. Checked with
  `cargo xtask ci` (the text and font tests cover the new shaping path). Not checked before the push: the
  Pages, Desktop and release workflows only run on pushes and tags, so the action bumps are proven by those runs.
- **2026-10-19 — Windows builds are signed with Azure Trusted Signing.** `desktop.yml` has a `sign` job in the
  `signing` environment, run only for published releases and manual runs: it unpacks the Windows zip, signs both `.exe` files, fails
  unless `Get-AuthenticodeSignature` says `Valid`, and replaces the artifact, which the release job then attaches to the published release. Needs the
  secrets `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET` and the variables `AZURE_SIGNING_ENDPOINT`,
  `AZURE_SIGNING_ACCOUNT`, `AZURE_SIGNING_PROFILE`. Not checked yet: the first manual run (the names and the action's inputs
  are unproven until it signs), and nobody has launched a signed `.exe` on a Windows machine.
  (Superseded on 2026-10-20: the environment is now `release` and all Azure values are secrets; see the entry below.)
- **2026-10-20 — Signing moved to the `release` environment; macOS signing and notarization added.** `desktop.yml` now has
  `sign-windows` and `sign-macos`, both in `environment: release` (the `release` branch and `v*` tags only), triggered by a
  published release or a manual run from one of those refs; a manual run from `main` is refused by the environment. Windows reads
  all six Azure values from secrets (`AZURE_SIGNING_ENDPOINT`, `AZURE_SIGNING_ACCOUNT`, `AZURE_CERT_PROFILE` are no longer
  variables). macOS imports the Developer ID certificate into a temporary keychain (`packaging/macos/import-cert.sh`), builds
  `LaserFoundry.app` (`packaging/macos/package.sh`, new: until now the macOS build was a bare binary in a tarball), signs with the
  hardened runtime and a timestamp, notarizes, staples and uploads `laserfoundry-macos-arm64.zip`. Missing secrets give an unsigned
  build and a warning, never a failed run. See `docs/releasing.md`. Both jobs were then run on a real pre-release; see the entry below.
- **2026-10-20 — Release signing tested on the pre-release `v0.0.1-rc.1`.** Run 39412511322 (event `release`, commit `cb2903d`)
  went green. Windows: Azure Trusted Signing signed `laserfoundry.exe` and `laserfoundry-cli.exe` with no warnings or errors, and
  `Get-AuthenticodeSignature` reported both `Valid` with the expected signer. macOS: `LaserFoundry.app` and `laserfoundry-cli` passed
  `codesign --verify`, `notarytool` returned `Accepted`, stapling and `stapler validate` worked and `spctl` assessed the app as
  `accepted` (source `Notarized Developer ID`). The `release` job attached the signed files. The signing jobs ran in the `release`
  environment from a `v*` tag, as it allows. The Windows signing was from the first attempt and the macOS signing from a re-run of
  that run (the Apple secrets were presumably added in between; not confirmed). The test release and tag were deleted afterwards.
  Not checked: how SmartScreen treats the signed `.exe` on a real Windows machine, a launch of the notarized app on a real Mac,
  the missing-secrets path (unsigned build plus warning) and a manual run from the `release` branch.
- **2026-10-20 — release notes for v0.1.0.** `docs/releases/v0.1.0.md`, written from this log, the README and `docs/gaps.md`:
  downloads and which builds are signed, what the app does, and the known limits, with the point that nothing machine-side has
  run on real hardware at the top. `docs/releasing.md` links it. Checked by reading every claim against the ROADMAP; not
  checked: the notes on a real release page (the release is not published yet).
- **2026-10-20 — font-download messages are translatable.** The result of the font library download reached the interface as a
  finished English sentence, built on the download's worker thread, where the person's translation is not active, so it was never
  translated. The host now sends figures and a reason (`FontDownload`, `FontDownloadFailure` in `crates/ui-egui/src/platform.rs`) and
  the interface words them with `tr`/`trf!`, in the desktop and browser hosts alike. The English wording is unchanged. Checked with a
  test of every English sentence, a test that loads a catalog and sees the status line translated (it fails if `tr` is removed), and
  `cargo xtask ci` (the browser host is compiled for wasm). Not done: the system's or network's own error text inside a failure
  stays as reported, and the browser build's fetch errors stay English; no translation ships.
- **2026-10-20 — corner-aware time estimates.** Estimates and the preview timeline used to time every move from rest to rest, which
  overstates anything cut as many short segments (a curve, text, a traced picture): 100 one-millimetre pieces of a straight line
  at 50 mm/s and 500 mm/s² cost about 8.9 s where the same line as one move costs 2.1 s. `laserfoundry_toolpath::move_times` now runs a look-ahead planner over each
  operation: the speed through a corner follows the junction-deviation model from GRBL's public cornering notes (`v = sqrt(a·R)`,
  `R = δ·sin(θ/2)/(1−sin(θ/2))`, never above either move's own speed; URLs in the code), then a backward and a forward pass keep every
  entry and exit speed reachable. New machine setting `junction_deviation` (`machine.set`, 0 to 10 mm, default 0.01 as GRBL's `$11`;
  older files get the default; 0 gives the old rest-to-rest estimate), a "Corner deviation" field next to Acceleration, and `job.estimate`
  and the preview both use it. Checked by tests: the formula (a right angle at 500 mm/s² and 0.01 mm gives 3.47 mm/s), a straight line in
  pieces equals one move, a reversal still stops, bad values fall back to the old numbers, and over 600 random segments at three speeds
  the planned time stays between the speed limit and the old estimate; the preview's last segment ends when the timeline says (it fails
  if they drift apart). Not done: the estimate assumes unlimited look-ahead where a real controller sees a limited number of moves,
  nothing here was compared with a real machine's clock, and the clean-room rule means GRBL's source was not read (public documents only).
- **2026-10-20 — old name tidied.** `docs/domain.md` no longer says the repository rename is still to come (it names the old
  `lightfoundry` address only to say not to use it), and the local git remote now points at `Stitchwright/laserfoundry`, so pushes
  no longer go through GitHub's redirect. Checked with `cargo xtask ci`. Not changed: the old name in earlier ROADMAP entries,
  which is history.
- **2026-10-21 — nesting.** `arrange.nest` packs the selection into the bed, or a region (`x`, `y`, `width`, `height`), without overlaps:
  a gap between parts (default 2 mm), a margin, and evenly spaced turns tried for each part (default 4, 1 keeps angles); a group is one
  part and keeps its shape; parts that do not fit stay where they were, are returned in `unplaced` and become the selection. New crate
  `laserfoundry-nest` (layer 1, in the wasm build), designed from the public pixel-based nesting approach and nothing else: the region is a
  grid, each part a conservative mask of the cells it touches (outline cells plus inside cells), thickened by the gap, placed largest
  first at the first free spot scanning from the top-left, the rotation whose bottom edge is highest winning. Work, sizes, parts (500),
  points and rotations are capped. Interface: a Nest section in the selection panel, **Arrange > Nest in the Bed**, and a status line
  saying how many were placed and how many did not fit. Checked with 21 crate tests, six engine tests (one undo restores every position,
  groups, leftovers, a region, refused input) and a panel test; the key test applies each placement to the real outlines and checks with
  independent segment-distance and point-in-polygon geometry that nothing overlaps, every gap holds and nothing leaves the region, over
  random shapes, gaps, margins and rotations. It fails when the gap thickening or the inside fill is removed (both tried). 100 parts nest
  into a 300 x 200 region in a fraction of a second. Not done (gap 23): holes in parts are not used, shapes already on the bed are ignored, no
  multiple sheets, and packing is only as fine as the cell grid. No fuzz target (it parses nothing); the engine's hostile-parameter sweep
  covers the command. Behaviour comes from the public literature on raster nesting; nothing was taken from any other program.
- **2026-10-21 — undo history window.** Every undo step now carries the label of the command that made it. `edit.history` returns the kept
  steps (oldest first) and how many are applied; `edit.history_goto {position}` undoes or redoes to that point (0 = before the oldest kept
  step, out-of-range refused and nothing changes). **Edit > Undo History…** opens a window listing "Start" and each step, the current
  point highlighted and steps that Redo would bring back shown faint; clicking a row jumps there. A new action after jumping back drops the
  later steps, as with ordinary undo. Checked with four engine tests (labels, jumping to the start, the middle and the newest step, bad
  positions, redo dropped by a new action) and a panel test that renders the window and jumps. Not done: the panel test calls the command
  rather than clicking a row (the row positions are not exposed to the frame tests), and nothing was looked at on screen.
- **2026-10-21 — select by size.** `edit.select_by_size {min_width?, max_width?, min_height?, max_height?, in_selection?}` selects the shapes
  whose bounds fit every limit given (mm, inclusive); a group is judged by the bounds of all its members and selected whole; `in_selection`
  narrows the current selection instead of searching the document. At least one limit is required; negative, non-finite or huge values are
  refused and the selection is untouched. It is not an undo step (selection only). Grouping uses a map and the selection a set, so large
  documents stay linear. **Edit > Select by Size…** opens a window with a tick box and a value per limit, an "only within the current
  selection" option and a Select button, and the status line says how many were selected. Checked with five engine tests (each limit and
  combinations, inclusive edges, narrowing, groups, refused input) and a panel test of the parameters and the selection; the group test fails
  when grouping is removed (tried). Not done: no limit on area or layer, and the window was not looked at on screen or clicked in a frame test.
- **2026-10-21 — font manager (browse and preview).** `text.preview {font?, text?, height?}` lays a sample out (default "Aa Bb Cc 123", at most
  100 characters, 1 to 100 mm) in any font text accepts and returns its outlines in mm from the top-left corner with the size and any characters
  the font lacks; it changes nothing and is not an undo step. **Edit > Font Manager…** (and a **Font manager…** button in the text
  inspector) opens a window listing the built-in stroke font, the downloaded font library and the .ttf/.otf files beside a file you pick
  ("Fonts from a folder…", desktop only; links are not followed, 400 files at most), each drawn as a sample, with an editable sample text, the
  selected text's font marked "In use", and **Use** applying a font to the selected text (`text.edit`). Rows are laid out only when visible,
  two per frame, so a large font does not stall the window; a font that fails to load says why in its row. Not done by choice: no adding or
  removing fonts, no Ink/Stitch SVG fonts beyond what text already reads. Checked with two engine tests (sizes, the corner origin, closed
  outlines from an outline font, nothing changed or undoable, bad text/height/font refused) and four panel tests (reading a result and junk,
  scaling, the folder scan, the window drawing the built-in sample and knowing the selected text's font). Looked at in the running app (headless):
  the menu entry and the window with the built-in font's sample; the library and folder lists, **Use** and the inspector button were not
  looked at or clicked (no fonts or network here).
- **2026-10-21 — nesting keeps clear of the shapes that stay.** `arrange.nest` no longer ignores the rest of the bed: every shape that is not
  being nested (a group counts once) is an obstacle and the nested parts keep the gap from it, as from each other. Shapes on a layer whose
  output is off (a backdrop photo, say) are not obstacles. `avoid_others` (default true) turns this off; the result says how many shapes were
  kept clear of (`avoided`). The nest crate gained `nest_around(parts, obstacles, region, options)` (`nest` calls it with none): each obstacle
  is drawn on the grid before any part is placed, segments are clipped to the region first so a shape far outside (or a billion mm long) costs
  nothing, obstacles with a non-finite coordinate are ignored, and at most 5,000 obstacles are taken (more is an error that names the limit;
  `avoid_others: false` is the way round it); their points count towards the point limit. The Nest section has a "Keep clear of other shapes"
  box. **Behaviour change:** a nest of part of the drawing used to land on top of the other shapes; it now goes round them. Checked with nine
  crate tests (the gap held against random obstacles, partly outside and open lines, using independent distance code; a part goes only into the
  slot that is big enough; a blocked region reports every part; obstacles outside or with bad numbers change nothing; 2,000 obstacles quickly;
  limits; the clipping), two engine tests and a panel test. The obstacle tests fail when obstacles are not drawn on the grid and hang when
  segments are not clipped (both tried). Not done (gap 23): holes in parts, several sheets, finer than the cell grid; the new box was not looked
  at on screen.
- **2026-10-21 — nesting can use the holes in parts.** `arrange.nest` takes `use_holes` (default false). When set, a closed outline that lies
  wholly inside another closed outline of the same part (a shape, or a group, which nests as one part) is taken to be a hole and its inside is left
  free, so smaller parts can be placed in it, keeping the gap from the hole's wall; an island inside a hole is solid again, and so on inwards.
  An outline counts as enclosed only when every point of it is inside the other and no edge touches an edge of the other, so overlapping,
  crossing, touching or doubled outlines are never taken for holes; a part with more than 64 closed outlines or 4,000 points is treated as solid.
  Every outline's own cells are marked last, so a rim thinner than a cell is never lost. It is off by default because a nested outline that is
  engraved solid (a Fill layer) is not a hole; shapes that stay on the bed (obstacles) are always solid. The Nest section has a "Use holes in parts"
  box. Checked with eight crate tests (a part goes in the hole only when asked, and the gap to the wall holds; an island is not room; overlapping,
  crossing, touching and doubled outlines are not holes; depths; a 0.4 mm rim; random plates and parts turned 0 to 8 ways, checked with independent
  even-odd and edge-distance code; holes make room solid parts do not), one engine test and one panel test. They fail when the final wall pass is
  removed, when the edge-touch test is removed and when islands are not filled again (all three tried; the island case needed its own test, as the
  first one did not catch it). Not done (gap 23): several sheets, packing finer than the cell grid; the new box was not looked at on screen.
- **2026-10-21 — nesting onto several sheets.** `arrange.nest` takes `sheets` (1 to 20, default 1) and `sheet_gap` (mm, default 20). Parts that do not
  fit the region go on sheet 2, a copy of the region laid out to its right `sheet_gap` away, and what that cannot take goes on sheet 3, and so on;
  it stops when nothing is left or a sheet takes nothing (so a part too big for any sheet is left where it was, selected). Each sheet keeps clear
  of the same obstacles and uses the same gap, margin, turns and holes setting; one undo puts every sheet's parts back. The result gains
  `per_sheet` (how many went on each sheet) and the status line says "Placed N on M sheets". The Nest section has a "sheets" box. The sheets are
  laid out beside the bed, not on it: moving a sheet onto the bed is up to the person. Done in the command, not the nest crate (each sheet is
  one ordinary nesting). Checked with three engine tests (six squares on one, two and three sheets, every shape inside its sheet and the
  spacing; the spacing asked for and a part too big for any sheet; bad counts and spacings refused with nothing moved) and a panel test of the
  status lines; the sheet tests fail when the offset is removed (tried). Not done (gap 23): packing finer than the cell grid; the new box was
  not looked at on screen.
- **2026-10-21 — nesting retries on a finer grid.** When no `resolution` is given and parts are left over, `nest_around` tries again with the cells
  halved, up to twice (cells 2x then 4x finer), and keeps the result that places the most parts (the coarser one on a tie). A try is made only
  while it is expected to stay inside the work limit: halving the cell costs about sixteen times the work, so it is skipped when the previous
  try already used more than a sixteenth of the limit or stopped early, and it stops when the cell cannot get smaller. A resolution given by hand
  is used as it is. `Nesting` gains `work` (grid cells inspected). Example: sixteen 24 mm squares with 1 mm gaps just fit a 100 mm square (99 mm
  across); the automatic 0.42 mm cell placed 9, the retry places all 16. Checked with five crate tests (that case, with the placed squares
  verified on the real outlines; a hand-set resolution left alone; random sets never place fewer than the first try; 300 parts that cannot all fit
  finish quickly and a part bigger than the region keeps the first cell size; the work reported and bounded); the first fails with refinement
  turned off (tried). Not done (gap 23, reworded): a big job with a very tight fit can still leave a part out, and there is no exact-geometry
  compaction (sliding parts together after placing).
- **2026-10-21 — Marlin controller link (simulator-only).** `Link::connect_with(transport, Protocol)` (and `connect_inline_with`) pick the protocol;
  `Protocol::Marlin` is new, `Protocol::Grbl` is what `connect` has always meant. The app chooses it from the machine's G-code setting when it
  connects (Marlin gets the Marlin link and, for the in-app simulator, a `SimulatedMarlin`; Smoothieware keeps the GRBL link, which it documents
  support for), and refuses to run a job if the setting was changed after connecting. What the Marlin link does differently: one line in flight
  at a time (Marlin tells the host nothing about free buffer space), `ok` or `ok ...` as the acknowledgement, `M114` for the position (only
  when idle, one request outstanding, marked as work coordinates), `M400` after the last line and the job is finished when it is answered,
  Pause stops sending (moves already queued still run: documented), Stop sends `M410` then `M5` (never a reset byte or any GRBL real-time
  byte), a job ends at the first `Error:` or `echo:Unknown command` with the same stop, jogs/home/unlock/zero are single commands per line
  (Marlin runs only the first command on a line) with `M5` first, `$$` settings are refused. Answers owed for lines sent before a job starts
  are used up first so they are not credited to the job. `parse_status` also reads the older comma form Smoothieware uses
  (`<Idle,MPos:..,WPos:..>`), and `parse_marlin_position` reads `M114`. Checked with 17 link tests on a scripted wire and on `SimulatedMarlin`
  (flow control, pause, stop bytes, errors, completion, a flood of junk, no GRBL byte ever sent, the whole thing end to end with the laser state),
  the parsers, two panel tests and the fuzz entry `controller_line`, which now also runs both protocols against arbitrary replies. Mutations
  of the stop sequence, the owed answers, pause, the one-line window and error handling each fail tests (tried). **Not checked against any real
  controller:** the simulator is written from the same public docs, so it can share my misreadings; `docs/hardware-bringup.md` has a Marlin
  section listing what to try first, with Stop and the laser state first. Not done: Marlin `M114` during a job, `ok` accounting if a firmware
  drops an acknowledgement, Smoothieware-specific settings.
- **2026-10-24 — End-to-end regression suite, and a crash it found.** `apps/laserfoundry-cli/tests/regression.rs` (24 tests) drives the
  real `laserfoundry-cli` binary on inputs written inside the test: byte-exact golden G-code for SVG, DXF and raster jobs and for scripts
  covering shapes, booleans, arrays, fill, passes and kerf, text, test grids, nesting, node edit, offset and resume-from-line
  (`tests/golden/`, re-recorded with `LF_BLESS=1`), invariants checked on every program (laser off, finite numbers, inside the bed), save and
  reopen, undo/redo, rollback, out-of-bed refusal, selection-only, SVG export re-import, the command catalogue, and hostile input through the
  binary (broken files and scripts, absurd parameters, every command with empty parameters). **Bug found and fixed:** a DXF circle of radius
  1e300 killed the process (a 7.5 GB allocation): kurbo's ellipse code squares the radii, which overflows, and then asks for an unbounded
  number of segments. `ellipse_path` now returns no outline past `MAX_ELLIPSE_RADIUS` (1e12 mm), and `ellipse_path` and `flatten` keep their
  tolerance above a fraction of the path's size; regression test in `laserfoundry-geom` (it aborted before the fix). Checked: a changed golden
  fails and names the first differing line (tried). The goldens record today's output, so they catch change, not correctness; none of it has
  been compared with a real machine. Noticed, not fixed: `info` can print `cut -0 mm` for an empty drawing. See `docs/testing.md`.
- **2026-10-24 — Hostile-input sweep, three more fixes.** Probed ~300 cases under a 3 GB memory limit and a 30 s timeout: every command with
  extreme values (absurd and at the edge of what is accepted) and importer files with extreme magnitudes (SVG, DXF, PNG headers that claim
  huge sizes, project files). The commands all validated cleanly; the importers did not. **Fixed, each with a regression test that fails
  without it:** (1) DXF `ARC` with a huge radius and `ELLIPSE` with a huge axis exhausted memory like the circle did, because kurbo sizes
  arcs from `radius / tolerance`; the DXF arc, ellipse and bulge code now uses `geom::curve_tolerance` and skips radii past
  `MAX_ELLIPSE_RADIUS`; (2) an SVG with a huge amount of text ran for minutes (layout time grows faster than the length: 0.6 s at 20,000
  characters, 7 s at 80,000), so `import_svg` now refuses more than `MAX_TEXT_BYTES` (40,000) of text with a clear message; (3) `info` printed
  `cut -0 mm` for an empty drawing. The new importer cases are also in the CLI regression suite. **Noticed, not fixed:** `path.boolean` on a
  40 x 40 array of 100-point stars works for 22 s before refusing with "too many points", so a refusal that comes sooner would be kinder; and
  usvg's text layout is third-party, so the 40,000-byte limit is a guard, not a fix of its speed. Not checked: the same sweep on the browser
  build, the UI, or with files from real-world sources.
- **2026-10-24 — Application icon.** New crate `laserfoundry-icon` (layer 0, native-only for the wasm check) draws the mark in code: the
  favicon's beam, spark and cut line on a dark rounded tile, with edge smoothing, at any size, and writes it as PNG, a Windows `.ico` (7 sizes,
  16 to 256, PNG-compressed) and a macOS `.icns` (16 to 1024, with the platform's margin). No image file is committed (nothing to list in
  `ATTRIBUTION.md`); a small binary, `laserfoundry-icon png|ico|icns <out>`, writes the files. Used in three places: the window icon
  (`ViewportBuilder::with_icon`), the Windows executable (`apps/laserfoundry/build.rs` with the `winresource` crate; a failure warns and keeps
  the default icon), and `LaserFoundry.app` (`package.sh` writes `AppIcon.icns`; `Info.plist` names it). Checked: 8 crate tests (the parts sit
  where the favicon has them, smoothed edges, margin, every size, hostile sizes, PNG/ICO/ICNS structure and that each embedded image decodes),
  I looked at the result at 256, 48 and 16 pixels, and a real window under Xvfb carries the 256-pixel icon with the right colours in
  `_NET_WM_ICON`. **Windows build checked in CI:** on the Desktop workflow's Windows runner (the push run and a manual run on 765172d) the
  new step found the 256-pixel icon image inside `laserfoundry.exe`, so the build script embeds it. The manual run's signing jobs ended after one
  second, as the `release` environment refuses runs from `main` (documented). **Not checked:** how the icon looks in File Explorer, the task bar
  or the Start menu (no one has looked on a Windows desktop yet), the
  macOS bundle (`package.sh` ran only as far as `ditto`, which exists only on macOS), and the Linux install, which has no desktop entry or
  icon file yet. The design is a straight copy of the favicon; a designer's version would be better.
- **2026-10-25 — Features and screenshots page.** The hosted page at the site root is the app itself (a full-screen canvas), so the page lives
  beside it at `/features/` (source `apps/laserfoundry-web/site/features/`; the Pages workflow copies `apps/laserfoundry-web/site/` into the site)
  and the app's address does not move, so existing links to it keep working. Static HTML and CSS, no scripts, its own content security policy,
  dark and light by the visitor's setting, skip link, alt text on every image. Three screenshots taken from the running desktop app under Xvfb
  on demo designs made for them (design with a selection, toolpath preview with per-layer times, a 12-part nest), a feature list that sticks to
  what is in the app now, a machine-status table that says GRBL has been run only against the simulator, and links to releases, source, the
  bring-up guide and the gap list. Checked: rendered in Chromium at 1280 and 390 pixels wide, dark and light, no horizontal overflow, no
  broken images, no console or policy errors; `cargo xtask brands` passes (it scans this page). **Not checked:** the live page after deploy,
  screen-reader behaviour beyond the structure, and other browsers. The og:image address uses the github.io site and needs changing when
  laserfoundry.app serves it. The screenshots show the desktop app on Linux; Windows and macOS windows will look a little different.
- **2026-10-26 — Weekly audit failed; lockfile updated.** The scheduled `Audit` run found two vulnerabilities and a yanked crate in `Cargo.lock`
  (no code of ours changed): `rustls-webpki` 0.103.15 (RUSTSEC-2026-0382, certificate verification may accept distrusted intermediates) and
  `webpki-roots` 1.0.9 (RUSTSEC-2026-0381, bundled roots allow distrusted emSign intermediates), both reached through the HTTPS client the
  desktop app uses for the optional AI service and font downloads, and `clipboard-win` 5.4.1 (yanked). Updated to 0.103.16, 1.0.11 and 5.4.3
  with `cargo update -p`; only the lockfile changed. Checked: `cargo audit --deny unsound --deny yanked` failed the same way here before and
  passes after (the three allowed "unmaintained" warnings remain), and `cargo xtask ci` passes. **Not checked:** the AI and font downloads against
  the live services with the new certificate libraries. Still open: the Dependabot PRs for egui and eframe 0.37 fail CI (they are API changes, not
  yet looked at).
- **2026-10-26 — egui and eframe 0.37, ureq 3.5.0 (open Dependabot PRs 14, 15 and 16).** The two UI crates have to move together, so one change
  does both. Code changes: `egui::WidgetType` is now `egui::Role` (the design canvas is announced as a canvas instead of "other"), and
  `Id::new` is `Id::unique` (same hashing, so the dialogs keep their ids). **A regression the tests did not catch:** the larger default button
  padding made "Rectangle" wrap to "Rectangl" / "e" in the tool column, seen only by taking a screenshot. The column now sizes itself from the
  real labels and fonts (`panels::column_width`), which also covers longer translated names; a frame test checks that a long name widens it.
  Checked: `cargo xtask ci` and `cargo audit` pass, the app was run under Xvfb with the same design as before (toolpath preview and per-layer
  times identical: 2 m 59 s, 1 m 38 s, 25 s), the window icon is unchanged, and the website screenshots were retaken. **Not checked:** other
  windows and dialogs under 0.37 beyond the main window and the preview, Windows and macOS rendering, and ureq 3.5.0 against the live services
  (this sandbox's network proxy re-signs HTTPS, so the font download cannot be tried here). Pull requests 12 (rust-cache) and 13
  (trusted-signing action) are workflow changes, left for you to review; 14, 15 and 16 are done by this change.
- **2026-10-30 — Correction: v0.1.0 was already released.** `v0.1.0` was published on 2026-10-20 (tag on a28e7f0, a full release, not a
  pre-release) and the Desktop workflow's release run succeeded: `sign-windows` including its "Verify the signatures" step, and `sign-macos`
  including notarization and stapling, with the three downloads attached (checked on GitHub today; the signatures themselves, SmartScreen and
  Gatekeeper on a fresh machine are still unchecked). Entries above that say the release was waiting for someone to publish it are out of date;
  I had not looked at the releases list and told the owner to publish a release that already existed. A release-notes edit I pushed in between
  (1c9b7c2) described features the published binaries do not have and was reverted. `main` is 24 commits ahead of the tag: nesting, the undo
  history window, select by size, the font manager, the Marlin link, corner-aware time estimates, the application icon, the regression suite, the
  importer fixes, egui 0.37 and the certificate-library updates. Those need a new tag (v0.1.1 or v0.2.0) with its own notes
  (`docs/releases/<tag>.md`); nothing is drafted yet.
- **2026-10-31 — The hosted web app was blank in browsers since the rename; fixed.** The page's content security policy lets its one inline
  start-up script run by hash. The 2026-10-19 rename changed the script's text (`lightfoundry_web.js` became `laserfoundry_web.js`) but not
  the hash, so Chromium refused to run it and the app never started at https://stitchwright.github.io/laserfoundry/ (12 days). It was found by
  loading the real page in Chromium for the first time; no earlier test or check loaded it. The hash is corrected, and a native test
  (`apps/laserfoundry-web/src/page.rs`) now computes the script's SHA-256 and fails when it is not in the policy; with the old hash put back it
  fails with the message that names the right one. The test checks the script and the policy, not that the app then runs: that was checked
  once by hand with the built bundle in Chromium (the app started and opened a file). **Not done:** a browser test in CI, which would catch
  other ways the page can fail to start.
- **2026-10-31 — MakerFoundry: publish a design, and open one from a link.** MakerFoundry's side was already built (its `docs/api.md`, MF-7 and
  MF-18) and waiting. `laserfoundry_ai::maker` (new, no network code) builds the `POST /api/v1/designs` request and reads its answer, and checks a
  `?open=` address strictly. The UI has **File › Publish to MakerFoundry…** (a window using the Foundry Account that the AI features use; the draft's
  page opens in the browser, where the title, licence and price are set) and the browser version opens a design from `?open=`: fetched with a
  plain GET, no cookies, no redirects, 32 MB at most, named from `Content-Disposition`, imported as a dropped SVG or DXF. New platform hooks
  (`startup_link`, `start_fetch`, `take_fetch`) keep the hosts thin; the web page's policy and the host's allow-list name MakerFoundry. **Checked:**
  33 `ai` tests (13 new) and 13 UI tests with every protection removed in turn (4 of 4 and 5 of 5 caught after adding one missing test); a fuzz
  entry that asserts accepted addresses are MakerFoundry's; the real release bundle in Chromium with MakerFoundry mocked (opens, address cleaned,
  404 sentence, foreign and look-alike hosts make no request); and the live services through the app's own builders: Foundry sign-up and
  account, an Ask question (2 credits, balance matched), error mapping, sign-out, and a **publish answered 201** by `makerfoundry.app`. **Side
  effects of that testing:** three throwaway Foundry accounts (`laserfoundry-livecheck-<time>@example.com`, no email is sent by sign-up) and
  one tiny private draft, design 1 on MakerFoundry, which MakerFoundry's admins can delete. **Not checked:** the desktop app's own HTTPS client
  against the live services (the sandbox proxy re-signs HTTPS), a real MakerFoundry design opened from its page (the button is off until
  MakerFoundry sets `LASERFOUNDRY_OPENS_LINKS`), photos with a publish (the API takes them; the app sends none yet), and Windows and macOS.
  See `docs/makerfoundry.md`.
- **2026-10-31 — Version 0.2.0.** The workspace version is 0.2.0 (`Cargo.toml`, both lockfiles), ahead of tagging `v0.2.0`; it becomes the macOS
  app's version, the Windows file properties and the `app` name MakerFoundry logs. `docs/releasing.md` now lists this as the first release step
  (it was missing). The v0.2.0 notes are in `docs/releases/v0.2.0.md`. Nothing is tagged or published yet: that needs the owner (tag pushes
  and release creation fail from this sandbox).
- **2026-10-31 — v0.2.0 released.** Published as a full release on 657bd8b (version 0.2.0). The Desktop run succeeded: all three builds,
  Windows still finds the icon in `laserfoundry.exe`, `sign-windows` including "Verify the signatures", and `sign-macos` including notarization and
  stapling; the three downloads are attached and the `laserfoundry.app` buttons (`releases/latest/download/...`) now serve them. Checked from
  GitHub's workflow records and by fetching each download's first bytes; the signatures themselves, SmartScreen and Gatekeeper on a fresh
  machine, and the app on a real laser are still unchecked. The 0.1.0 and 0.2.0 releases are the only ones; 0.2.0 is the first with the working
  web app (0.1.0's page was blank from the 19 October rename until 31 October, which does not affect the downloads).
- **2026-10-31 — A name field in Publish to MakerFoundry.** MakerFoundry titles a draft from the file name it is sent, and an unsaved project was sent
  as `design.svg`, so its draft would be titled just "design" (MakerFoundry's chat pointed this out). The window now has a **Name** field: the
  project's name when there is one, empty for an unsaved project (Send waits for a name), and what the person types wins and stays. The name
  is made plain (letters, digits, spaces, hyphens, underscores; anything else becomes `_`; a typed `.svg` is dropped; 60 characters; no stray
  `_`, `.` or space at the ends). **Checked:** 14 UI tests for publishing and naming (a table of 24 typed names including paths, control
  characters, emoji, other scripts and an empty result, and a check that whatever is typed gives a name MakerFoundry takes), five deliberate
  breakages each caught (an empty name sent, the typed name ignored, an empty field falling back, no cleaning, a kept `.svg`; the last one
  was first a no-op that I noticed and redid), and the real desktop app run under Xvfb with a stand-in session: the saved project shows
  its name, the unsaved one shows the hint and a greyed Send, and typing into the field works. **Not checked:** a publish from the window
  with a typed name against the live service (the earlier live publish used the old fixed name), and the title MakerFoundry makes from a
  cleaned name (its own naming is not deployed yet).

