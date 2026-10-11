# What is not parsed yet

This file lists the parts of the NCZ format that `nczrs` reads as raw bytes, skips, or decodes without using
them. The DXF export also drops some data. The value statistics come from running the parser over the
`ncz/` corpus: 462 files, about 75 M objects. Files over 60 MB were left out of the field statistics.

Every geometry record is read to its last byte. A raw scan found no trailing bytes for any geometry type, so
the remaining gaps are fields with unknown meaning, not unknown lengths.

## Recently resolved
These were confirmed against the corpus and are now parsed:
- Polyline area (f64).
- Text font index.
- Symbol rotation.
- Window rotation (probable).
- The f32 angle copies in arcs.
- Object 5 (`LinkedFile`, a `.NCO` path).
- Thumbnails, which are complete BMP files.

The DXF export, now written as R2000, also gained:
- Extents.
- The thumbnail.
- One text style per font.
- Italic (15° oblique) and underlined text.
- Point names and feature codes as XDATA under the application `NCZ`, as `NAME`/`CODE` key–value strings.

## 1. Objects with no decoder

| Id | Length | Seen in | Notes |
|----|--------|---------|-------|
| 10 | always 0 | 16 files | A marker with no body. Its position in the stream may say what it marks. |
| 226 `LayerInformation` | – | 0 files | Defined in `object.rs` and `ncz.pat` but never found in the corpus. |
| 0, 52, 54, 65 | – | 2 files | Caused by corrupt files (see §6), not real object types. |

## 2. Objects kept as raw bytes

### `FileHeader` (id 1, 120 bytes)
None of it is decoded. The first bytes look like u32 fields, e.g. `b2 8f 29 00 | e5 b1 01 00 | 7d ac 01 00 | bc 00 00 00 | ff ff ff ff | ff ff ff ff`.
- The first three values differ from file to file, so they are probably counts or file offsets. The index pages also use file offsets.
- `0xbc` or `0xbd` might be a format revision.
- The `-1` values look like the "no next/previous" sentinel that `IndexPage` uses.

### `UserInformation` (id 2, always 366 bytes)
Only the leading user name is read. The other ~350 bytes are ignored. They may hold a company, licence or date.

### `HatchBegin` (id 30) / `HatchEnd` (id 31)
605,791 hatches in 135 files.
- The body is kept raw, usually 149 bytes, with longer ones growing in steps of 55 bytes. That suggests one fixed part plus 55-byte repeated entries, perhaps pattern lines.
- It starts with `ff ff ff ff`, then a u32 such as `0x13c`, `0x154` or `0x16c`, then `01 05`/`01 06`, then ASCII such as `0/`.
- **The DXF export ignores hatches.** The polyline they wrap is exported as a plain outline, so the fill is lost.

### `GeometryStyle` (id 33)
1.75 M in 86 files.
- Almost all of them style circles (geometry type 3). One styles a polyline, and six style contours.
- Bodies are 28 bytes, or 37 bytes in a few cases. They usually start `01 00 00 …`.
- The rare variants hold `ff ff ff` (a colour?) or a name such as `PLANTADİLA…`.
- **Not applied to the following geometry.**

### `Metadata` (id 28)
These entries stay `MetadataContent::Binary`:

| Name | Files |
|------|-------|
| `LEX.ST2` | 444 |
| `MAPP` | 337 |
| `MPROJ` | 299 (also text in 138) |
| `MACROPU` | 213 |
| `MPROJ2` | 28 |
| `LEX.STE` | 2 |
| `LEX.ST8` | 1 |
| `""` | 1 |

The text entries are only decoded as strings, not interpreted: `PRJPAR`, `REF`, `CONMAN`, `GISWIZ`, `P_MAP_PROJ`, `GEOPAR`, `CAPMAN`, `TILED_XML`, `VARIABLES`, `KmPrjProp`, …. `PRJPAR`, `P_MAP_PROJ` and `GEOPAR` probably hold the coordinate system (datum, projection, zone). Those would be worth parsing for georeferencing.

`SYMBOLS` is parsed, but `unknown` (u32) and 26 bytes per library are still unknown.

## 3. Unknown fields inside parsed objects

### `Layers` (id 6)
- `unknown1`: 9 leading bytes, each 0 or 1. They look like boolean settings.
- `unknown2`: **4 trailing bytes, present in every file**, shaped `[n, 0, 1, 0]` with `n` from 0 to about 15. This could be the current layer index (u16) followed by a u16 flag.
- For each layer:
  - `unknown1` is almost always 0.
  - `unknown2` is usually 1, sometimes 2.
  - `unknown3` is 0 or 1.
  - `flags` is a bitmask. Common values are `0xFFFFFEFD`, `0xFD`, `0xFFFFFFFD`, `0xFFFFFEFC` and `0x200FD`. Visibility, lock or freeze state is likely in here and is not mapped to DXF layer flags.

### `LineTypes` (id 8)
- `offset` and `unknown1` (u16) are not used. `unknown1` is non-zero mostly when `segment_count` is negative, and the meaning of a negative count is unknown.
- `unknown2` holds 3 trailing bytes per line type.
- Segments at or above 1000 seem to refer to symbols. The DXF export drops them.

### `RasterReferences` (id 26)
Only the `.DRE` path is read. The other 100 bytes of each entry are unknown. They probably hold the image's placement, scale or rotation.

### `BlockBegin` (id 15)
Only the name is read. No block base point has been found, so DXF blocks are inserted with origin (0,0).

## 4. Unknown fields in geometries

