use crate::object::Object;
use bopper::{BigEndianReadBuffer, ReadBuffer};

pub mod coordinate;
#[cfg(feature = "dxf")]
pub mod dxf_export;
pub mod geometry;
pub mod line_type;
pub mod object;
pub mod string;
#[cfg(feature = "dxf")]
pub mod summary;
#[cfg(target_arch = "wasm32")]
mod wasm;

#[cfg(feature = "dxf")]
pub use dxf_export::objects_to_dxf;

pub fn parse_objects(bytes: &[u8]) -> Vec<Object> {
    let mut buffer = BigEndianReadBuffer::new(bytes);

    let mut objects = vec![];
    while !buffer.is_empty() {
        let Some(object) = Object::from_buffer(&mut buffer) else {
            continue;
        };

        objects.push(object);
    }

    objects
}
