# Attribution

Every file under `assets/` needs a row here (path, author, source, licence); `cargo xtask assets`
fails otherwise. This repository currently ships **no assets**: toolbar and canvas art are drawn in
code, and default layer colours are our own palette (`crates/doc/src/lib.rs`).

| Path | Author | Source | Licence | Notes |
|------|--------|--------|---------|-------|
| (none) |      |        |         |       |

## Screenshots

`apps/laserfoundry-web/site/features/img/*.png` and `docs/screenshots/*.png` are screenshots of LaserFoundry itself, taken from
the running app on designs made for them (shapes, our own single-line font, our own palette). They contain no third-party
artwork, fonts or files. They are not under `assets/`, so `cargo xtask assets` does not list them.

