# What is not parsed yet

This file lists the parts of the NCZ format that `nczrs` reads as raw bytes, skips, or decodes without using them. The
DXF export also drops some data. The value statistics come from running the parser over the
`ncz/` corpus: 462 files, about 75 M objects. Files over 60 MB were left out of the field statistics.

To decode a field, draw test objects in NetCAD, export them to DXF, and run
`cargo run --release --bin ncz-match -- file.NCZ [file.dxf] [more files or directories]`. It pairs each geometry with
the entity NetCAD wrote for it and reports which NCZ fields predict which DXF group codes.

After changing the parser or the DXF export, run `cargo run --release --bin corpus-check`. It converts every file in
`ncz/`, checks the DXF output (handles, extended data, loading back) and compares the results with the baseline in
`target/corpus-baseline.json`. `--update` accepts the new results. Files over 60 MB are left out unless `--all` is
given, which needs `--jobs 1` on most machines (the largest file takes about 13 GB).

Every geometry record is read to its last byte. A raw scan found no trailing bytes for any geometry type, so the
remaining gaps are fields with unknown meaning, not unknown lengths.

## Recently resolved

These were confirmed against the corpus and are now parsed:

- Polyline area (f64).
- Text font index.
- Symbol rotation.
- Window rotation (probable).
- The f32 angle copies in arcs.
- Object 5 (`LinkedFile`, a `.NCO` path).
- Thumbnails, which are complete BMP files.
- Layer records, which were read one field out of step. A record is `flags (u32), name (21), color, 3 bytes`, and the 4
  bytes before the first record end in a u16 layer count (checked on all 425 readable files). The old
  "4 trailing bytes" were the last layer's color. Flag bit `0x01` is set when the layer is on.
- The 28-byte feature code of object 22 holds two strings: `code` (length byte + 10) and `link` (length byte + 16). The
  first length never exceeds 10 and the second never exceeds 16 (1.37 M records). `link` is set on 89% of them. It is
  mostly a database record reference (`@TAR453489`, `@PRG…`, `@SEM…`, `@TAB…`) and sometimes a free label such as a
  street name (`BULAK DERESİ`). It is exported as the XDATA key `LINK`.
- `MPROJ` (all 137 bytes): projection type, datum and central meridian, see §2.
- `LEX.ST2`: one 256-byte entry per layer, starting with the layer's true color, see §2.
- Window rotation: radians, counter-clockwise around `corner1`, with `corner2` the unrotated opposite corner. The DXF
  export now draws rotated windows (28 of 517 windows are rotated).

