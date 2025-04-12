use nanoid::nanoid;
use crate::types::ReverieId;

pub const TOPIC_DELIMITER: &'static str = "/";
const NANOID_ALPHABET: [char; 16] = [
    '1', '2', '3', '4', '5', '6', '7', '8', '9', '0', 'a', 'b', 'c', 'd', 'e', 'f'
];

pub fn nanoid4() -> String {
    nanoid!(4, &NANOID_ALPHABET) //=> "4f90"
}

pub const REVERIE_ID_PREFIX: &'static str = "reverie_";

pub fn reverie_id() -> ReverieId {
    format!("{}{}", REVERIE_ID_PREFIX, nanoid!(16, &NANOID_ALPHABET))
}