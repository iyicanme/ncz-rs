use bopper::BigEndianReadBuffer;
use nczrs::{Object};
use std::fs::File;
use std::io::{Read, Write};
use jiff::{Unit, Zoned};

fn main() {
    let mut file = File::open("ncz/FOO.NCZ").unwrap();
    let mut bytes = vec![];

    file.read_to_end(&mut bytes).unwrap();

    let mut buffer = BigEndianReadBuffer::new(bytes.as_slice());

    let mut objects = vec![];
    while let Some(object) = Object::from_buffer(&mut buffer) {
        // print_object(&object);
        objects.push(object);
    }

    let serialized = serde_json::to_vec(&objects).unwrap();

    let filename = format!("out/{}.json", &Zoned::now().round(Unit::Second).unwrap().strftime("%Y%m%d_%H%M%S"));
    let mut outfile = File::create(filename).unwrap();
    let _ = outfile.write(&serialized).unwrap();
}
