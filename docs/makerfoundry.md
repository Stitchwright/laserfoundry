# MakerFoundry

[MakerFoundry](https://makerfoundry.app) is the Foundry family's marketplace for designs. LaserFoundry works with it in two ways.
MakerFoundry's side of the contract is in its [`docs/api.md`](https://github.com/SiteSentrix/makerfoundry/blob/main/docs/api.md);
the code here is `crates/ai/src/maker.rs` (requests, answers and the address checks, no network code),
`crates/ui-egui/src/maker_ui.rs` (the window and the hand-offs to the host) and `apps/laserfoundry-web` (the browser's fetch).

## Publish to MakerFoundry (File menu)
Sends the open drawing to MakerFoundry as an SVG, signed in with the person's Foundry Account (the same account and token as the AI
features: **AI › Foundry Account…**). MakerFoundry keeps it as a **draft** and answers with its page; the person's browser opens that page,
where the title, licence and price are set. Nothing is public until the person publishes it there.

- `POST https://makerfoundry.app/api/v1/designs` with `Authorization: Bearer <session token>` and
  `{file_name, file (base64), app}`. Only a Foundry session token (`sfs_…`) is ever sent there: an Anthropic key never is.
- The window has a **Name** field, filled with the project's name (empty for a project that has not been saved, and Send waits until
  there is one). MakerFoundry titles the draft from the file name, so the name is cleaned to letters, digits, spaces, hyphens and
  underscores (anything else becomes `_`, a typed `.svg` is dropped, at most 60 characters) and sent as `<name>.svg`.
  MakerFoundry's own sentence is shown as it is when it refuses (too many designs today, too big, not readable).
- The draft's address in the answer is opened only if it is on the same site the request went to.
- Works in the desktop app and the browser version (the page's content security policy and the host's allow-list name MakerFoundry).

## Open in LaserFoundry (a link on a MakerFoundry page)
MakerFoundry's **Open in LaserFoundry** button links to `https://stitchwright.github.io/laserfoundry/?open=<the design file's address>`.
The browser version reads it once at start, and:

1. checks the address strictly (`maker::open_link`): `https`, exactly `makerfoundry.app` or `www.makerfoundry.app`, no user name or port,
   a path under `/api/v1/`, no spaces, control characters, backslashes or non-ASCII, at most 2,048 characters;
2. takes `?open=` out of the page's address (so a reload does not fetch it again) and fetches **the checked address**, not the text
   that came in, with a plain GET: no cookies, no redirects, a time limit and at most 32 MB;
3. names the file from `Content-Disposition` (else the end of the address), requires `.svg` or `.dxf`, and opens it as if it had
   been dropped on the page. A refusal (`403` for sale, `404`, `429`) is shown in MakerFoundry's own words.

The desktop app has no link handler yet: the button opens the browser version.

## What was checked
- Unit tests for every part, with look-alike hosts, user-info and port tricks, traversal, encoded and hostile file names and headers,
  size limits, and answers that try to send the browser to another site; each protection was removed in turn and a test failed.
- A fuzz entry (`fuzzkit::maker`, `fuzz/fuzz_targets/maker.rs`) that also asserts anything accepted is MakerFoundry's.
- The real browser bundle in Chromium with MakerFoundry's reply mocked: the file opens, the page address loses `?open=`, a missing
  design shows MakerFoundry's sentence, and a look-alike or foreign host makes **no request at all**.
- The live services, using the app's own request builders (`crates/ai`), through `curl` from a sandbox: Foundry sign-up, the
  account, an Ask question (2 credits taken, balance matched), a wrong token read as "sign in", sign-out ending the session, and a
  publish to `makerfoundry.app` answered `201` with a draft page that the app read correctly.

Not checked: the desktop app's own HTTPS client against the live services (the sandbox's proxy re-signs HTTPS), a real MakerFoundry
design opened from its page (the button is off until it is switched on; the browser test used a stand-in for MakerFoundry), and
Windows and macOS.

## For MakerFoundry's chat
Once this is released and the hosted page serves it, MakerFoundry can switch the button on: set `LASERFOUNDRY_OPENS_LINKS` to `true`
in `apps/makerfoundry/src/api.rs`, and keep `MF_API_ORIGINS` listing `https://stitchwright.github.io`.
