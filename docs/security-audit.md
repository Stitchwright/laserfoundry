# Security audit, 2026-10-17

A full pass over the repository: dependency advisories, secrets, `unsafe`, the file and command surface,
the local agent link (MCP), G-code and the machine link, the font download, the browser build and the GitHub
workflows. Method: `cargo audit` and pattern scans by hand; two independent read-only code reviews (one for
files, commands and the agent link, one for G-code, the machine link, networking, the browser build and the
workflows) whose findings were checked against the code. Not done: a dynamic test on real hardware, a
third-party review, or a dependency licence review.

Result: no critical issue, nothing that leaves the laser on or injects G-code, no known-vulnerable dependency
(`cargo audit`, 459 crates, 1334 advisories), no secrets in the tree.

## Fixed in this pass

| Finding                                                                                                   | Fix                                                                                       |
|-----------------------------------------------------------------------------------------------------------|-------------------------------------------------------------------------------------------|
| Importing or opening a path such as `/dev/zero` read until memory ran out (size came from file metadata)   | `format::read_capped` reads at most the limit; used by open, SVG/DXF/picture import       |
| The agent-link token fell back to a time-seeded value where `/dev/urandom` is missing (Windows)            | OS generator through `getrandom`; no token means no link                                  |
| A client could hold a link slot for ever by sending one byte at a time before the token                    | One total deadline for the handshake                                                      |
| An array of many selected shapes could multiply into millions of copies                                    | The shapes times the copies is capped at 200,000                                          |
| After a failed write mid-job the link dropped the job without trying a soft reset                          | A soft reset is attempted first                                                           |
| The browser's serial input buffer had no bound                                                             | Keeps the newest 64 KiB                                                                   |
| Saving wrote through a predictable temp name that could be a planted link                                  | A fresh, unguessable name created with `create_new`; removed on failure                   |
| The browser page had no content security policy                                                            | CSP with a hash for the one inline script, same-origin connections only                   |
| `desktop.yml` gave every build job a write token                                                           | Builds read-only; only the release job can write                                          |
| Checkouts kept the token in `.git/config`                                                                  | `persist-credentials: false` (CI, Desktop, Fix workflows)                                 |
| No automated advisory or action updates                                                                    | `audit.yml` (weekly and on `Cargo.lock` changes) and `dependabot.yml`                     |
| The bug-fix workflow ran `cargo test` while holding the API key, on untrusted issue text                   | It now runs no builds or tests (the pull request's CI runs the gate); keep requiring a maintainer to apply the `claude-fix` label after reading the issue |
| Actions were pinned to tags, not commit hashes                                                            | Every action is pinned to a commit hash (with its version in a comment); Dependabot bumps them |
| Agents in the open window could write any file the user can (`file.save`, `file.autosave`, exports)         | Over the live link they may only write beside the open project or in the temporary folder, never onto a link |
| Undo history had no memory budget (200 steps, each a clone of the document)                                | Also bounded to about 256 MB of estimated shape data; the newest step is always kept       |
| The link's queues were unbounded after the token check; a client that never reads could grow memory        | Bounded request and reply queues; an agent whose replies pile up, or whose writes stall for 30 s, is dropped |
| Autosave, recovery, recent list and presets used default permissions                                       | The app's data folder is owner-only (mode 0700 on Unix; Windows relies on the profile folder) |
| Current-position and user-origin jobs could not be checked against the bed and started without a word      | Starting one asks for confirmation first (with "do not ask again in this session")         |
| Closing a browser tab mid-job (or with unsaved work) gave no warning                                       | The page asks the browser to confirm before closing or reloading while a job runs or work is unsaved |
| Closing one app instance removed another's link file                                                       | Only the file that still names this instance's port is removed                             |
| A controller not in laser mode (`$32=0`) could fire during travel moves of a started job                   | The app reads the controller's settings once it answers and refuses to start or frame a job while `$32=0`; the settings panel can turn it on |

## Still open

1. **Three "unmaintained" crates** (`zbus_names`, `zvariant`, `zvariant_derive`, RUSTSEC-2026-0370 to -0372,
   reported 2026-10-16) arrive through the Linux screen-reader support (`accesskit_unix`, from egui's
   `accesskit` feature). They are not vulnerabilities; the weekly audit shows them without failing. Watch for
   an `accesskit` release that moves to a maintained replacement.

2. **A hostile project file** can still hold up to 100,000 shapes (the importer's cap); the limits above keep
   copies and history bounded, but very large documents are slow. Not a safety issue.
3. **Not covered by this audit**: a test on real hardware, a third-party review, a dependency licence review,
   and the Windows owner-only permission on the data folder (it relies on the profile folder's access list).
