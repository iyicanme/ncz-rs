// ImHex pattern for NetCAD NCZ drawings (NetCAD 4 to 8).
//
// Everything here was worked out from files, not from documentation, so field names starting with `unknown` are just
// that. Fields of known meaning carry a comment; the converter in this repository (src/object.rs, src/geometry.rs,
// src/line_type.rs) reads the same layout.
//
// General rules:
// - Little-endian throughout.
// - A file is a flat sequence of objects: u8 type, u32 body length, body. The first object is always a FileHeader
//   (type 1, length 120). There is no table of contents; the index pages (types 27, 29) are an optional spatial index.
// - One exception to the length rule: an Image geometry (object 21 whose geometry type, body byte 1, is 15) declares a
//   length without its 81-byte geometry header, which follows the declared body.
// - Strings are Windows-1254 (Turkish). Most are a u8 length followed by a fixed-size field ("padded strings", see
//   PaddedString); stale bytes after the text are common, and a few length bytes exceed the field.
// - Coordinates are f64 metres in the drawing's projected system. NCZ x is northing and y easting, the reverse of the
//   usual CAD order. Elevations are mostly f32. Angles are radians, counter-clockwise.
// - Order in a typical file: FileHeader, Version, UserInformation, Scale, Extents, LinkedFile, Layers, LineTypes,
//   Fonts, Libraries, Metadata entries, Thumbnail, blocks (13, 15 … 16, 23), then the geometries, interleaved with
//   index pages and area records.

#pragma description NetCAD NCZ drawing
#pragma endian little
// Real drawings hold up to millions of objects; raise these further for large files.
#pragma array_limit 4000000
#pragma pattern_limit 64000000

import std.mem;

// ---------------------------------------------------------------------------------------------------------------------
// Helpers

// Size of the shared geometry header without the code/link strings of object 22.
// GEOMETRY_HEADER_LENGTH = 81

// Offset just past the object starting at `start`, including the Image exception.
fn object_end(u128 start) {
    u8 object_type = std::mem::read_unsigned(start, 1);
    u32 length = std::mem::read_unsigned(start + 1, 4, std::mem::Endian::Little);
    if (object_type == 21 && length >= 2) {
        u8 geometry_type = std::mem::read_unsigned(start + 6, 1);
        if (geometry_type == 15)
            return start + 5 + length + 81;
    }
    return start + 5 + length;
};

fn varint_size(u128 address) {
    u128 size = 1;
    while ((std::mem::read_unsigned(address + size - 1, 1) & 0x80) != 0)
        size = size + 1;
    return size;
};

fn varint_value(u128 address) {
    u128 value = 0;
    u128 shift = 0;
    u128 i = 0;
    while (true) {
        u8 byte = std::mem::read_unsigned(address + i, 1);
        value = value | ((byte & 0x7F) << shift);
        shift = shift + 7;
        i = i + 1;
        if ((byte & 0x80) == 0)
            break;
    }
    return value;
};

fn format_text(auto s) {
    return s.text;
};

// u8 length + text.
struct String {
    u8 length;
    char text[length];
} [[format("format_text")]];

// u8 length + an N-byte field holding the text, so 1 + N bytes. The length is clamped to the field.
struct PaddedString<auto N> {
    u8 length;
    char text[length < N ? length : N];
    u8 unused[N - (length < N ? length : N)];
} [[format("format_text")]];

// LEB128-style length (7 bits per byte, low bits first, high bit set on all but the last byte) + UTF-8 text.
// Only used in Image properties.
struct VarintString {
    u8 length[varint_size($)];
    char text[varint_value(addressof(length))];
} [[format("format_text")]];

struct Coordinate2D {
    double x; // northing
    double y; // easting
};

struct Coordinate3D {
    double x;
    double y;
    double z;
};

// A 2D coordinate with an f32 elevation.
struct Vertex {
    double x;
    double y;
    float z;
};

// ---------------------------------------------------------------------------------------------------------------------
// Enums

