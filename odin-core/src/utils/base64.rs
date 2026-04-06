//! Base64 encoding and decoding.

const ENCODE_TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Encode bytes to base64 string.
pub fn encode(data: &[u8]) -> String {
    let mut result = String::with_capacity(data.len().div_ceil(3) * 4);
    let mut i = 0;

    while i + 2 < data.len() {
        let n = (u32::from(data[i]) << 16) | (u32::from(data[i + 1]) << 8) | u32::from(data[i + 2]);
        result.push(ENCODE_TABLE[(n >> 18) as usize & 0x3F] as char);
        result.push(ENCODE_TABLE[(n >> 12) as usize & 0x3F] as char);
        result.push(ENCODE_TABLE[(n >> 6) as usize & 0x3F] as char);
        result.push(ENCODE_TABLE[n as usize & 0x3F] as char);
        i += 3;
    }

    match data.len() - i {
        1 => {
            let n = u32::from(data[i]) << 16;
            result.push(ENCODE_TABLE[(n >> 18) as usize & 0x3F] as char);
            result.push(ENCODE_TABLE[(n >> 12) as usize & 0x3F] as char);
            result.push('=');
            result.push('=');
        }
        2 => {
            let n = (u32::from(data[i]) << 16) | (u32::from(data[i + 1]) << 8);
            result.push(ENCODE_TABLE[(n >> 18) as usize & 0x3F] as char);
            result.push(ENCODE_TABLE[(n >> 12) as usize & 0x3F] as char);
            result.push(ENCODE_TABLE[(n >> 6) as usize & 0x3F] as char);
            result.push('=');
        }
        _ => {}
    }

    result
}

/// Decode base64 string to bytes.
pub fn decode(input: &str) -> Result<Vec<u8>, String> {
    let mut output = Vec::with_capacity(input.len() * 3 / 4);
    let mut buffer: u32 = 0;
    let mut bits: u8 = 0;

    for ch in input.bytes() {
        let val = match ch {
            b'A'..=b'Z' => ch - b'A',
            b'a'..=b'z' => ch - b'a' + 26,
            b'0'..=b'9' => ch - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\n' | b'\r' | b' ' => continue,
            _ => return Err(format!("invalid base64 character: {}", ch as char)),
        };
        buffer = (buffer << 6) | u32::from(val);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode_roundtrip() {
        let data = b"Hello, World!";
        let encoded = encode(data);
        assert_eq!(encoded, "SGVsbG8sIFdvcmxkIQ==");
        let decoded = decode(&encoded).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_empty() {
        assert_eq!(encode(b""), "");
        assert_eq!(decode("").unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn test_padding_cases() {
        assert_eq!(encode(b"a"), "YQ==");
        assert_eq!(encode(b"ab"), "YWI=");
        assert_eq!(encode(b"abc"), "YWJj");
    }
}
