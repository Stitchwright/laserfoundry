# First run on a real diode laser

A checklist for the first sessions with a real GRBL laser (gap 1 in `gaps.md`). Everything machine-side so far
was tested against the built-in simulator, so **expect surprises and treat the first runs as tests of the
program, not just of the laser.** The aim is to find what breaks, safely, and to bring back enough to fix it.

## Safety first (read before connecting)

- **Eyes.** Wear laser safety glasses rated for your laser's wavelength (a blue 445 nm diode needs different
  glasses from a 1064 nm or a CO2). Everyone in the room wears them. Never look at the beam or its reflection.
- **Fire.** Never leave a running laser unattended. Keep a fire extinguisher or a bucket of water within reach;
  work on a non-flammable surface; use air assist and ventilation; keep paper, cloth and fuel away.
- **Materials.** Only known materials. Never PVC, vinyl, or anything that may contain chlorine; avoid unknown
  plastics, leather of unknown tan, coated metals.
- **A hand on the stop.** Know the machine's own emergency stop or power switch before the first move. The
  program's **Stop** sends a soft reset (laser off), but it is software: the switch is your last resort.
- **Start with the beam off or at its lowest power.** Test motion with the laser disabled, then at 1 % power
  on a scrap, then raise power in small steps.

## If it will not connect or complains

- A port that will not open says why when it can: on Linux your user must be in the `dialout` (or `uucp`)
  group; on any system another program (a laser program, a terminal) may hold the port; a gone port means a
  loose cable. Close other programs that use the laser before connecting.
- Most boards reset when the port opens: the console shows the `Grbl 1.1…` banner a second or two later, and
  nothing is sent to the controller before it answers.
- GRBL's numbered replies are explained in the console and in errors (`error:9 (G-code is locked out…)`,
  `ALARM:3 (the machine was reset while moving…)`); other firmware may number things differently, so the
  number is always shown too.
- A job will not start while the controller reports an alarm, hold, open door, homing or movement: the message
  says which. After an alarm use **Unlock** (`$X`) or **Home** (`$H`) first.

## Before the first job

1. **Connect** (Laser panel → choose the serial port → Connect). Note the port name and what the console shows
   (`Grbl 1.1…`, any alarm).
2. **Read the controller settings** (`$$`, the settings section). The program flags what it can:
   - `$32` must be `1` (laser mode). With it `0` the program refuses to start a job. Turn it on from the
     settings section, then re-check the console.
   - `$30` is the S maximum (often 1000 or 255); the document's "S max" must match, or power percentages come
     out wrong. Use the "Use controller's" button when it differs.
   - `$31` is the S minimum (normally 0). `$130`/`$131` are the travel in X and Y: set the document's bed to
     match; the program warns if they differ.
   - `$110`/`$111` (max rate) and `$120`/`$121` (acceleration) decide how fast the machine can move; keep job
     speeds below the max rate.
   - `$20` (soft limits) and `$21` (hard limits) protect the frame if you have homed; ask yourself whether you
     have homing switches (`$22`). Without homing, set the work origin by hand (below) and keep jobs small.
3. **Save a machine report** (Laser → Save machine report…) now and keep it: it is the baseline to compare against.

## Motion test (laser off)

1. **Jog** a few millimetres each way with the jog pad. Confirm the directions match the screen: pressing
   "up" should move the head the way the on-screen head moves. If an axis is reversed, that is a controller
   setting (`$3`, direction invert) or the document's origin corner: note which.
2. **Set the origin** where you want the job to start (the head's position) and use **Go to origin** to check.
3. **Frame** the job (Frame button) at the lowest power the machine allows, or `0 %` if your controller does
   not light at `S0`. Watch that the head traces the job's outline where you expect on the material, and that
   nothing hits the frame. Wrong place or wrong size is a bug worth reporting (see below).

## First marks (low power, scrap)

1. Make a small test: a 20 mm square and a line, on a scrap of known wood or card, in **Line mode, one pass**.
2. Use the **material test grid** (Arrange or Generate → Material Test Grid) with a speed range and a power range
   suited to your laser, small cells, on scrap. This is the fastest way to find workable settings.
3. Start at the low end, watching the whole time. Stop at any smoke you did not expect, flames, a sound that
   changes, or a move that looks wrong: press **Stop**.
4. Compare the result with the preview: same place, same size, same order? Measure a 100 mm line with a rule and
   note the error in each axis (a steps-per-mm error is a controller setting, `$100`/`$101`).

## What to bring back

For each problem or surprise, save and keep:

- the **machine report** (Laser → Save machine report…) right after the problem, before disconnecting;
- the **project file** and the **generated G-code** (Laser → Generate G-code, then the console or a saved file);
- what you **expected**, what **happened**, and a **photo** of the result if it is on the material;
- the **laser model**, controller board, firmware version (`$I` in the console), OS and how it is connected;
- **measured** numbers (a rule's reading) marked as measured, and anything estimated marked as estimated.

Use the `/debug` workflow: write the symptom down without a theory, reproduce it once more, and keep a log in
`docs/debug-logs/` for anything that takes more than one sitting. A bug that can leave the laser on, ignore the
bed limits or lose work is the first thing to fix.

## Things the program has never done on real hardware

Jog and jog-to, frame, a short cut, pause and resume, stop (soft reset), run from a line, the reconnect after a
unplugged cable, the controller settings editor's writes, the laser-mode refusal, and the first-use behaviour of
a real serial port (baud rate, line endings, buffer sizes). Please try them one at a time, in that order, and
say which worked.