enum ObjectType : u8 {
    FileHeader = 1,
    UserInformation = 2,
    Scale = 3,
    Extents = 4,
    LinkedFile = 5,
    Layers = 6,
    LineTypes = 8,
    Marker = 10,            // always empty, meaning unknown
    Fonts = 11,
    Libraries = 12,
    BlockSectionBegin = 13,
    Thumbnail = 14,
    BlockBegin = 15,
    BlockEnd = 16,
    Geometry = 21,
    GeometryWithCode = 22,  // a geometry whose header also holds a feature code and a link
    BlockSectionEnd = 23,
    Version = 25,
    RasterReferences = 26,
    IndexPage = 27,
    Metadata = 28,
    IndexGroup = 29,
    AreaBegin = 30,
    AreaEnd = 31,
    GeometryStyle = 33,
    LayerInformation = 226, // known from older readers, never seen in real files
};

enum GeometryType : u8 {
    Point = 1,
    Line = 2,
    Circle = 3,
    Arc = 4,
    Text = 5,
    Symbol = 6,
    PolyLine = 7,
    // 8 never seen
    Contour = 9,
    Window = 10,
    MapSheet = 11,
    Triangle = 12,
    BlockInsert = 13,
    Empty = 14,
    Image = 15,            // NetCAD's "SmartObject": images, rich text and other property-based objects
};

enum Projection : u8 {
    Undefined = 0,
    Geographic = 1,
    Utm6Degree = 2,
    TransverseMercator3Degree = 3, // Turkish 3° zones: scale 1, false easting 500 km
};

enum Datum : u8 {
    Wgs84 = 0,     // uncertain, some files with it are named ITRF
    TurefItrf = 1,
    Ed50 = 4,
    Ed50Hgk = 254,
    // 83 and 255 also occur
};

// ---------------------------------------------------------------------------------------------------------------------
// Header objects

// Not decoded. The first three u32s differ per file, the fourth (0xbc or 0xbd) may be a format revision, and the two
// -1 values look like the "none" offsets of IndexPage.
struct FileHeader {
    u32 unknown1[3];
    u32 unknown2;
    s32 unknown3[2];
    u8 unknown4[parent.length - 24];
};

// Always 366 bytes; only the user name is known.
struct UserInformation {
    String name;
    u8 unknown[parent.length - sizeof(name)];
};

struct Scale {
    double scale; // map scale / 1000, e.g. 0.5 for 1:500; line patterns, symbols and point markers follow it
};

// Often stale; the geometry bounds are reliable.
struct Extents {
    Coordinate2D lower_bound;
    Coordinate2D upper_bound;
};

struct LinkedFile {
    PaddedString<79> path; // a .NCO file, empty in most files
};

bitfield LayerFlags {
    visible : 1;
    unknown1 : 7;
    point_markers : 1; // draw a small circle with a dot at each Point of the layer, see Layers.unknown1[5]
    unknown2 : 23;
};

struct Layer {
    LayerFlags flags;
    PaddedString<20> name;
    u8 color;          // NetCAD palette number; the palette's RGB values are in the LEX.ST2 metadata
    u8 unknown[3];     // mostly 0, 1, 0
};

struct Layers {
    // Each byte 0 or 1. Byte 5 switches the point markers of layers with LayerFlags.point_markers on (1) or off (0).
    u8 unknown1[9];
    u8 unknown2;       // mostly 1
    u8 unknown3;       // mostly 1 or 4
    u16 count;
    Layer layers[count]; // a geometry's layer_id indexes this table
};

// Segments: positive values are dashes, negative values gaps, 0 a dot. A value of k·1000 + n (either sign) places
// symbol n of the first library in Libraries at k times its size, turned along the line, and takes no length; a
// negative one turns the symbol a further half turn. Lengths are in map millimetres (scaled by Scale).
struct LineType {
    PaddedString<81> description;
    PaddedString<10> name;
    // Number of used segments. A negative count makes a multi-line: the line type also draws line type
    // `parallel_line_type` at `offset` to the right, which may chain on until a line type with a positive count.
    s8 segment_count;
    float pattern_length;      // the sum of the segment lengths for most line types, but not reliable
    float offset;
    u16 parallel_line_type;    // index into this table
    float segments[28];        // unused slots hold stale data
    u8 unknown[3];
};

struct LineTypes {
    LineType types[parent.length / 219]; // a geometry's line_type indexes this table
};

// Fonts and Libraries.
struct FileNames {
    u32 count;
    PaddedString<79> names[count];
};

struct BlockBegin {
    PaddedString<30> name; // inserted by BlockInsert geometries; blocks have no base point, it is (0, 0)
};

struct Version {
    String name; // e.g. "5.0.026"
};

