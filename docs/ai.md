# AI features and accounts

> **Status:** 2026-10-18. The request and reply code is written and tested (`crates/ai`); the interface
> and the hosts' network clients are next (see the ROADMAP log).

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

## What is not done yet

The account window, the four dialogs, the hosts' network clients and the session token's storage; a
privacy note shown before the first request; terms of service (the service owner's).
