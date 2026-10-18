# AI features and accounts

> **Status:** 2026-10-18. The request and reply code (`crates/ai`), the engine commands, the AI menu and its
> windows, and the desktop app's network client are built and tested. Not hosted or tried against a real
> service or key yet; the hosted browser build does not offer the AI menu yet (see the end).

Four features use an AI model (Claude, through the Messages API). Each is paid for in one of two ways:
the person's **own Anthropic API key**, or a **session with the account service**, which takes credits.

| Feature            | What it does                                                                                                       |
|--------------------|--------------------------------------------------------------------------------------------------------------------|
| Design from Prompt | Draws vector artwork from a description: red outlines cut through, blue lines scored, black shapes engraved        |
| Redraw Image       | The same, from a picture the person owns (a logo of their own, a drawing)                                          |
| Ask About Lasering | Answers questions with the open document's layers and machine as context                                           |
| Check My Cut       | Reads photos of a test cut and suggests setting changes (speed, power, passes), which the person can apply         |

## Design rules

- **No network code in `crates/ai`.** Every function builds an `HttpRequest` for the host to send or reads a
  status and body the host got back, so the browser build can use the crate. The desktop app sends with its
  own client; the browser sends with `fetch` (the page's content security policy then has to name the
  service).
- **Replies are untrusted.** Artwork goes through the SVG importer (size-capped, nesting-bounded); a check's
  suggestions are read from one JSON object and kept only when the layer exists and every number is in a
  sensible range (`check::read_check`), so a confused answer can never set a layer to nonsense. Applying one is
  a normal `layer.set` command, one undo step, only when the person asks.
- **The document is context, not instructions.** What is sent about the open document (layers in use, machine,
  notes the person typed) is labelled as data in the prompt.
- **A key is never sent to the service, and a session token never to Anthropic** (`cloud::messages` refuses a
  token that does not start with `sfs_`).
- **Safety in the prompts.** The model is told to mention fire and fumes, never to advise cutting PVC or
  anything with chlorine, to say when it does not know, and to say stop first when a photo shows flames.
- **Copyright.** Design from Prompt declines other people's logos, trademarks and characters, with a reason
  shown to the person.

## The account service

One service answers every app that uses it, so an account and its credits are shared. Its address is set in
the account window (or at build time); it must be HTTPS, or plain HTTP on this computer for local use.
Endpoints, all `POST` with a JSON body and `Authorization: Bearer <session token>` except sign-up and sign-in:

| Endpoint            | Body                    | Answer                                                                    |
|---------------------|-------------------------|---------------------------------------------------------------------------|
| `/v1/signup`        | `{email, password}`     | `{token, account: {email, credits}}`                                      |
| `/v1/signin`        | `{email, password}`     | `{token, account}`                                                        |
| `/v1/signout`       | `{}`                    | ends the session                                                          |
| `/v1/account`       | `{}`                    | `{email, credits, packs: [{id, credits, cents}], payments, history}`      |
| `/v1/checkout`      | `{pack}`                | `{url}`: a payment page for the person's browser (the service never sees card details) |
| `/v1/messages`      | an Anthropic request    | the Anthropic answer, paid for in credits                                 |

Errors come in Anthropic's shape; 401 means sign in again, 402 not enough credits, 409 the email is taken,
429 wait, 503 the service is not set up. Prices, packs and free credits are the service owner's settings.
The service's own documentation is the reference for what it keeps and how it is hosted.

## Where things are

| Part                                         | Where                                                                                           |
|----------------------------------------------|-------------------------------------------------------------------------------------------------|
| Requests, replies, errors, the account API   | `crates/ai` (no network code; fuzzed: `ai_reply`)                                               |
| Describing the document, placing artwork, preparing a picture | engine commands `ai.context`, `ai.place_art`, `ai.prepare_picture`              |
| The AI menu and its windows                  | `crates/ui-egui/src/ai_ui.rs` (the menu appears only where the host can send requests)          |
| Sending, and keeping the session token       | `apps/lightfoundry/src/net.rs` (no redirects, 240 s timeout, answers capped at 16 MB; the token and key live in `secrets.json` in the app's private data folder, readable by its owner only) |

The service's address is typed in the account window, or set at build time with `LF_CLOUD_URL` (the browser
build takes only the built-in address).

## What is not done yet

- **A real try.** Everything is tested against scripted answers and a local one-shot server, never a real
  account service or Anthropic key. The first real run may need prompt tuning (the artwork's colours, sizes and
  the line widths the importer reads).
- **The hosted browser build** has a `fetch` client (`apps/lightfoundry-web`, no redirects, 240 s limit) and the
  AI menu, but only for the service address it was built for: set the repository variable `LF_CLOUD_URL` (an
  https address; Settings › Secrets and variables › Actions › Variables) and the Pages build puts that origin
  in the page's content security policy and into the app. Without it the AI menu is hidden. Pictures come through the browser's
  file chooser, so all four features and the account work there. The session token is kept in the browser's local storage (the policy keeps scripts
  from elsewhere out). The service must answer cross-origin requests (the StitchFoundry service does).
- **Terms.** StitchHub's draft terms (`legal/terms-2026-10-16-draft.md` in the `stitchhub` repository, not in
  force, awaiting a lawyer) say they do not apply to StitchFoundry or LightFoundry, so the app does not link or
  claim them. It follows the principles of their section on the Assistant instead: an AI only runs when a
  button is pressed; each feature has its own switch (Account window); the person is told what is sent and
  where; a log of every request and answer is kept on the person's computer (`ai-log.jsonl` in the data
  folder, owner-only, pictures and tokens left out) so they can check; answers can be wrong and must be read
  before use; and the AI must be used lawfully, with a pointer to Anthropic's usage policy. Terms for the
  Foundry apps and the account service are still the owner's to write.
- **Privacy note (old wording).** Before the first AI request the app asks (Send / Send and do not ask again / Cancel),
  naming where it goes and what: the description or question, any picture, and a plain-text summary of the
  machine and layers in use, never the artwork. A privacy note and terms of service for the service are its
  owner's to write.
- **Command line and agents.** `ai.context`, `ai.place_art` and `ai.prepare_picture` are commands any agent can
  call, but the requests themselves are made only from the windows.
- **Saved conversations and checks**, pictures from the browser's file chooser, and sharing checks to improve
  the service (the StitchFoundry service has this for its own check).