struct RasterReference {
    PaddedString<80> path; // a .DRE raster registration file
    u8 unknown[100];
};

struct RasterReferences {
    RasterReference references[parent.length / 181];
};

// A spatial index over the geometries that follow. IndexGroup (29) has the same layout and groups the pages after it.
struct IndexPage {
    Coordinate2D lower_bound;
    Coordinate2D upper_bound;
    u32 geometry_types;  // bit 1 << t set when a geometry of type t is in the page
    u8 layer_mask[32];   // bit per layer index 0–255 (byte index / 8, bit index % 8)
    s32 next;            // file offset of the next page of the same kind, -1 for none
    s32 previous;        // file offset of the previous page, -1 for none
    u32 count;           // number of geometries in the page
};

// ---------------------------------------------------------------------------------------------------------------------
// Metadata (28): a padded name and content whose layout depends on the name.
//
// Decoded: SYMBOLS, MPROJ, LEX.ST2. Text (UTF-8 or UTF-16LE, often XML): REF (reference drawings and rasters), PRJPAR,
// P_MAP_PROJ, GEOPAR, CONMAN, GISWIZ, CAPMAN, VARIABLES, KmPrjProp, TILED_XML (holds SRS="EPSG:n"). Binary, unknown:
// MAPP, MACROPU, MPROJ2, LEX.STE, LEX.ST8.

struct SymbolLibrary {
    PaddedString<80> name; // e.g. F2.NCS, PLAN.NCS; the symbol shapes live in these NetCAD files, not in the NCZ
    double scale;
    u8 unknown[26];
};

struct SymbolLibraries {
    u32 version;         // 100
    u32 count;
    u32 unknown;
    SymbolLibrary libraries[count];
    SymbolLibrary terminator; // always empty
};

// The coordinate system, 137 bytes.
struct CoordinateSystem {
    Projection projection;
    Datum datum;
    u8 unknown1[3];
    u8 central_meridian; // degrees, e.g. 33
    u8 unknown2[object_end(addressof(parent.parent)) - $];
};

struct LayerStyle {
    u8 red;              // the RGB colour of the layer's palette number, as the user's palette defines it
    u8 green;
    u8 blue;
    u8 unknown[253];     // starts with a zero byte
};

// One entry per layer, in the order of Layers.
struct LayerStyles {
    u32 version;         // 100
    u32 layer_count;     // equals the Layers count; a few files hold one entry less than it says
    u8 unknown1[55];
    // The entries fill the content up to a 201-byte trailer.
    LayerStyle layers[(object_end(addressof(parent.parent)) - $ - 201) / 256];
    u8 unknown2[201];
};

struct Metadata {
    PaddedString<10> name;

    if (name.text == "SYMBOLS")
        SymbolLibraries content;
    else if (name.text == "MPROJ")
        CoordinateSystem content;
    else if (name.text == "LEX.ST2")
        LayerStyles content;
    else
        char content[object_end(addressof(parent)) - $];
};

// ---------------------------------------------------------------------------------------------------------------------
// Area records (30 … 31) wrap the polylines of an area: an outer ring and its holes. Some files wrap every polyline,
// open ones included; NetCAD draws them as outlines, nothing is filled.

struct AreaRing {
    u32 first_point;     // index of the ring's first point, counted over the rings (some records count from the 2nd)
    u32 point_count;
    u8 unknown1;
    char name[10];       // zero-padded, e.g. the area name without suffix
    Coordinate2D lower_bound; // real bounds on the last ring only, ±huge values on the others
    Coordinate2D upper_bound;
    u32 point_count2;    // equals point_count
};

struct AreaBegin {
    s32 unknown1;        // -1
    u32 unknown2;        // 196 + 24 × point_count for single-ring areas, probably a length
    u8 unknown3;         // 1
    PaddedString<10> name; // e.g. a parcel number
    u32 point_count;     // sum over the rings
    Coordinate2D lower_bound;
    Coordinate2D upper_bound;
    u16 unknown4;        // 1, rarely 2
    u16 hole_count;      // rings - 1
    Coordinate2D centroid;
    Coordinate2D label;  // equal to the centroid or near it
    u8 unknown5;         // 1, 0xff or 0 (0 only for polylines under 3 points); not a fill flag
    u8 unknown6;         // 0
    AreaRing rings[(object_end(addressof(parent)) - $) / 55];
};

