//! Minimal base64 encoding/decoding. Avoids pulling in an external crate
//! for a simple operation used by storage (WASM localStorage) and invite codes.

pub fn encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        result.push(CHARS[((n >> 18) & 63) as usize] as char);
        result.push(CHARS[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARS[((n >> 6) & 63) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[(n & 63) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

pub fn decode(input: &str) -> Option<Vec<u8>> {
    fn char_val(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a' + 26) as u32),
            b'0'..=b'9' => Some((c - b'0' + 52) as u32),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let bytes = input.as_bytes();
    if !bytes.len().is_multiple_of(4) || bytes.is_empty() {
        return None;
    }
    let mut result = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        let a = char_val(chunk[0])?;
        let b = char_val(chunk[1])?;
        let n = (a << 18) | (b << 12);
        result.push((n >> 16) as u8);
        if chunk[2] != b'=' {
            let c = char_val(chunk[2])?;
            let n = n | (c << 6);
            result.push((n >> 8) as u8);
            if chunk[3] != b'=' {
                let d = char_val(chunk[3])?;
                let n = n | d;
                result.push(n as u8);
            }
        }
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let data = b"hello, willow!";
        let encoded = encode(data);
        let decoded = decode(&encoded).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn empty() {
        let encoded = encode(b"");
        assert_eq!(encoded, "");
        assert!(decode("").is_none());
    }

    #[test]
    fn padding() {
        // 1 byte -> 4 chars with 2 padding
        let e1 = encode(b"a");
        assert!(e1.ends_with("=="));
        assert_eq!(decode(&e1).unwrap(), b"a");

        // 2 bytes -> 4 chars with 1 padding
        let e2 = encode(b"ab");
        assert!(e2.ends_with('='));
        assert_eq!(decode(&e2).unwrap(), b"ab");

        // 3 bytes -> 4 chars, no padding
        let e3 = encode(b"abc");
        assert!(!e3.contains('='));
        assert_eq!(decode(&e3).unwrap(), b"abc");
    }

    #[test]
    fn invalid_input() {
        assert!(decode("!!!").is_none());
        assert!(decode("ab").is_none()); // not multiple of 4
    }
}
