# Parity with the target application

> **Last updated:** 2026-10-14 · **Target:** LightBurn-class laser software (GRBL first)
> **Method:** self-assessed, deliberately low, all numbers *estimated* (nothing measured against the
> target; the clean-room rule forbids running it). Behaviour reference so far: general laser and GRBL
> conventions plus the GRBL project's public documentation. Before closing a gap, cite the public
> documentation or specification the behaviour comes from.

| Area                  | Score | Shipped                                                                 | Missing (highlights)                                                                                     |
|-----------------------|------:|-------------------------------------------------------------------------|----------------------------------------------------------------------------------------------------------|
| Drawing tools         |  70% | rectangle (rounded), ellipse, line, polygon, star, editable text (stroke font or filled outline fonts with kerning, text on a path), grid/circular arrays, groups, join, boolean ops, offset, node editing, pen/Bézier tool, create by numbers; move, resize, rotate, flip, align, distribute, nesting (grid-based packing with gap, margin and turning) | vertical/RTL layout controls, font fallback, weld/trim tools beyond booleans, nesting into holes and around shapes already on the bed |
| Import                |  55% | SVG (shapes, embedded pictures, text via installed fonts), DXF (lines, polylines, arcs, circles, ellipses, splines, block inserts), raster images with thumbnails, tone adjustments, crop (with canvas handles), resample, sharpen, trace to vectors | AI/PDF/EPS, DXF text/hatches/dimensions, curve-fitted and colour tracing, SVG gradients/filters |
| Layers & cut settings |  75% | 16 layers; line, fill, fill+line, image; speed, power, passes, interval, kerf offset, tabs/bridges, lead-in/out, perforation, cross-hatch, overscan, power ramp, Z offset and per-pass step; seven dither modes; per-shape overrides; presets (per project and shared) and starter presets; material test grid | sub-layers, a measured material library |
| Toolpath planning     |  60% | inner-first, nearest neighbour, duplicate removal, order by group, chosen start point, scan fill, dithered raster, kerf, tabs, lead-in/out, estimate by layer, cut selected only | overlapping-segment removal, acceleration-aware times |
| Output                |  55% | GRBL G-code (M3/M4, origin flip, bounds refusal), Marlin-laser and Smoothieware dialects (untested on hardware), Z moves, rotary Y scaling, frame, job origin (absolute / user origin / current position, 9-point anchor), run from a chosen line | Ruida/Trocen/TopWisdom, galvo, chuck rotary |
| Machine control       |  65% | persistent link: status/position, jog, home, unlock, set/go to origin, pause/resume, stop, console, streaming with progress, simulator, controller settings (the dollar-dollar report) editor, head shown on the canvas and click to move, auto-reconnect, Web Serial in the browser | camera capture, verification on real hardware |
| Preview               |   45% | toolpath preview with travel toggle, timeline playback and scrubbing with a head marker, time per layer, acceleration- and corner-aware times (look-ahead, `$11` junction deviation), colouring by layer, power or speed | a real controller's limited look-ahead buffer, 3D                                                        |
| Files & workflow      |  65% | native JSON project, atomic save, undo/redo (200) with a history window to jump to any step, autosave with three generations and read-back checks, crash recovery, unsaved-changes prompts, recent files, project templates, print-and-cut two-mark alignment, MCP server and JSON scripting, browser build with file open/save | scripting UI |
| UI                    |  45% | menus, tool bar, canvas pan/zoom/grid, rulers, guides, snapping (grid, edges/centres, vertices, guides), Shift constraints, layer palette, inspector, groups, undo, light/dark/system themes, web build | keyboard map editor, i18n, accessibility, layer list reorder by drag |
| Robustness            |  75% | no-panic lints, rollback on failure, hostile-input tests, mutation fuzz tests plus cargo-fuzz targets (found two real bugs), real-time-byte refusal in G-code, safe streaming | soak tests on real hardware, running the coverage-guided fuzzers |
| AI & accounts         |  35% | Design from Prompt and Redraw Image (SVG whose colours mean cut, score, engrave), Ask About Lasering (document as context), Check My Cut (photo to setting suggestions, applied on request); an account with credits on the shared service or your own API key; first-use consent; desktop and browser | any run against the real service and key (estimated: prompts need tuning), terms of service, saved conversations |

**Not yet verified on real hardware.** Everything on the machine side has been exercised against the
built-in simulator and unit tests only. Treat the first run on a real machine as a test: use Frame,
low power, and stay present.

## Evidence of what ships

- 473 tests across the workspace (`cargo test --workspace`); gates in `cargo xtask ci`.
- End-to-end: `laserfoundry-cli gcode demo.svg` → G-code; the desktop app was driven under Xvfb
  (draw with each tool, generate, start on the simulator, progress and completion shown; jog pad
  clicks moved the simulated head the expected way and the console logged each `$J=` line).
