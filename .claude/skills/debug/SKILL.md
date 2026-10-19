---
name: debug
description: Systematic debugging workflow (reproduce, probe, examine, fix) for LightFoundry. Use when a test fails and the cause is not obvious, a G-code or toolpath result is wrong, the UI misbehaves, a crash or hang is reported, a change "should have helped" and did not, or any bug resists the obvious approaches. Adapted from Kuhlmann, The Book of Debugging (No Starch Press); chapter numbers refer to it.
---

# Debugging workflow

Use this when something fails and the cause is not obvious. The core idea: a computer is deterministic, so
every bug has a cause that can be found. A workflow keeps effort from scattering. Loop through the four
steps until the bug can no longer be reproduced; always start with Reproduce.

LightFoundry drives a laser, so a bug that can leave the beam on, ignore the bed limits, or lose work comes
first: stop, make it safe (refuse, not clamp), then investigate.

## 0. Before anything else

- **Take responsibility.** Assume the bug is in our code, not in egui, a crate, the OS or the controller.
  Third-party code that many people use is less likely to be wrong than code written for this project.
- **Write the symptom down** (see the log below): what happens, what you expected, the exact command or
  steps. Symptoms only, objectively; keep theories out of that line, they bias the search.
- One bug at a time. Finish or park it before starting another.
- If the report is thin, it is still probably real. Ask what is missing (version, platform, the file,
  "when did it last work?") instead of dismissing it.

## 1. Reproduce (ch. 1-3)

- **Make it happen on demand** from a named input: a JSON command script (`lightfoundry-cli run`), a test, an
  SVG/DXF file, or numbered steps in the app. If you cannot reproduce it you are missing a factor (platform,
  data, order, timing, a stale build): keep looking, do not guess a fix.
- **Do it the way the reporter did**, in the real app, before reading code. A frame test or a command is
  usually the shortest path; use the simulated controller instead of hardware.
- **Shorten the loop first.** `cargo test -p <crate> <test name>` beats `cargo xtask ci`; build in your own
  `CARGO_TARGET_DIR`. A fast loop is worth more than a clever theory. Shorten only while the bug still shows.
- **Cut the case down.** Fewest shapes, layers, characters, lines of G-code that still fail. If it stops
  failing, the part you removed matters: put it back in halves.
- **Vary one thing at a time** (order, timing, size, layer settings, origin mode, dialect) to learn which steps
  are essential. If a new symptom appears elsewhere, an earlier step probably corrupted state.
- **Automate the repeat**: a script, or a test that calls the suspect function directly in a loop over the
  inputs. For rare bugs, run it many times (`fuzzkit` mutation tests are this). The automation must leave
  evidence (a message, a dump), not just fail.
- Cannot automate it? A human factor (timing, a click sequence) matters. That is information.

## 2. Probe: find an entry point (ch. 4-12)

- **Educated guess, then stop.** Try shortcuts from experience for about an hour, or two or three tries, or
  until the guesses stop being based on evidence. Then switch strategy and cross the ruled-out systems off.
- **Suspect the glue** between systems and the code that has broken before, before old trusted code. Heavily
  commented or often-patched code is a sign of difficulty, not blame.
- **Bisect when it used to work** (ch. 6): find a good and a bad commit and `git bisect` between them (build or
  reuse binaries), then read the offending diff for the one change that matters. Look at other branches too.
- **Search everything** (ch. 11): the message text (partial strings: text is composed in code), the command id,
  `crates/i18n/engine-messages.txt`, docs, ROADMAP and gaps, git history, tests. Use unique names and markers
  when you write code so you can find it again.
- **Instrument, do not theorise** (ch. 8): look at the data the program produced (`file.project_data`, the
  generated G-code, `job.gcode`, a preview) before reasoning about it. Use `log::debug!`, never `println!`
  in library code. No panics: shipped code returns `Result`; an assertion belongs in a test. Keep new
  instrumentation optional and off by default, and **re-check the bug still happens after adding it**
  (a log line once "fixed" a bug by changing timing).
- **Remove randomness** (ch. 9): fixed seeds (the mutation tests use fixed ones), fixed order, no clock, one
  thread. A "random" failure has a cause: time, uninitialised data, threads (the machine link and its
  worker), an RNG, external input, or a stale file. Replay recorded input where you can.
