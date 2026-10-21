# LaserFoundry: copy for the landing page

Material for the StitchFoundry team to build the LaserFoundry page from. Everything below is true of the
software today; the claims are kept to what is built and tested (see `target-app-parity.md` for the honest
status). Two rules for the page:

- **Do not name or compare with other vendors' laser software** on the page (trademark risk, and our clean-room
  rule). "Free alternative to paid laser software" says enough.
- **Keep the "early" and "not yet tried on real hardware" notes.** Overclaiming on a tool that drives a laser
  is a safety problem, and the honest note builds trust with makers.

## Names and links

| Item            | Value                                                          |
|-----------------|----------------------------------------------------------------|
| Name            | LaserFoundry                                                   |
| App (browser)   | https://stitchwright.github.io/laserfoundry/ (later https://laserfoundry.app/) |
| Source          | https://github.com/Stitchwright/laserfoundry                   |
| Licence         | MIT or Apache-2.0, free, no account needed for the core app    |
| Platforms       | Browser (Chrome, Edge and other Chromium browsers can talk to the laser over Web Serial), Windows, macOS, Linux desktop |
| Part of         | The Foundry family of maker tools (one Foundry Account for the AI features) |

## Tagline options

1. **Design it. Preview it. Burn it.** Free, open-source laser software.
2. **From idea to cut, in your browser.**
3. **The open laser workbench.**
4. **Laser software you can read, change and trust.**

## One-liner (for cards and link previews)

LaserFoundry is free, open-source software for laser engravers and cutters: lay out your artwork, set speed and
power per colour, preview the toolpath and send it to your GRBL laser, from your browser or desktop.

## Short description (about 60 words)

LaserFoundry takes you from artwork to finished cut. Draw shapes and text or import SVG, DXF and photos,
give each colour its own speed, power and passes, watch a preview of exactly what the laser will do, then
stream it to a GRBL machine, or to a built-in simulator when nothing is plugged in. Free, open source, and it
runs in your browser.

## Long description

Most laser software is either expensive, closed, or tied to one machine. LaserFoundry is a different kind of
tool: a complete laser workbench that is free, open source, and runs the same way in a browser tab, on your
desktop, or from the command line.

Lay out vector and photo artwork, assign cut settings by colour layer, preview the toolpath with a timeline
before a single beam fires, and send GRBL G-code straight to your machine over USB. If you don't have a laser
connected, a built-in simulator lets you try the whole workflow anyway.

It is written in Rust with safety as a design rule: the G-code generator refuses to move outside your bed
rather than quietly clipping, a cancelled job soft-resets the controller so the laser goes off, and the app is
built so that a bad file can't crash it and take your work with it.

## Feature blocks (headline + one or two lines each)

**Draw or import anything**
Rectangles, ellipses, polygons, stars, a pen tool with editable curves, text (a built-in single-line font for
engraving, or your own TrueType and OpenType fonts), and arrays. Import SVG, DXF and photos (PNG, JPEG, BMP,
GIF, WebP).

**Settings by colour**
Sixteen layers, each with line, fill, fill + line or image mode, its own speed, power range, passes, fill
spacing and angle, kerf compensation, tabs, lead-ins, air assist and more. Change a colour, change what the laser
does.

**Photos that engrave well**
Brightness, contrast, gamma, crop, sharpen and trace, then seven dithering methods (threshold, Floyd-Steinberg,
Atkinson, Jarvis, ordered, halftone and grayscale) so a photograph becomes a good engraving.

**Combine and shape**
Union, subtract, intersect and XOR shapes; offset to grow or shrink; join paths; align, distribute, rotate and
flip; undo up to 200 steps.

**See the job before you run it**
A toolpath preview with a play and scrub timeline, a head marker and a time estimate per layer. Frame the job
to see the boundary on your material first.

**Talk to your laser**
Live GRBL connection: jog pad, home, set origin, pause, resume and stop, a console, a controller settings
editor that warns when your laser mode or power range doesn't match the document, and streaming with progress.
Works over USB in desktop builds and over Web Serial in Chromium browsers.

