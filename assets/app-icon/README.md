# Niveta app icon

An **N laid over stacked sheets**: a bold angular N whose right leg runs into layered bands, the
mark for an editor that stacks layers. Ready-made from the owner's `newlogo.jpg` drop-in (2048 px);
the committed SVG is a trace of that artwork.

## Palette

Two colours:

| Colour | Hex | Used for |
|---|---|---|
| Ink | `#14151b` | the full-bleed field |
| Niveta blue | `#378df0` | the mark |

## Geometry

A 512-unit tile (`viewBox="0 0 512 512"`), rounded square with `rx=112`, no border. The mark keeps
the source's margins (roughly 85 units on each side). The macOS renders pad it onto Apple's
824/1024 icon grid; the Windows and Linux renders crop 22 units off each side so the mark reads at
16–48 px.

## Provenance

The owner's `newlogo.jpg` artwork, traced to `niveta.svg` with potrace (flat fills, polygons
smoothed to beziers) and coloured to the palette above. License: see `LICENSE.txt`.

## Files

- `niveta.svg`: canonical master.
- `niveta-small.svg`: lighter trace (from a 512 px mask); also the hicolor scalable icon.
- `niveta-1024.png`: 1024 px render on the macOS grid.
- `niveta.icns`: macOS bundle icon (`CFBundleIconFile`).
- `niveta.ico`: Windows icon, 16–256 px, embedded in the `.exe` by `apps/photocraft/build.rs`.
- `hicolor/<size>/apps/ai.storyteller.photocraft.png` (16–512) and `hicolor/scalable/...svg`:
  Linux icon theme.

The app also sets the window icon and Wayland app ID at runtime (`apps/photocraft/src/app_icon.rs`,
`main.rs`). On Windows the taskbar shows the icon of the Start Menu shortcut that launches the
exe, so the MSI's `<Icon Id>` keeps the `.exe` extension (ICE50); `app_icon.rs` documents the
details and tests the `.ico` sizes and the WiX icon references.

## Regenerate

Replace `niveta.svg` (and `niveta-small.svg`), then run `packaging/icons.sh`. It needs
`resvg`, plus `iconutil` on macOS; the `.ico` is packed by `cargo xtask ico`.
