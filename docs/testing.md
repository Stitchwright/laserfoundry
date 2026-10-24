# Testing

`cargo xtask ci` is the gate (format, clippy, every test, layers, clean-room, brands, assets, messages, wasm). This page is
about the tests themselves.

## Layers of tests

| Layer                    | Where                                          | What it protects                                            |
|--------------------------|------------------------------------------------|-------------------------------------------------------------|
| Unit and module tests    | `crates/*/src` (`mod tests`)                   | One function or type at a time, hostile inputs included     |
| Engine command tests     | `crates/engine/src/tests.rs`                   | Each command, undo, rollback, `command_sweep_never_panics`  |
| UI frame tests           | `crates/ui-egui/src/tests.rs`                  | Panels and interactions fed real pointer events             |
| Fuzz entry points        | `crates/fuzzkit`, `fuzz/`                      | Importers, parsers and the controller protocol              |
| **End-to-end regression**| `apps/laserfoundry-cli/tests/regression.rs`    | The real binary, whole pipeline, byte-exact G-code          |

## The regression suite

`cargo test -p laserfoundry-cli --test regression` runs the `laserfoundry-cli` binary on inputs written inside the test (no
third-party files):

- **Golden programs** (`tests/golden/*.gcode`): SVG, DXF and raster-image jobs, and scripts for shapes, booleans, arrays, fill
  layers, passes and kerf, text, material test grids, nesting, node editing and offset, resume-from-line. Output is compared
  byte for byte, and the first differing line is reported.
- **Invariants on every program**: millimetres and absolute moves set up front, the laser switched off, no non-finite numbers,
  every `X`/`Y` inside the bed, no negative power or feed.
- **Behaviour**: save and reopen gives the same program; undo and redo restore earlier and later programs; a failing step rolls
  back; out-of-bed jobs are refused and write nothing; "cut selected only"; SVG export re-imports with the same shape count;
  `info`.
- **Command catalogue** (`tests/golden/commands.txt`): adding, removing or renaming a command is a visible change.
- **Hostile input through the binary**: broken SVG, DXF, PNG and project files, broken scripts and arguments, absurd
  parameters, and every command run with empty parameters must give an error, never a crash (exit code 101, a signal or a
  panic message).

### When a golden changes

A failure means the output changed. Read the diff the test prints. If the change is wanted (a deliberate planner or generator
change), re-record and review the result in git:

```
LF_BLESS=1 cargo test -p laserfoundry-cli --test regression
git diff apps/laserfoundry-cli/tests/golden
```

Never re-record to make a failure go away without knowing why the output moved. A change in G-code can move a laser, so say
in the commit what changed and why.

### What it does not cover

Real hardware (no test here talks to a machine), the egui window beyond its frame tests, the browser build at run time, and
the signed installers. See `docs/hardware-bringup.md` and `docs/releasing.md` for those.