struct AreaEnd {
    u32 begin_length;    // the body length of the matching AreaBegin
};

// Extra style for the next geometry, mostly before circles. No visible effect known.
struct GeometryStyle {
    u32 length;
    GeometryType geometry_type; // of the next geometry
    u8 unknown[object_end(addressof(parent)) - $];
};

struct LayerInformation {
    u32 color;
    String name;
};

// ---------------------------------------------------------------------------------------------------------------------
// Geometries (21, 22): a shared header, then a body by geometry type. Every body is read to its last byte; the unknown
// fields are of unknown meaning, not of unknown length. The anchor in the header is the first point of every type.

struct Attributes {
    u8 unknown1;         // 0, rarely 1
    u8 unknown2;         // mostly 0, maybe a line width or pen
    u8 unknown3;         // always 0
    u8 color;            // NetCAD palette number, 0 for the layer's colour
    u8 unknown4[8];      // almost always 0
    float spacing;       // Text: character spacing, scales the text width (0 means 1); other types: unknown, often 1
};

// Point. NetCAD picks the marker it draws from the name (P.… traverse, N.… triangulation) and the layer flags.
struct PointBody {
    PaddedString<10> name;
    u8 unknown[12];      // often looks like a padded string, e.g. a GNSS precision
};

// Line from the anchor to `end`.
struct LineBody {
    Vertex end;
};

// Circle around the anchor. A zero radius is drawn as a point.
struct CircleBody {
    double radius;
};

// Arc around the anchor.
struct ArcBody {
    double radius;
    float start_angle_copy;
    float end_angle_copy;
    u8 unknown[2];
    double start_angle;
    double end_angle;
};

bitfield TextFlags {
    italic : 1;
    underline : 1;
    highlight : 1;
    unknown : 5;
};

// Text at the anchor, which is the lower-left corner for most snap codes.
struct TextBody {
    float size;          // height
    float tilt;          // rotation
    // Alignment as an ASCII character:
    //   L, 6: left baseline    7: centre baseline   8: right baseline
    //   C: centre bottom       R: right bottom
    //   1, P: left middle      M: centre middle     5, Q: right middle
    //   2: left top            3, U: centre top     4: right top
    // P, Q and U also put the text half a height beside (P, Q) or above (U) the anchor. Others act as L.
    char snap;
    TextFlags flags;
    u8 font;             // index into Fonts
    String text;
};

// A symbol of a NetCAD symbol library (the shapes are not in the NCZ).
struct SymbolBody {
    float scale;         // 0: not drawn
    float rotation;
    u16 symbol;          // number in the library
    u8 library;          // index into Libraries
};

bitfield PolyLineFlags {
    closed : 1;
    unknown1 : 1;
    curve : 1;           // drawn as a curve through the points
    unknown2 : 1;
    polyline : 1;        // common, meaning unknown
    unknown3 : 1;
    smooth_curve : 1;
    unknown4 : 25;
};

// Points continue to the end of the object; the anchor equals the first one.
struct PolyLineBody {
    PaddedString<10> name;
    u32 id;              // unique in the file
    PolyLineFlags flags;
    double area;         // registered area for parcels, often 0, sometimes stale
    Coordinate3D points[(object_end(addressof(parent.parent)) - $) / 24];
};

struct ContourPoint {
    float dx;            // offset from the anchor
    float dy;
    u8 gap;              // 1: part of the gap left for the elevation label
};

// A contour line at the anchor's elevation.
struct ContourBody {
    u8 unknown[36];
    ContourPoint points[(object_end(addressof(parent.parent)) - $) / 9];
};

// A named rectangle, e.g. a plot window: the anchor is one corner, `corner2` the opposite corner before rotation.
struct WindowBody {
    PaddedString<17> name;
    Coordinate2D corner2;
    float rotation;      // around the anchor
};

// A map sheet (pafta) frame, e.g. "G29-A-07-B-3-A". The name gives the geographic rectangle in the Turkish sheet
// system; the header bounds are the box of its projected corners.
struct MapSheetBody {
    PaddedString<17> name;
};

// A face of a triangulated terrain model; the anchor is the first vertex.
struct TriangleBody {
    Vertex second;
    Vertex third;
    float min_z;
    float max_z;
};

