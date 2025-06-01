struct String {
    u8 length;
    char content[length];
};

struct Coordinate2D {
    double x;
    double y;
};

struct Coordinate3D {
    double x;
    double y;
    double z;
};

struct LineType {
    u8 unknown[82];
    String name;
    u8 unknown2[136 - name.length];
};

struct Point {
    Coordinate3D position;
};

struct Line {
    u8 unknown1[1];
    Coordinate3D start;
    u8 line_type;
    u8 line_type2;
    u8 unknown2[16];
    Coordinate2D lower_bound;
    Coordinate2D upper_bound;
    u8 unknown3[4];
    Coordinate2D end;
    u8 unknown4[4];
};

struct Circle {
    u8 unknown[1];
    Coordinate3D center;
    u8 unknown2[18];
    Coordinate2D lower_bound;
    Coordinate2D upper_bound;
    u8 unknown3[4];
    double radius;
};

struct Arc {
    u8 unknown[1];
    Coordinate3D center;
    u8 unknown2[18];
    Coordinate2D lower_bound;
    Coordinate2D upper_bound;
    u8 unknown3[4];
    double radius;
    u8 unknown4[10];
    double start_angle;
    double end_angle;
};

struct Text {
    u8 unknown1[1];
    Coordinate3D position;
    u8 unknown2[14];
    float spacing;
    Coordinate2D lower_bound;
    Coordinate2D upper_bound;
    u8 unknown3[4];
    float size;
    float tilt;
    u8 snap;
    u8 flags; // ? ? ? ? ? HIGHLIGHT UNDERLINE ITALIC
    u8 unknown4[1];
    String text;
};

enum PolyLineType : u8 {
    Curve = 4,
    PolyLine = 16,
    SmoothCurve = 64,
};

struct PolyLine {
    u8 unknown1[43];
    Coordinate2D lower_bound;
    Coordinate2D upper_bound;
    u8 unknown2[19];
    PolyLineType polyline_type;
    u8 unknown3[11];
    // Lines
};

enum GeometryType :u8 {
    Unknown = 0,
    Point = 1,
    Line = 2,
    Circle = 3,
    Arc = 4,
    Text = 5,
    PolyLine = 7
};

struct Geometry {
    u8 layer_id;
    GeometryType type;

    if (type == GeometryType::Point) {
        Point point;
    } else if (type == GeometryType::Line) {
        Line line;
    } else if (type == GeometryType::Circle) {
        Circle circle;
    } else if (type == GeometryType::Arc) {
        Arc arc;
    } else if (type == GeometryType::Text) {
        Text;
    } else if (type == GeometryType::PolyLine) {
        PolyLine polyline;
    } else {
        u8 body[parent.length];
    }
};

enum ObjectType : u8 {
    Unknown = 0,
    FileHeader = 1,
    UserInformation = 2,
    LineTypes = 8,
    Geometry = 21,
    Version = 25,
    Metadata = 28,
    LayerInformation = 226,
};

struct Object {
    ObjectType type;
    u32 length;

    if (type == ObjectType::UserInformation) {
        String username;
        u8 unknown1[length - sizeof(username)];
    } else if (type == ObjectType::LineTypes) {
        LineType line_type[127];
    } else if (type == ObjectType::Geometry) {
        Geometry geometry;
    } else if (type == ObjectType::Metadata) {
        String name;
        u8 pad[10 - name.length];
        char content[length - 11];
    } else {
        u8 body[length];
    }
};

Object object_at_0x00[49] @ 0x00;