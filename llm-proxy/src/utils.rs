use bytes::Bytes;
use flate2::read::{GzDecoder, DeflateDecoder};
use std::io::{self, Read};

/// Decompresses response body bytes based on Content-Encoding header.
///
/// Args:
///     content_encoding: Optional string slice representing the Content-Encoding header value.
///     body_bytes: Bytes object containing the raw response body.
///
/// Returns:
///     Result containing a Vec<u8> of the decompressed bytes, or an io::Error if decompression fails.
pub fn decompress_body(content_encoding: Option<&str>, body_bytes: &Bytes) -> io::Result<Vec<u8>> {
    let mut decompressed_bytes = Vec::new();

    match content_encoding {
        Some("gzip") => {
            let mut decoder = GzDecoder::new(&body_bytes[..]);
            decoder.read_to_end(&mut decompressed_bytes)?; // Propagate error
        },
        Some("deflate") => {
            let mut decoder = DeflateDecoder::new(&body_bytes[..]);
            decoder.read_to_end(&mut decompressed_bytes)?; // Propagate error
        },
        _ => {
            // No compression or unknown, copy original bytes
            decompressed_bytes.extend_from_slice(body_bytes);
        }
    }
    Ok(decompressed_bytes)
}