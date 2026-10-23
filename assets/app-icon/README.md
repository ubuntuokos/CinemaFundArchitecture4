# CFA4 official logo

The approved CFA4 emblem is the canonical brand artwork for this project. It adapts the CFA3
film-and-architecture mark and reads **CFA4** / **CINEMA FUND ARCHITECTURA 4**.

## Source and outputs

- `cfa4-official-logo.png` is the approved 1254 × 1254 px master image.
- `filmcraft.svg` embeds a 1024 px copy of the approved logo. It is the canonical input to
  `packaging/icons.sh`.
- `filmcraft-small.svg` embeds a 512 px copy for the Linux scalable icon.
- The PNG, ICO and ICNS assets are rendered from the approved master with no recolouring or
  redesign. macOS exports retain the existing transparent inset.
- The FilmCraft web favicon uses the same artwork.

Every exported asset has an attribution sidecar and an entry in `ATTRIBUTION.md`.

## Regenerate

Run `packaging/icons.sh` with `resvg` installed; macOS `.icns` can be refreshed with
`iconutil`, and Windows `.ico` is assembled by `cargo xtask ico`.