- **Do not trust a build you have not seen change.** Make an *obvious* change and confirm it shows up before
  believing a result: stale `target/`, a stale `apps/lightfoundry-web/pkg` (rebuild with `build.sh`), the
  wrong data folder (`XDG_DATA_HOME`), a cached font or autosave, the wrong branch.

## 3. Examine: find the root cause (ch. 13-19)

- **Follow the data** (ch. 15): find the original input, follow it through each transformation (import, layer
  assignment, path ops, toolpath, G-code), and find where it first becomes bad. Compare a good and a bad
  case side by side. Distinguish *undesired* data (the logic is wrong) from *corrupt* data (an
  implementation bug): look for NaN, infinity, negative sizes, huge numbers, empty paths.
- **Question everything** (ch. 16): list your assumptions and verify each one. Do not trust a feature because
  it exists: check that it fires (count what it covered, how often a branch ran). A symptom can sit far from
  its cause; the cause is in something you assumed.
- **Read the code as the machine would** (ch. 14): execute it by hand, line by line, recording intermediate
  values; do not exempt code you "know" works. Know the Rust traps here: `as` casts that saturate or wrap,
  float comparison and non-finite values, integer overflow in release builds, off-by-one at the first and
  last element, slices indexed by data.
- **Experiment with a hypothesis** (ch. 17). For each one write down: the hypothesis, how it will be tested,
  and what you will learn either way. Test with the least effort; sloppy code is fine for a test. Try the
  hackiest possible fix first to confirm you found the right place, then write the clean one and keep the
  dirty one to compare. One hypothesis at a time; a fix can expose the next bug.
- **Make experiments revertible**: mark them with a unique comment, and keep the tree clean so `git diff`
  shows exactly what you changed. Never commit an experiment marker.
- **Do not revert an idea after one bad run** until it has been re-tested on a baseline known to be correct;
  if a baseline bug turns up later, re-run the reverted experiments.
- **Keep a debugging log** (ch. 18), and write down a new idea instead of chasing it at once: finish the
  current lead, then order the leads by effort against likelihood. Always have a next lead.
- **Build the tool you keep wishing for** (ch. 19): a flag, a command (everything is a command), a test
  helper. A hidden tool does not help; document it.
- **Talk it through** (ch. 23) when stuck: stating the problem exposes the assumption that is wrong. Sleep on it.

## 4. Fix (ch. 23-30)

- A good fix is minimal, complete, revertible and addresses the root cause, not the symptom. One bug per
  commit. If the clean fix is not possible yet, say that a workaround is one, and record what remains.
- **Verify** (ch. 23): reproduce it several times; undo the fix and confirm the bug returns; re-apply and
  confirm it goes. **A crash fix lands with a regression test that fails without it** (repo rule): run the
  test with the fix removed to prove it.
- **Check side effects separately**: run the whole gate (`cargo xtask ci`) on current stable Rust before
  relying on a green local result; look at the real app or a screenshot for UI work.
- **Share what you learned** (ch. 25): the commit message names the symptom, the root cause and the fix; add
  the ROADMAP entry; ask how the cause got in and what safeguard stops it recurring (a lint, a test, a
  cap, a check in `xtask`).
- **Search for the family** (ch. 26): the same mistake may exist elsewhere (another importer, another command,
  the other dialect). Fix them while the context is fresh, with a test each.
- Where you cannot check something (real hardware, a real service), say so and say how it was checked
  instead (measured against the simulator, a local fake, a unit test). Never report "verified" for what was
  not.

## Anti-patterns (avoid)

- Changing parameters one at a time hoping a number moves (desperate guessing).
- Reverting an idea on one bad run; trusting a baseline that had a latent bug.
- Taking a count as proof without looking at why.
- Running a long experiment when a short one answers the same question.
- Claiming a feature works because it compiled; measure that it did what you think.
- "Fixing" a bug by adding a log line, a sleep or a retry without finding why (it will come back).
- Pushing back because the report is thin or "that cannot be broken".

## Debugging log template

For a bug that takes more than one sitting, create `docs/debug-logs/YYYY-MM-DD-<short-topic>.md` (keep it
in the repository and name it in the commit):

```
# <topic>  (command or test, commit)
Symptom (objective, no theory):
Expected:
Repro (fast version: ...):
Frequency / measured numbers (say measured or estimated):

## Leads (prioritised: effort vs chance)
1. ...

## Experiments
- [date] Hypothesis / how tested / result / what it rules in or out

## Ruled out
- ...

## Root cause / fix / verification / related places checked
```
