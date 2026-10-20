# Translations

LaserFoundry ships in English only, with no translation or font files (the assets rule). You can
supply your own.

A translation is a JSON file:

```json
{
  "name": "Deutsch",
  "font": "/path/to/a/font/that/has/your/script.ttf",
  "strings": { "Open…": "Öffnen…", "Rulers": "Lineale" }
}
```

- `strings` maps the English text, exactly as the program shows it, to your text. Anything missing
  or empty stays English.
- Texts with values in them are keyed with named places, for example `"Job finished: {lines} lines"`;
  keep the `{name}` places in your translation, in whatever order your language needs. A place you
  leave out simply is not shown; one that is not recognised is left as written.
- `font` is optional. The built-in font covers Latin scripts; for others (Cyrillic extras, Greek
  polytonic, CJK, Arabic, …) name a `.ttf` or `.otf` that has the characters. It is added behind the
  built-in one.
- Use **View > Language > Save translation template…** to write every text the program has shown so
  far, each as its own translation, then edit the values. Open the panels and menus you want covered
  before saving; the template lists only what has been drawn.
- **View > Language > Load translation…** picks a file, keeps a copy in the data folder and uses it
  from then on. **Use English** removes it.

Limits: 4 MB per file, 20,000 strings, 2,000 characters per text, 32 MB per font.

## Messages from the engine

Errors and some results come from the engine in English, with values in them ("shape 12 does not
exist"). They are translated by template: put the template in `strings` with its places named, and
the values are picked out of the message and put where your text says.

```json
"shape {id} does not exist": "Form {id} gibt es nicht",
"layer {1} does not exist": "Ebene {1} gibt es nicht",
"bad parameters: {0}": "falsche Angaben: {0}"
```

- Places are written as in the list of messages: a name (`{id}`), a number (`{0}`), or, where the
  source had a bare `{}`, its position (`{1}`, `{2}`, …). Use the same names in your text, in any order.
- A message that wraps another ("bad parameters: …") translates both: the inner message is
  translated by its own entry.
- **Save translation template** already includes all of these (about 135, from
  `crates/i18n/engine-messages.txt`, which `cargo xtask messages` keeps in step with the source and
  `cargo xtask ci` checks).
- Where two templates could match, the one with more fixed text wins.

## What is covered

Menus, buttons, labels, tooltips, headings, drop-downs, the status-bar messages the desktop app
writes itself, engine errors, and the font-library download's results and failures. Not covered yet: the technical detail inside a
download failure (the operating system's or the network's own words, and the browser build's low-level fetch errors stay as
reported), and a font for non-Latin scripts in the browser build.

## In the browser

The hosted build has the same **View > Language** menu. *Load translation…* opens a file chooser, the
catalog is kept in the browser's local storage and used on the next visit, *Use English* forgets it, and
*Save translation template…* downloads the template. The `font` entry is ignored there (a page cannot read
a font path), so scripts the built-in font lacks are not available in the browser yet.

## The command line

`laserfoundry-cli` prints through the same catalogs: its usage text, `info` labels, option errors and
engine errors. It uses `--translation FILE` (a file that cannot be read is an error), else
`$LASERFOUNDRY_TRANSLATION`, else the desktop app's own file (`translation.json` in the LaserFoundry data
folder; a broken one is ignored). `laserfoundry-cli translation-template` prints a catalog with every
text the tool shows plus the engine messages, to start from. The shared code is the `laserfoundry-i18n`
crate.