### Shared header
| Field | Observed | Guess |
|-------|----------|-------|
| `unknown1` | 0 in all v4/v5/v6/v7 files. In v8 files: 2, 5, 7, 9, … | A copy of the geometry type in v8. |
| `line_type2` | Equals `line_type`. Bit `0x80` is set on 2.3% of geometries. | A flag, unknown which. |
| `attributes[0]` | 0, or 1 on 0.05% | Flag |
| `attributes[1]` | Mostly 0, ~30 distinct values (55, 70, 65, …) | Maybe line width or pen |
| `attributes[2]` | Always 0 | Padding |
| `attributes[3]` | **Colour (parsed)** | – |
| `attributes[4..12]` | Almost always 0. The rare non-zero values repeat (`33 33 …`, `99 99 …`). | Perhaps an f64 such as thickness or width |
| `attributes[12..16]` | f32. 1.0 is common (`00 00 80 3f`). | Read as `spacing` for text only, unknown for the other types |
| `unknown2` | **Always 0** | Padding |
| `code` (object 22) | Present on ~6% of geometries | Exported as XDATA |

### Point (1)
- `unknown3` (12 bytes) is mostly zero. Variants include `00 ff ff ff 00 00 00 00 <u32>` and `00 51 1b c6 00 51 00 00 …`, which may be a colour followed by an id.

### Arc (4)
`unknown3` holds 2 bytes that follow the f32 copies of the angles.

### Text (5)
- `flags` beyond the italic, underline and highlight bits are not checked.
- `highlight` and `spacing` are **not exported**.
- Fonts are exported as text styles that use `<font>.ttf`, which is a guess. NetCAD's own fonts such as `F1` have no TrueType counterpart, so CAD programs substitute another font.
- `snap` values have no mapping in `text_justification`, so they fall back to Center/Middle:

| Value | Count |
|-------|-------|
| `M` | 409k, the most common value |
| `P` | 53k |
| `U` | 1.6k |
| `Q` | 1.4k |
| `\0` | 274 |
| `E` | 64 |
| `H` | 63 |
| `A` | 51 |
| `O` | 45 |

  The mapped letters `L`, `C` and `R` are guesses.

### Symbol (6)
Not exported. 472k symbols in 398 files are dropped.
- The symbol definitions are in external libraries (`.NCI` files and `Libraries`), which are not parsed.
- `unknown4` takes the values 0, 1 or 2.

### PolyLine (7)
- `flags`: 71 distinct values. Common ones are 19 (`0x13`), 0, 1, 3, 17 and 18.
  - Only `0x01` (closed) is used.
  - `ncz.pat` names 4 as Curve, 16 as PolyLine and 64 as SmoothCurve. **Curves and smooth curves are exported as straight segments.**
  - `0x02` is unknown.
- `name`, `id` and `area` are not exported.

### Contour (9)
- `unknown3` holds 36 bytes.
- `point_flags` are 0 or 1 (26% are 1) and are ignored. A value of 1 may mark a break or pen-up, or a smoothing node. Today the whole contour is exported as one continuous polyline.

### Block insert (13)
`id` and `unknown3` are not decoded. `unknown3` is mostly 1, sometimes 343, 3, 0, ….

### Image (15)
Not exported. There are 8 in 2 files.
- `unknown3` (24 bytes) and `unknown4` are unknown, as are the 7 bytes after each property.
- Property keys: `imageContent`, `imageWidth`, `imageHeight`, `width`/`genislik`, `height`, `imageSaveType`, `imageCapturePoint`, `backgroundColor`, `rtfData`.
- `imageContent` could be decoded and written as a DXF `IMAGE` entity plus a file. `rtfData` could become `MTEXT`.

### Window (10)
`rotation` is a small f32 in every sample, so it is probably an angle, but this is not confirmed. The export draws
windows unrotated.

### Empty (14)
694 records with no body. It is not known what they mean.

### Geometry type 8
It never appears in the corpus. Any type that does not match falls back to `Geometry::Other`, but none occurred.

## 5. Parsed but not used by the DXF export
- `Libraries`, `RasterReferences`, `LinkedFile`, `Metadata` and `UserInformation`.
- `Image`: `imageContent` is a base64 PNG and `rtfData` is base64 RTF. A DXF `IMAGE` needs the PNG as a separate file
  next to the drawing, which does not fit the web app's single-file download.
- `IndexGroup`/`IndexPage`: this is a spatial index that can be derived from the geometry, so nothing needs to be exported.

## 6. Robustness issues found in the corpus
- **`224-2.NCZ`** is all zero bytes (1.6 MB). The parser turns it into 331k empty `Unknown { id: 0 }` objects. It should be detected and rejected.
- **`KARABUK_tüm-2012.NCZ`** (738 MB) breaks at offset `0xC951038`.
  - At that offset a Line record (object 22) is cut off in the middle of its bounds.
  - Bytes from a sorted string table follow: `XT`, `XTB`, `XY`, `Y_BINA`, `YAPI`, `Yerleştirilmiş Nesne`, …. The entries look like u32 length, name, `00 01 00 00`, u32 id. This looks like data that was overwritten, e.g. on disk.
  - The parser never gets back in sync, so the remaining ~527 MB come out as garbage objects (ids 0, 52, 54, 65).
  - The fix is a resync step: scan ahead for a valid `type, length` pair.
- `parse_objects` drops an object without any warning when its decoder returns `None`. No list of failures is kept for diagnostics.
- `ncz/.gitignore` is read like an NCZ file when the whole directory is processed.
