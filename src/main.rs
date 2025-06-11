use bopper::{BigEndianReadBuffer, ReadBuffer};
use jiff::{Unit, Zoned};
use std::fs::File;
use std::io::{Read, Write};
use nczrs::object::Object;

fn main() {
    let mut file = File::open("ncz/Karacapınar_ED50_3DERECE.NCZ").unwrap();
    let mut bytes = vec![];

    file.read_to_end(&mut bytes).unwrap();

    let mut buffer = BigEndianReadBuffer::new(bytes.as_slice());

    let mut objects = vec![];
    while !buffer.is_empty() {
        let Some(object) = Object::from_buffer(&mut buffer) else {
            println!("parse failed");
            continue;
        };

        objects.push(object);
    }

    let serialized = serde_json::to_vec(&objects).unwrap();

    let filename = format!(
        "out/{}.json",
        &Zoned::now()
            .round(Unit::Second)
            .unwrap()
            .strftime("%Y%m%d_%H%M%S")
    );
    
    let mut outfile = File::create(filename).unwrap();
    let _ = outfile.write(&serialized).unwrap();
}
