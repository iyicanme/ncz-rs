# Symbols

The shapes of the symbols that NCZ files refer to. NCZ files store only a symbol's library, number, position, scale
and rotation. `nczrs` embeds these files and draws each symbol as a DXF block with the shape.

Each symbol is a file `<LIBRARY>/<number>.txt`, where `LIBRARY` is the name of the library an NCZ file lists, without
its extension and in upper case (`F2.NCS` → `F2`), and `number` is the symbol number (0–255).

## Format

A line per shape, numbers separated by spaces. Empty lines and lines starting with `#` are ignored.

```
# A circle with a line through its centre
circle 0 0 0.5
line -1 0 1 0
```

| Shape                     | Meaning                                                  |
|---------------------------|----------------------------------------------------------|
| `line x1 y1 x2 y2`        | A line from `(x1, y1)` to `(x2, y2)`.                    |
| `circle x y radius`       | A circle around `(x, y)`.                                |

Coordinates are in drawing units for a symbol with scale 1, relative to its insertion point, with `x` to the east and
`y` to the north. A symbol with scale `s` and rotation `r` is drawn scaled by `s` and turned by `r` counter-clockwise
around the insertion point, exactly like a DXF insert of the shape.

## Adding symbols

Write a file by hand, or convert a NetCAD library you have the rights to with
`cargo run --release --bin ncs-port -- symbols path/to/LIBRARY.NCS`. Rebuild afterwards, the files are embedded at
build time.
