# Gaps

Every known shortfall, ranked. This is the work list; pick from the top. Scores and evidence are in
[`target-app-parity.md`](target-app-parity.md).

| #  | Gap                                                                                  | Why it matters                                                      |
|----|--------------------------------------------------------------------------------------|---------------------------------------------------------------------|
| 1  | Verify on real GRBL hardware (jog, frame, short cut), fix what it finds (a checklist and a machine report exist: `hardware-bringup.md`) | Everything machine-side is only simulator-tested                    |
| 2  | Resume after a power loss, checked on a real machine: the saved position is offered at next start (run-from-line, auto-reconnect, the $$ editor and the saved job position are done); the head must be re-homed by the person, and a real-machine check is still owed | Day-to-day machine ownership                                        |
| 14 | A real material library needs measured values per machine (generic starters, per-project presets, a shared preset library and test grids are done) | Reusing settings for a known material                               |
| 15 | Other controllers: verify the Marlin/Smoothieware dialects on hardware and add a link for them; Ruida is a large project | Reach beyond GRBL                                                   |
| 16 | Live camera capture and lens calibration (two-point print-and-cut alignment is done; a photo of the bed can serve as a backdrop on a non-output layer) | Placing work on printed or irregular material                       |
| 18 | Browser build: try Web Serial on real hardware; installing as a PWA needs a service worker (JavaScript, which the Rust-only rule forbids unless generated from Rust) and an icon drawn in code | Web users can design and generate G-code today |
| 20 | Galvo/fibre/CO2-RF controllers (proprietary protocols); a tilted or 3D Z map; chuck rotary with steps-per-rotation |  Hardware variants |
| 21 | Translations: font-download messages are still English; the browser build has no font for non-Latin scripts; no translation ships (see `translations.md`) | Users outside English-speaking markets |
| 22 | AI features and accounts: try them against a real service and key and tune the prompts; the apps now default to the shared Foundry service (`cloud.stitchfoundry.app`) but have not been tried against it signed in; terms of service for the service | The AI menu is built and tested only against scripted answers |