**Place work on the material**
Absolute, user origin, or current-position job origin with a nine-point anchor. Two-point print-and-cut
alignment, and a photo of the bed as a backdrop.

**Safe by design**
Out-of-bed moves are refused, not clamped. Comments cannot inject commands. Cancel soft-resets the controller.
Unsaved work is autosaved and offered back after a crash.

**AI help, when you want it** *(optional, sign in with a Foundry Account)*
Describe a design and get vector artwork whose colours already mean cut, score and engrave. Redraw a picture as
line art. Ask a question about your machine and settings. Photograph a test cut and get suggested speed and
power changes you can apply with one click. Nothing is sent until you say so, you can switch each AI feature off,
and a local log shows exactly what was sent.

**Scriptable**
A command-line tool turns SVG into G-code in one line and runs JSON scripts. It also speaks the Model Context
Protocol, so AI agents can drive LaserFoundry as a tool.

## Who it is for

- Hobbyists with a diode laser (GRBL controllers) who want free, capable software.
- Makers and small shops who prefer open tools they can inspect and fix.
- Schools and makerspaces: runs in a browser, nothing to install, no licence keys.
- Developers and tinkerers who want to script or extend a laser workflow.

## Why it is different (three bullets for a comparison strip)

- **Free and open.** MIT or Apache-2.0. Read the source, fix a bug, send a patch.
- **Runs anywhere.** The same engine powers the browser, desktop and command line.
- **Safety first.** Refuses unsafe output instead of guessing, and is tested against hostile files.

## Honest status (put this on the page)

LaserFoundry is early. The core pipeline (design, layers, preview, G-code, GRBL control) works and is well
tested, but it has so far run only against a built-in simulator and automated tests, not yet on real laser
hardware, and it is a fraction of what the biggest commercial tools do. Start with the Frame button and low
power, and stay with your machine. A laser is a fire and eye hazard: wear eye protection and never leave a
running laser unattended. LaserFoundry is independent software, unaffiliated with any laser or software vendor.

## Call to action

- Primary button: **Open LaserFoundry** (the app).
- Secondary: **View on GitHub**.
- Tertiary: **Report a bug** (Help menu in the app, or GitHub issues).

## FAQ

**Is it really free?** Yes. The app, desktop builds and command-line tool are free and open source. Only the
optional AI features use credits, through a Foundry Account.

**Do I need an account?** Not for designing, previewing, saving or running a job. A Foundry Account is only for
the AI features.

**Which lasers work?** GRBL controllers (most hobby diode lasers). Marlin and Smoothieware output and a Marlin
controller link exist but are untested on hardware. Other controller types are not supported yet.

**Does it work in my browser?** Yes. Chrome, Edge and other Chromium browsers can also connect to the laser over
Web Serial. Firefox and Safari can design and export G-code but can't connect to the machine directly.

**Where do my files go?** Nowhere. Projects are saved on your own computer. The AI features only send what the
first-use dialog says, and only when you ask.

**Can I use my designs from other programs?** Import SVG, DXF and common image formats. Native projects save as
`.laserfoundry` files.

## Search keywords

free laser software, open source laser engraver software, GRBL laser software, diode laser software, laser
cutter software, SVG to G-code, laser engraving software browser, image to G-code, laser dithering, Web Serial
laser.

## Screenshots to capture

Canvas with a design and layer palette; toolpath preview mid-playback; photo dithering before and after; the
machine panel with the jog pad; the AI Design from Prompt window with a result. (Take them from the real app;
no third-party artwork.) One is ready: `docs/screenshots/canvas.png`, the desktop app (1280x800, software
rendering) with a small demo design from scratch: the layer palette is on the right and the colours of the
drawing are the layers that decide cut, score or engrave. It is an original screenshot of our own app; the page
should add the toolpath preview, the dithering comparison and the AI window.
