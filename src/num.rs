fn _decode_hex_digit(d: u8) -> Option<u8> {
    match d {
        b'0'..=b'9' => Some(d - b'0'),
        b'a'..=b'f' => Some(d - b'a' + 10),
        b'A'..=b'F' => Some(d - b'A' + 10),
        _ => None,
    }
}

#[test]
fn test_hex_decode() {
    assert_eq!(_decode_hex_digit(b'f').unwrap(), 0xf);
    assert_eq!(_decode_hex_digit(b'9').unwrap(), 0x9);
    assert_eq!(_decode_hex_digit(b'F').unwrap(), 0xf);
    assert_eq!(_decode_hex_digit(b'Z'), None);
}

/// Parses an arbitrary-length hex literal into bytes
pub(crate) fn parse_hex(hex: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();

    for chunk in hex.chunks_exact(2) {
        let (d1, d2) = match chunk {
            &[a, b] => (a, b),
            _ => unreachable!("chunks_exact guarantees this will not happen"),
        };
        out.push(_decode_hex_digit(d1)? << 4 | _decode_hex_digit(d2)?);
    }
    Some(out)
}

#[test]
fn test_parse_hex() {
    assert_eq!(parse_hex(b"00").unwrap(), vec![0x00]);
    assert_eq!(parse_hex(b"10").unwrap(), vec![0x10]);
    assert_eq!(parse_hex(b"a1").unwrap(), vec![0xa1]);
    assert_eq!(parse_hex(b"a1b2").unwrap(), vec![0xa1, 0xb2]);
}

/// Parses an octal literal
pub(crate) fn parse_octal(octal: &[u8]) -> Option<u32> {
    let mut out: u32 = 0;
    for &digit in octal {
        if digit < b'0' || digit > b'7' {
            return None;
        }
        out <<= 3;
        // this is *maybe* evil bit twiddling?
        let n = digit - b'0';
        out += n as u32;
    }
    Some(out)
}

#[test]
fn test_parse_octal() {
    assert_eq!(parse_octal("777".as_bytes()).unwrap(), 0o777);
    assert_eq!(parse_octal("501".as_bytes()).unwrap(), 0o501);
    assert_eq!(parse_octal("8".as_bytes()), None);
}
