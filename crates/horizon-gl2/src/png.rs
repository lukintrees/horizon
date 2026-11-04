//! A minimal PNG encoder for the probe's frames: RGBA, 8 bits per channel, stored (uncompressed)
//! deflate blocks. Frames are small and only leave the CI runner or the card as artifacts, so the
//! size does not matter, and no crate is pulled in for it.

/// Encodes `rgba` (rows top to bottom, 4 bytes per pixel) as a PNG file.
pub fn encode_rgba(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let row = width as usize * 4;
    debug_assert_eq!(rgba.len(), row * height as usize);

    // Each scanline starts with its filter type; 0 means "no filter".
    let mut raw = Vec::with_capacity((row + 1) * height as usize);
    for y in 0..height as usize {
        raw.push(0);
        raw.extend_from_slice(&rgba[y * row..(y + 1) * row]);
    }

    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    // Bit depth 8, colour type 6 (RGBA), compression 0, filter 0, no interlace.
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    push_chunk(&mut out, b"IHDR", &header);
    push_chunk(&mut out, b"IDAT", &zlib_stored(&raw));
    push_chunk(&mut out, b"IEND", &[]);
    out
}

fn push_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut body = Vec::with_capacity(4 + data.len());
    body.extend_from_slice(kind);
    body.extend_from_slice(data);
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc32(&body).to_be_bytes());
}

/// A zlib stream made of stored deflate blocks: header, blocks of at most 65535 bytes, Adler-32.
fn zlib_stored(raw: &[u8]) -> Vec<u8> {
    // CMF 0x78 (deflate, 32 KiB window) and FLG 0x01, which makes the pair a multiple of 31.
    let mut out = vec![0x78, 0x01];
    if raw.is_empty() {
        // One final empty stored block.
        out.extend_from_slice(&[1, 0, 0, 0xFF, 0xFF]);
    }
    let mut blocks = raw.chunks(65_535).peekable();
    while let Some(block) = blocks.next() {
        let last = blocks.peek().is_none();
        // The first byte holds BFINAL in bit 0 and BTYPE 00 (stored); the rest is padding.
        out.push(u8::from(last));
        let len = block.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(block);
    }
    out.extend_from_slice(&adler32(raw).to_be_bytes());
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    const MODULUS: u32 = 65_521;
    // 5552 is the longest run for which the sums cannot overflow 32 bits before the modulo.
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(5552) {
        for &byte in chunk {
            a += u32::from(byte);
            b += a;
        }
        a %= MODULUS;
        b %= MODULUS;
    }
    (b << 16) | a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_the_standard_check_values() {
        // The CRC of the IEND chunk type is the constant every PNG ends with.
        assert_eq!(crc32(b"IEND"), 0xAE42_6082);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn adler32_matches_the_standard_check_value() {
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
        assert_eq!(adler32(b""), 1);
    }

    #[test]
    fn one_pixel_file_has_the_png_structure() {
        let png = encode_rgba(1, 1, &[255, 0, 0, 255]);
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        // The file ends with the IEND chunk: length 0, type, and its fixed CRC.
        assert_eq!(&png[png.len() - 12..], &[0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82]);
        // IHDR follows the signature with length 13.
        assert_eq!(&png[8..16], &[0, 0, 0, 13, b'I', b'H', b'D', b'R']);
    }

    #[test]
    fn large_frames_split_into_several_stored_blocks() {
        // 300 rows of 300 pixels is about 360 KB, so several blocks are needed.
        let rgba = vec![7u8; 300 * 300 * 4];
        let png = encode_rgba(300, 300, &rgba);
        assert!(png.len() > rgba.len());
        assert_eq!(&png[png.len() - 4..], &[0xAE, 0x42, 0x60, 0x82]);
    }
}