struct BlockInsertBody {
    PaddedString<34> name; // a block of BlockBegin
    u32 id;
    u32 unknown;           // mostly 1
    double rotation;
    double scale_x;
    double scale_y;
};

struct ImageProperty {
    String key;          // imageContent (base64 image), rtfData (base64 RTF), imageWidth, imageHeight, width, …
    u8 kind;
    VarintString value;
    VarintString caption;
    u8 unknown[7];
};

// An object described by properties: an embedded image, rich text, or another kind (e.g. a grid).
struct ImageBody {
    float scale;
    u8 unknown1[24];
    u32 data_length;     // bytes after this field, plus 8
    u8 kind;             // 2 for images and rich text, 1 for others
    u8 class_id[16];     // identifies the kind of object
    u32 unknown2;
    u32 property_count;
    ImageProperty properties[property_count];
    // A few records have 4 more zero bytes here (shown as `unparsed`).
};

struct Geometry {
    u8 unknown1;         // 0 up to version 7, a copy of the geometry type in version 8
    GeometryType type;
    u8 layer_id;         // index into Layers
    Vertex anchor;
    u8 padding[4];
    u8 line_type;        // index into LineTypes
    u8 line_type2;       // equal to line_type, sometimes with bit 0x80 set (meaning unknown)
    Attributes attributes;
    Coordinate2D lower_bound; // drawn extent; inverted and empty for Empty and unplaced symbols
    Coordinate2D upper_bound;
    u32 unknown2;        // always 0
    if (parent.type == ObjectType::GeometryWithCode) {
        PaddedString<10> code; // feature code
        PaddedString<16> link; // mostly a database record reference such as @TAR453489, sometimes a free label
    }

    match (type) {
        (GeometryType::Point): PointBody body;
        (GeometryType::Line): LineBody body;
        (GeometryType::Circle): CircleBody body;
        (GeometryType::Arc): ArcBody body;
        (GeometryType::Text): TextBody body;
        (GeometryType::Symbol): SymbolBody body;
        (GeometryType::PolyLine): PolyLineBody body;
        (GeometryType::Contour): ContourBody body;
        (GeometryType::Window): WindowBody body;
        (GeometryType::MapSheet): MapSheetBody body;
        (GeometryType::Triangle): TriangleBody body;
        (GeometryType::BlockInsert): BlockInsertBody body;
        (GeometryType::Image): ImageBody body;
        // Empty has no body; other types never occur.
        (_): u8 body[object_end(addressof(parent)) - $];
    }
};

// ---------------------------------------------------------------------------------------------------------------------
// Objects

struct Object {
    ObjectType type;
    u32 length;

    match (type) {
        (ObjectType::FileHeader): FileHeader body;
        (ObjectType::UserInformation): UserInformation body;
        (ObjectType::Scale): Scale body;
        (ObjectType::Extents): Extents body;
        (ObjectType::LinkedFile): LinkedFile body;
        (ObjectType::Layers): Layers body;
        (ObjectType::LineTypes): LineTypes body;
        (ObjectType::Fonts): FileNames body;
        (ObjectType::Libraries): FileNames body; // symbol and block libraries, e.g. F2.NCS
        (ObjectType::Thumbnail): u8 bmp[length]; // a complete BMP file
        (ObjectType::BlockBegin): BlockBegin body;
        (ObjectType::Geometry | ObjectType::GeometryWithCode): Geometry body;
        (ObjectType::Version): Version body;
        (ObjectType::RasterReferences): RasterReferences body;
        (ObjectType::IndexPage | ObjectType::IndexGroup): IndexPage body;
        (ObjectType::Metadata): Metadata body;
        (ObjectType::AreaBegin): AreaBegin body;
        (ObjectType::AreaEnd): AreaEnd body;
        (ObjectType::GeometryStyle): GeometryStyle body;
        (ObjectType::LayerInformation): LayerInformation body;
        // Marker, BlockSectionBegin, BlockEnd and BlockSectionEnd are empty; anything else is unknown or corrupt.
        (_): u8 body[length];
    }

    // Leftover bytes, e.g. a partial line type record or stale data.
    if ($ < object_end(addressof(this)))
        u8 unparsed[object_end(addressof(this)) - $];
};

// Stops at the end of the file. Corrupt files (garbage object types or lengths) need parsing to restart by hand at the
// next valid object.
Object objects[while(!std::mem::eof())] @ 0x00;