Some of these came from reading [Jeomatik NCZ Reader](https://github.com/erdincunal/Jeomatik-NCZ-Reader), an independent
Python reader. Its hints were checked against the corpus before use, and none of its code was copied.

Comparing NCZ files with DXF files that NetCAD 5 exported (`ncz-match`) also showed:

- NetCAD writes the NCZ `y` as DXF X and `x` as DXF Y; the NCZ stores northing first. Angles (arc start and end, text
  tilt) are in radians in the usual east-north frame and need no change. The DXF export swaps them, and
  `ncz-match` finds the same orientation in our output as in NetCAD's for all 5 sample pairs. Two checks over the
  whole corpus agree:
  - Coordinate ranges: 301 files are in a Turkish national grid, all with `x` as northing. No file was the other
    way round. The other files use local grids (e.g. Karabük ≈ 100,000 on both axes), so ranges cannot tell.
  - Text tilt against the nearest non-axis-parallel line: `x` as northing wins in 250 of 263 files with enough
    aligned texts, and loses in none. The other 13 are cadastral files with few aligned texts, and they lean the same way.
- NetCAD 5 writes DXF R12 in DOS code page 857 and turns Turkish letters in layer names into ASCII (`@EĞRİ` → `XEGRI`).

The DXF export, now written as R2000, also gained:

- Extents.
- The thumbnail.
- One text style per font.
- Italic (15° oblique) and underlined text.
- XDATA under the application `NCZ`, as key–value pairs: `NAME` for points and polylines, `ID` (a number, unique
  per file) and `AREA` (only when not zero) for polylines, and `CODE` and `LINK` for all geometries. NetCAD 5's own
  export writes no XDATA, nor any names, so this is data its export loses.
- The dxf crate 0.6.2 writes extended data after a polyline's vertices and `SEQEND`, where readers attach it to the
  `SEQEND`, and it has two handle bugs in blocks (see §2, `BlockBegin`). A patched copy is vendored in `vendor/dxf`
  (see `vendor/dxf/PATCHED.md`).

## 1. Objects with no decoder

| Id                     | Length   | Seen in  | Notes                                                                    |
|------------------------|----------|----------|--------------------------------------------------------------------------|
| 10                     | always 0 | 16 files | A marker with no body. Its position in the stream may say what it marks. |
| 226 `LayerInformation` | –        | 0 files  | Defined in `object.rs` and `ncz.pat` but never found in the corpus.      |
| 0, 52, 54, 65          | –        | 2 files  | Corrupt bytes (see §6), not real object types. Now skipped.              |

## 2. Objects kept as raw bytes

### `FileHeader` (id 1, 120 bytes)

None of it is decoded. The first bytes look like u32 fields, e.g.
`b2 8f 29 00 | e5 b1 01 00 | 7d ac 01 00 | bc 00 00 00 | ff ff ff ff | ff ff ff ff`.

- The first three values differ from file to file, so they are probably counts or file offsets. The index pages also use
  file offsets.
- `0xbc` or `0xbd` might be a format revision.
- The `-1` values look like the "no next/previous" sentinel that `IndexPage` uses.

### `UserInformation` (id 2, always 366 bytes)

Only the leading user name is read. The other ~350 bytes are ignored. They may hold a company, licence or date.

### `AreaBegin` (id 30) / `AreaEnd` (id 31)

These were read as `HatchBegin`/`HatchEnd`, but they are not hatches. They are an area record around polylines.
Some files wrap every polyline in one, and nothing is filled.

- In 115 of the 120 files under 60 MB that have them, every polyline is wrapped, open ones included (127k
  polylines). In the other 5, fewer than 3% are.
- NetCAD 5's DXF export writes nothing for them beyond the polyline. The fill lines inside block `VANTUZ` of
  `PLANKOTE` are ordinary Line geometries.
- The thumbnails, which NetCAD renders, show the wrapped polylines as outlines: the 4,528 parcels of `üniv-parsel nolu`
  and the large `ILAHIYAT` area of `22 _ilahiyat`.
- Exporting the polyline as an outline, as the DXF export does, therefore matches NetCAD.

The body is kept raw. It is 149 bytes for one polyline and 55 bytes longer for each further one (up to 14 or more,
probably holes). Offsets for one polyline:

| Offset  | Content                                                                                                 |
|---------|---------------------------------------------------------------------------------------------------------|
| 0       | `ff ff ff ff`                                                                                           |
| 4       | u32 `196 + 24 × n`, `n` the point count, probably a length                                              |
| 8       | `01`                                                                                                    |
| 9       | Name: length byte and 10 bytes, the polyline's name (a parcel number such as `0/02132`, or `X`)         |
| 20      | u32 `n`                                                                                                 |
| 24      | Bounding box, 4 f64 in NCZ axes                                                                         |
| 56      | Number of polylines. Multi-polyline records hold `0x10001`, `0x20001`, …, so the high half differs.     |
| 60      | Two points (4 f64). The first is the centroid. The second equals it or lies near it, maybe a label spot |
| 92      | `01` (93%), `ff` (5.7%) or `00` (1.5%, only around polylines of fewer than 3 points). Not a fill flag.  |
| 97      | u32 `n` again, then 12 zero bytes                                                                       |
| 113     | Bounding box again                                                                                      |
| 145     | u32 `n` again                                                                                           |

### `GeometryStyle` (id 33)

1.75 M in 86 files.

- Almost all of them style circles (geometry type 3). One styles a polyline, and six style contours.
- Bodies are 28 bytes, or 37 bytes in a few cases. They usually start `01 00 00 …`.
- The rare variants hold `ff ff ff` (a colour?) or a name such as `PLANTADİLA…`.
- **Not applied to the following geometry.**

### `Metadata` (id 28)

These entries stay `MetadataContent::Binary`:

| Name      | Files |
|-----------|-------|
| `MAPP`    | 337   |
| `MACROPU` | 213   |
| `MPROJ2`  | 28    |
| `LEX.STE` | 2     |
| `LEX.ST8` | 1     |
| `""`      | 1     |

`MPROJ` is decoded as `MetadataContent::Projection`. The 138 files where it used to decode as text held only zero bytes.

- Byte 0 is the projection: 1 geographic, 2 for 6° zones, 3 for 3° zones, 0 undefined.
- Byte 1 is the datum: 0 WGS-84, 1 ITRF, 4 ED50, 254 ED50 (HGK). 255 and 83 also occur. Datum 0 is uncertain:
  several of the 22 files with `(3, 0, 33)` are named as ITRF (`ALINCA_ITRF`, `AHMETLER_ITRF_…`).
- Byte 5 is the central meridian in degrees (33 or 39 in the corpus).
- Corpus (files under 60 MB): `(3, 1, 33)` ×222, `(0, 0, 0)` ×127, `(3, 0, 33)` ×22, `(3, 4, 33)` ×21,
  `(0, 255, 39)` ×5, `(0, 1, 0)` ×3, and single files with datum 255 or 83. 22 files have no `MPROJ`. No file under
  60 MB has a `TILED_XML` entry to check the codes against.
- The web summary names the coordinate system with its EPSG code: TUREF / TM27–TM45 (EPSG:5253–5259) for ITRF and
  ED50 / TM27–TM45 (EPSG:2319–2325) for ED50 3° zones, the WGS 84 and ED50 UTM zones for 6° zones, and EPSG:4326,
  5252 or 4230 for geographic coordinates. Datum 0 and the unknown datums get a description without a code. DXF
  has no standard place for it, so the DXF export does not write it.
- Bytes 2–4 and 6–136 are unknown. Bytes 12 onwards often hold f64-like values, maybe false easting or scale.

`LEX.ST2` is decoded as `MetadataContent::LayerStyles`.

- Byte 4 equals the low byte of the layer count.
- 256-byte entries start at byte 63, one per layer in layer table order.
- After the entries come 264, 8 or 65,800 more bytes.
- An entry starts with 3 color bytes and a zero. Each layer color index maps to one fixed value across all 406 files
  that have both: 1 → `00 00 ff`, 2 → `00 ff 00`, 3 → `00 ff ff`, 4 → `ff 00 00`, 14 → `ff ff 00`, 15 → `ff ff ff`. User
  palettes change it, e.g. 102 is `9a cf 19` in one file and `80 ff 00` in another.
- NetCAD's DXF writes the layer index unchanged as group 62, so the DXF export keeps the index.
- If the bytes are R, G, B (as Jeomatik reads them), NetCAD's palette is not ACI: 1 is blue and 4 is red. This is not
  confirmed (it could be B, G, R). Once it is, it could be exported as true color (group 420).
- The other 252 bytes of each entry are unknown.

`TILED_XML` holds an `SRS="EPSG:nnnn"` attribute, which gives the EPSG code directly.

The text entries are only decoded as strings, not interpreted: `PRJPAR`, `REF`, `CONMAN`, `GISWIZ`, `P_MAP_PROJ`,
`GEOPAR`, `CAPMAN`, `TILED_XML`, `VARIABLES`, `KmPrjProp`, …. `PRJPAR`, `P_MAP_PROJ` and `GEOPAR` probably hold the
coordinate system (datum, projection, zone). Those would be worth parsing for georeferencing.

`SYMBOLS` is parsed, but `unknown` (u32) and 26 bytes per library are still unknown.

## 3. Unknown fields inside parsed objects

### `Layers` (id 6)

- `unknown1`: 9 leading bytes, each 0 or 1. They look like boolean settings.
- `unknown2` and `unknown3`: two bytes before the u16 layer count, mostly `1, 1` or `1, 4`.
- For each layer:
    - `flags` is a bitmask. Bit `0x01` is on/off (confirmed on 303 layers against NetCAD's DXF). The other bits, e.g. in
      `0xFFFFFEFD` and `0x200FD`, are unknown. Lock or freeze would not show in a DXF R12 export.
    - The three bytes after the color are mostly `0, 1, 0`.

### `LineTypes` (id 8)

- `offset` and `unknown1` (u16) are not used. `unknown1` is non-zero mostly when `segment_count` is negative, and the
  meaning of a negative count is unknown.
- `unknown2` holds 3 trailing bytes per line type.
- Segments at or above 1000 seem to refer to symbols. The DXF export drops them.

### `RasterReferences` (id 26)

Only the `.DRE` path is read. The other 100 bytes of each entry are unknown. They probably hold the image's placement,
scale or rotation.

### `BlockBegin` (id 15)

Only the name is read. No block base point has been found, so DXF blocks are inserted with origin (0,0).

The dxf crate 0.6.2 left the vertices and `SEQEND` of a polyline in a block without handles, so ezdxf could not
open such files (2 of the 5 sample exports), and it wrote a block's handle on both `BLOCK` and `ENDBLK`. Both are
fixed in the vendored copy (`vendor/dxf/PATCHED.md`), as is a third bug: table heads (`VPORT`, `LTYPE`, …) had no
handles. The 5 sample exports pass ezdxf's audit without errors, fixes or warnings.

## 4. Unknown fields in geometries

### Shared header

| Field                      | Observed                                                                 | Guess                                                        |
|----------------------------|--------------------------------------------------------------------------|--------------------------------------------------------------|
| `unknown1`                 | 0 in all v4/v5/v6/v7 files. In v8 files: 2, 5, 7, 9, …                   | A copy of the geometry type in v8.                           |
| `line_type2`               | Equals `line_type`. Bit `0x80` is set on 2.3% of geometries.             | A flag, unknown which.                                       |
| `attributes[0]`            | 0, or 1 on 0.05%                                                         | Flag                                                         |
| `attributes[1]`            | Mostly 0, ~30 distinct values (55, 70, 65, …)                            | Maybe line width or pen                                      |
| `attributes[2]`            | Always 0                                                                 | Padding                                                      |
| `attributes[3]`            | **Colour (parsed)**                                                      | –                                                            |
| `attributes[4..12]`        | Almost always 0. The rare non-zero values repeat (`33 33 …`, `99 99 …`). | Perhaps an f64 such as thickness or width                    |
| `attributes[12..16]`       | f32. 1.0 is common (`00 00 80 3f`).                                      | Read as `spacing` for text only, unknown for the other types |
| `unknown2`                 | **Always 0**                                                             | Padding                                                      |
| `code`, `link` (object 22) | See "Recently resolved"                                                  | Exported as XDATA                                            |

### Point (1)

- `unknown3` (12 bytes) looks like a padded string: a length byte and up to 11 characters, e.g. `0.0049`
  next to the code `0.0065`, or `000000`. On GNSS points it may be a precision. Some files have
  `00 ff ff ff 00 00 00 00 <u32>` and `00 51 1b c6 00 51 00 00 …`, which may be stale padding. NetCAD 5 does not export
  it, so this is not confirmed.

### Arc (4)

`unknown3` holds 2 bytes that follow the f32 copies of the angles.

### Text (5)

- `flags` beyond the italic, underline and highlight bits are not checked.
- `highlight` and `spacing` are **not exported**.
- Fonts are exported as text styles that use `<font>.ttf`, which is a guess. NetCAD's own fonts such as `F1` have no
  TrueType counterpart, so CAD programs substitute another font.
- NetCAD 5 exports `snap` as these DXF alignments (72 horizontal, 73 vertical), from 4,082 texts:

  | `snap` | Alignment | | `snap` | Alignment |
      |---|---|---|---|---|
  | `L` | left, baseline | | `1` | left, middle |
  | `C` | centre, bottom | | `2` | left, top |
  | `M` | centre, middle | | `3` | centre, top |
  | `R` | right, bottom | | `4` | right, top |
  | `6` | left, baseline | | `5` | right, middle |
  | `7` | centre, baseline | | | |

  The DXF export follows this table, and matches NetCAD's alignment on all 2,680 texts of the 5 sample pairs that
  could be matched by position. The letters `P U Q E H A O` did not occur in the exported files and fall back to
  centre/middle:

| Value | Count                       |
|-------|-----------------------------|
| `M`   | 409k, the most common value |
| `P`   | 53k                         |
| `U`   | 1.6k                        |
| `Q`   | 1.4k                        |
| `\0`  | 274                         |
| `E`   | 64                          |
| `H`   | 63                          |
| `A`   | 51                          |
| `O`   | 45                          |

### Symbol (6)

The shapes are in NetCAD's symbol libraries (`F2.NCS`, `PLAN.NCS`, `3D.NCS`, listed in `Libraries` and `SYMBOLS`),
which are not part of the NCZ file. NetCAD 5 exports each symbol as loose LINE, CIRCLE and ARC entities.

- Position, scale and rotation (radians, counter-clockwise) behave exactly like a DXF insert of the symbol drawn in
  its own frame. Mapped back into that frame, NetCAD's drawing of every instance of a symbol is the same shape, to
  0.01 (4 rotated instances of symbol 1 and 37 of symbol 240 in `29B01D3A`).
- The header bounds hold the drawn extent in world coordinates.
- Scale 0 (13% of the 18.5k symbols in files under 60 MB, mostly symbol 0) comes with empty or zero-size bounds, so
  nothing is drawn.
- `unknown4` takes the values 0, 1 or 2.

The DXF export writes each symbol as an INSERT of a block `NCZ_SYMBOL_<n>`, one per symbol number. The block holds a
rectangle around the symbol's footprint, drawn by block, so a user who has the real symbols can redefine the block
and every instance updates. The footprint is measured on the instance turned closest to a right angle. On the
sample pairs, 140 of 144 exploded inserts match NetCAD's bounds exactly, and 4 (symbol 1, turned 0.1 rad off a right
angle) are larger. Symbols with scale 0 are not exported.

### PolyLine (7)

- `flags`: 71 distinct values. Common ones are 19 (`0x13`), 0, 1, 3, 17 and 18.
    - `0x01` (closed), `0x04` and `0x40` are used.
    - `ncz.pat` names 4 as Curve, 16 as PolyLine and 64 as SmoothCurve. They are rare: in files under 60 MB, 387 of
      345k polylines are curves (14 files) and 10 are smooth curves (1 file).
    - The points of a curve are digitized along it, typically 4–9 m apart and turning 5–30° at each point, so they lie
      on the curve rather than tracing it densely.
    - NetCAD 5 leaves curves out of its DXF export (all 6 in `üniv-parsel nolu` are missing), so its own curve is
      not known. The DXF export draws both kinds as a centripetal Catmull-Rom spline through the points, with 8
      segments per span and linear elevations. It keeps every stored point and stays within 0.25 m of the straight
      segments on the samples. A closed curve with right-angle corners, such as in block `MEMBA` of `PLANKOTE`,
      becomes a rounded outline. **Unconfirmed:** NetCAD may use another spline, and SmoothCurve may approximate the
      points instead of passing through them.
    - `0x02` is unknown.
- `name`, `id` and `area` are exported as XDATA. `id` numbers the polylines of a file uniquely. `area` is zero on
  most polylines. On parcels it is within about 1% of the area of the points, and round values such as `7437.00`
  suggest the registered area rather than a computed one. NetCAD 5 exports none of the three.
- **Widths are not found.** NetCAD 5 writes a constant width (groups 40/41) on some polylines, which the DXF export
  does not:
  - In `üniv-parsel nolu`, all 95 polylines of layer `IMAR_18UYGULAMA` get 0.5, and no other polyline gets a width.
  - In `22 _ilahiyat`, three polylines in blocks get 0.15, 0.15 and 0.5.

  Nothing stored in these files tells them apart:
  - Each `IMAR_18UYGULAMA` parcel has a twin with the same points on another layer (e.g. `MAHALLE_100YILKESIN`),
    which gets no width. The two records match in every field, the header attributes and the area record (§2)
    included. Only `id`, the layer and the low bits of the area record's centroid differ.
  - The two layers' records match except for name and color, and so do their `LEX.ST2` entries. The layer name
    occurs nowhere else in the file.
  - Color does not decide it: `ADA_ALN` is color 9 like `IMAR_18UYGULAMA`, and its polylines get no width. In
    `22 _ilahiyat`, the polyline with width 0.5 shares color, line type and layer color with 226 polylines without one.
  - `attributes[4..12]` (see above) is zero on all of them.

  The width probably comes from NetCAD's settings rather than the file, e.g. a pen table or layers named `IMAR_*`.
  Deciding it needs NetCAD: how `IMAR_18UYGULAMA` gets its thickness, or a test drawing with a polyline given a width.

### Contour (9)

- `unknown3` holds 36 bytes.
- `unknown3[0]` counts up from contour to contour in the one file checked (0, 1, 2, …), so it may be an index.
- `point_flags` (0 or 1) mark gaps, probably where the elevation label goes. NetCAD 5 exports one polyline per
  drawn run. Flagged points at the start of a contour are not drawn at all. Anywhere else, only a segment between two
  flagged points is hidden, so a run ends at the first flagged point of a gap and resumes at the last one. The DXF
  export follows this rule, and all 81 contour polylines of `29B01D3A` match NetCAD's vertex for vertex. Only this
  one file has been checked. That file has 7 fully flagged contours, which NetCAD also leaves out.
  Across the corpus (434k contours in 78 files), flags are only 0 or 1.
  - 36k contours (8%) are fully flagged, so they are not exported. Most are in the large city plans
    (`KARABUK_İMAR_2015` 6,935, `ADN_den karabük` 5,568).
  - 7,703 interior gaps are a single flagged point, and the rule draws straight through them. No sample DXF has one,
    so this is unconfirmed.

### Block insert (13)

`id` and `unknown3` are not decoded. `unknown3` is mostly 1, sometimes 343, 3, 0, ….

### Image (15)

Not exported. There are 8 in 2 files.

- Jeomatik calls type 15 a "SmartObject", and images are one class of it. Their files also hold a `BASIC`
  grid/table class with width, height and grid spacing f64s.
- They read the shared header's `unknown2` as a rotation in grads for this type. Our 8 samples cannot confirm it.
- Symbols with code 0 on layer 0 drawn next to such objects are its internal grid markers.
- `unknown3` (24 bytes) and `unknown4` are unknown, as are the 7 bytes after each property.
- Property keys: `imageContent`, `imageWidth`, `imageHeight`, `width`/`genislik`, `height`, `imageSaveType`,
  `imageCapturePoint`, `backgroundColor`, `rtfData`.
- `imageContent` could be decoded and written as a DXF `IMAGE` entity plus a file. `rtfData` could become `MTEXT`.

### Window (10)

`rotation` is in radians, counter-clockwise around `corner1` (see "Recently resolved"). This comes from Jeomatik's
reader and has not been checked against a NetCAD DXF.

### Empty (14)

694 records with no body. It is not known what they mean.

### Geometry type 8

It never appears in the corpus. Any type that does not match falls back to `Geometry::Other`, but none occurred.

## 5. Parsed but not used by the DXF export

- `Libraries`, `RasterReferences`, `LinkedFile`, `Metadata` and `UserInformation`.
- `Image`: `imageContent` is a base64 PNG and `rtfData` is base64 RTF. A DXF `IMAGE` needs the PNG as a separate file
  next to the drawing, which does not fit the web app's single-file download.
- `IndexGroup`/`IndexPage`: this is a spatial index that can be derived from the geometry, so nothing needs to be
  exported.

## 6. Robustness issues found in the corpus

- Every readable file starts with a `FileHeader` (id 1, length 120). Files that do not, such as **`224-2.NCZ`** (all
  zero bytes, 1.6 MB), are rejected as not NCZ.
- An object whose decoder fails, or whose type no file uses, marks corruption. Across the corpus the only such type
  in healthy files is 10, the empty marker. `parse_objects` lists each one as a failure (offset, type id, declared
  length and bytes skipped). It then resumes where the next 8 objects in a row decode, trying the end of the
  object's declared length first and then every following byte. A cut-off object at the end of a file is skipped
  to the end. The CLI prints failures as warnings and the web app shows their count.
- **`KARABUK_tüm-2012.NCZ`** (738 MB) is corrupt at offset `0xC951038`.
    - At that offset a Line record (object 22) is cut off in the middle of its bounds.
    - 1,266 bytes from a sorted string table follow: `XT`, `XTB`, `XY`, `Y_BINA`, `YAPI`, `Yerleştirilmiş Nesne`, ….
      The entries look like u32 length, name, `00 01 00 00`, u32 id. This looks like data that was overwritten, e.g.
      on disk.
    - Without a resync, the remaining ~527 MB came out as 21 garbage objects (ids 0, 52, 54, 65) with huge lengths.
      With it, parsing resumes after the 1,266 bytes and reads the other 3.6 M objects (2.9 M points, 639 k lines,
      33 k polylines, the spatial index) to the end of the file, in the same coordinate range as before the gap. This
      is the only failure in the corpus.
