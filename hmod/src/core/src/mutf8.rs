//! Java NBT stores strings as modified UTF-8, including compound keys and root names.

pub(crate) fn decode(bytes: &[u8]) -> Result<String, String> {
    // Older native exports used standard UTF-8, which remains readable here.
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Ok(text.to_owned());
    }

    const INVALID: &str = "NBT: invalid modified UTF-8";
    let continuation = |byte: u8| {
        if byte & 0xc0 == 0x80 {
            Ok(u16::from(byte & 0x3f))
        } else {
            Err(INVALID)
        }
    };
    let mut units = Vec::new();
    let mut bytes = bytes.iter().copied();
    while let Some(first) = bytes.next() {
        let unit = match first {
            0x00..=0x7f => u16::from(first),
            0xc0..=0xdf => {
                let next = continuation(bytes.next().ok_or(INVALID)?)?;
                let value = (u16::from(first & 0x1f) << 6) | next;
                if value != 0 && value < 0x80 {
                    return Err(INVALID.into());
                }
                value
            }
            0xe0..=0xef => {
                let second = continuation(bytes.next().ok_or(INVALID)?)?;
                let third = continuation(bytes.next().ok_or(INVALID)?)?;
                let value = (u16::from(first & 0x0f) << 12) | (second << 6) | third;
                if value < 0x800 {
                    return Err(INVALID.into());
                }
                value
            }
            _ => return Err(INVALID.into()),
        };
        units.push(unit);
    }
    String::from_utf16(&units).map_err(|_| INVALID.into())
}

pub(crate) fn encode(value: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(value.len());
    for unit in value.encode_utf16() {
        match unit {
            0x01..=0x7f => bytes.push(unit as u8),
            0x00 | 0x80..=0x7ff => {
                bytes.push(0xc0 | (unit >> 6) as u8);
                bytes.push(0x80 | (unit & 0x3f) as u8);
            }
            _ => {
                bytes.push(0xe0 | (unit >> 12) as u8);
                bytes.push(0x80 | ((unit >> 6) & 0x3f) as u8);
                bytes.push(0x80 | (unit & 0x3f) as u8);
            }
        }
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::{decode, encode};

    #[test]
    fn java_modified_utf8_wire_bytes() {
        let value = "A\0é水😀";
        let bytes = [
            0x41, 0xc0, 0x80, 0xc3, 0xa9, 0xe6, 0xb0, 0xb4, 0xed, 0xa0, 0xbd, 0xed, 0xb8, 0x80,
        ];
        assert_eq!(encode(value), bytes);
        assert_eq!(decode(&bytes).unwrap(), value);
    }

    #[test]
    fn reads_previous_native_utf8_strings() {
        let value = "old\0😀";
        assert_eq!(decode(value.as_bytes()).unwrap(), value);
    }

    #[test]
    fn rejects_truncated_and_invalid_java_strings() {
        assert!(decode(&[0xc0]).is_err());
        assert!(decode(&[0xc0, 0x81]).is_err());
        assert!(decode(&[0xed, 0xa0, 0xbd]).is_err());
        assert!(decode(&[0xe0, b'A', 0x80]).is_err());
    }
}
