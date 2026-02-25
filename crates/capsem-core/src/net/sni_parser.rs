/// Extract the SNI hostname from a TLS ClientHello message.
///
/// Parses the TLS record layer and ClientHello handshake to find the
/// Server Name Indication (SNI) extension (type 0x0000). Returns the
/// hostname string if found, or `None` if the data is not a valid
/// ClientHello or contains no SNI extension.
///
/// This handles both TLS 1.2 and TLS 1.3 ClientHello messages, as the
/// SNI extension format is identical in both versions.
pub fn extract_sni(data: &[u8]) -> Option<String> {
    // Minimum TLS record: 5 bytes header + 4 bytes handshake header + content
    if data.len() < 11 {
        return None;
    }

    // TLS record layer
    // [0]    ContentType: 0x16 = Handshake
    // [1..3] ProtocolVersion (ignored -- TLS 1.3 still sends 0x0301 here)
    // [3..5] Length of handshake data
    if data[0] != 0x16 {
        return None;
    }

    let record_len = u16::from_be_bytes([data[3], data[4]]) as usize;
    let handshake = if data.len() >= 5 + record_len {
        &data[5..5 + record_len]
    } else {
        // Truncated record -- use what we have
        &data[5..]
    };

    // Handshake header
    // [0]    HandshakeType: 0x01 = ClientHello
    // [1..4] Length (24-bit)
    if handshake.is_empty() || handshake[0] != 0x01 {
        return None;
    }

    if handshake.len() < 4 {
        return None;
    }

    let hello_len =
        ((handshake[1] as usize) << 16) | ((handshake[2] as usize) << 8) | handshake[3] as usize;
    let hello = if handshake.len() >= 4 + hello_len {
        &handshake[4..4 + hello_len]
    } else {
        &handshake[4..]
    };

    // ClientHello body
    // [0..2]  ProtocolVersion
    // [2..34] Random (32 bytes)
    // [34]    SessionID length
    // [...]   SessionID
    // [...]   CipherSuites length (2 bytes) + data
    // [...]   CompressionMethods length (1 byte) + data
    // [...]   Extensions length (2 bytes) + extensions
    let mut pos = 0;

    // Skip version (2) + random (32) = 34 bytes
    if hello.len() < 34 {
        return None;
    }
    pos += 34;

    // Skip session ID
    if pos >= hello.len() {
        return None;
    }
    let session_id_len = hello[pos] as usize;
    pos += 1 + session_id_len;

    // Skip cipher suites
    if pos + 2 > hello.len() {
        return None;
    }
    let cipher_suites_len = u16::from_be_bytes([hello[pos], hello[pos + 1]]) as usize;
    pos += 2 + cipher_suites_len;

    // Skip compression methods
    if pos >= hello.len() {
        return None;
    }
    let compression_len = hello[pos] as usize;
    pos += 1 + compression_len;

    // Extensions
    if pos + 2 > hello.len() {
        return None;
    }
    let extensions_len = u16::from_be_bytes([hello[pos], hello[pos + 1]]) as usize;
    pos += 2;

    let extensions_end = pos + extensions_len;
    let extensions_end = extensions_end.min(hello.len());

    // Walk extensions looking for SNI (type 0x0000)
    while pos + 4 <= extensions_end {
        let ext_type = u16::from_be_bytes([hello[pos], hello[pos + 1]]);
        let ext_len = u16::from_be_bytes([hello[pos + 2], hello[pos + 3]]) as usize;
        pos += 4;

        if ext_type == 0x0000 {
            // SNI extension
            return parse_sni_extension(&hello[pos..pos.saturating_add(ext_len).min(hello.len())]);
        }

        pos += ext_len;
    }

    None
}

/// Parse the SNI extension data to extract the hostname.
///
/// SNI extension format:
/// [0..2] ServerNameList length
/// For each entry:
///   [0]    NameType (0 = host_name)
///   [1..3] HostName length
///   [3..]  HostName (UTF-8)
fn parse_sni_extension(data: &[u8]) -> Option<String> {
    if data.len() < 2 {
        return None;
    }

    let list_len = u16::from_be_bytes([data[0], data[1]]) as usize;
    let list = if data.len() >= 2 + list_len {
        &data[2..2 + list_len]
    } else {
        &data[2..]
    };

    let mut pos = 0;
    while pos + 3 <= list.len() {
        let name_type = list[pos];
        let name_len = u16::from_be_bytes([list[pos + 1], list[pos + 2]]) as usize;
        pos += 3;

        if name_type == 0 {
            // host_name
            if pos + name_len <= list.len() {
                return std::str::from_utf8(&list[pos..pos + name_len])
                    .ok()
                    .map(|s| s.to_lowercase());
            }
        }
        pos += name_len;
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // A minimal TLS 1.2 ClientHello with SNI="example.com"
    // Built by hand following the TLS 1.2 spec.
    fn make_client_hello(hostname: &str) -> Vec<u8> {
        let hostname_bytes = hostname.as_bytes();

        // SNI extension
        let sni_entry_len = 1 + 2 + hostname_bytes.len(); // type(1) + len(2) + name
        let sni_list_len = sni_entry_len;
        let sni_ext_data_len = 2 + sni_list_len; // list_len(2) + entries

        let mut sni_ext = Vec::new();
        sni_ext.extend_from_slice(&0x0000u16.to_be_bytes()); // extension type = SNI
        sni_ext.extend_from_slice(&(sni_ext_data_len as u16).to_be_bytes()); // ext length
        sni_ext.extend_from_slice(&(sni_list_len as u16).to_be_bytes()); // list length
        sni_ext.push(0x00); // name type = host_name
        sni_ext.extend_from_slice(&(hostname_bytes.len() as u16).to_be_bytes());
        sni_ext.extend_from_slice(hostname_bytes);

        // Extensions block
        let extensions_len = sni_ext.len();

        // ClientHello body
        let mut hello_body = Vec::new();
        hello_body.extend_from_slice(&[0x03, 0x03]); // version: TLS 1.2
        hello_body.extend_from_slice(&[0u8; 32]); // random
        hello_body.push(0); // session_id length = 0
        hello_body.extend_from_slice(&2u16.to_be_bytes()); // cipher suites length
        hello_body.extend_from_slice(&[0x00, 0x2f]); // TLS_RSA_WITH_AES_128_CBC_SHA
        hello_body.push(1); // compression methods length
        hello_body.push(0); // null compression
        hello_body.extend_from_slice(&(extensions_len as u16).to_be_bytes());
        hello_body.extend_from_slice(&sni_ext);

        // Handshake header
        let mut handshake = Vec::new();
        handshake.push(0x01); // ClientHello
        let hello_len = hello_body.len();
        handshake.push((hello_len >> 16) as u8);
        handshake.push((hello_len >> 8) as u8);
        handshake.push(hello_len as u8);
        handshake.extend_from_slice(&hello_body);

        // TLS record
        let mut record = Vec::new();
        record.push(0x16); // Handshake
        record.extend_from_slice(&[0x03, 0x01]); // TLS 1.0 (compat)
        record.extend_from_slice(&(handshake.len() as u16).to_be_bytes());
        record.extend_from_slice(&handshake);

        record
    }

    fn make_client_hello_no_sni() -> Vec<u8> {
        // A ClientHello with no extensions at all
        let mut hello_body = Vec::new();
        hello_body.extend_from_slice(&[0x03, 0x03]); // TLS 1.2
        hello_body.extend_from_slice(&[0u8; 32]); // random
        hello_body.push(0); // session_id length
        hello_body.extend_from_slice(&2u16.to_be_bytes()); // cipher suites length
        hello_body.extend_from_slice(&[0x00, 0x2f]);
        hello_body.push(1); // compression length
        hello_body.push(0);
        // No extensions

        let mut handshake = Vec::new();
        handshake.push(0x01);
        let len = hello_body.len();
        handshake.push((len >> 16) as u8);
        handshake.push((len >> 8) as u8);
        handshake.push(len as u8);
        handshake.extend_from_slice(&hello_body);

        let mut record = Vec::new();
        record.push(0x16);
        record.extend_from_slice(&[0x03, 0x01]);
        record.extend_from_slice(&(handshake.len() as u16).to_be_bytes());
        record.extend_from_slice(&handshake);

        record
    }

    // -- Positive cases --

    #[test]
    fn extract_sni_simple_domain() {
        let data = make_client_hello("example.com");
        assert_eq!(extract_sni(&data), Some("example.com".to_string()));
    }

    #[test]
    fn extract_sni_subdomain() {
        let data = make_client_hello("api.github.com");
        assert_eq!(extract_sni(&data), Some("api.github.com".to_string()));
    }

    #[test]
    fn extract_sni_deep_subdomain() {
        let data = make_client_hello("a.b.c.d.example.org");
        assert_eq!(extract_sni(&data), Some("a.b.c.d.example.org".to_string()));
    }

    #[test]
    fn extract_sni_normalizes_to_lowercase() {
        let data = make_client_hello("GitHub.COM");
        assert_eq!(extract_sni(&data), Some("github.com".to_string()));
    }

    #[test]
    fn extract_sni_long_hostname() {
        let long_host = format!("{}.example.com", "a".repeat(200));
        let data = make_client_hello(&long_host);
        assert_eq!(extract_sni(&data), Some(long_host.to_lowercase()));
    }

    // -- Negative cases --

    #[test]
    fn extract_sni_empty_input() {
        assert_eq!(extract_sni(&[]), None);
    }

    #[test]
    fn extract_sni_too_short() {
        assert_eq!(extract_sni(&[0x16, 0x03, 0x01]), None);
    }

    #[test]
    fn extract_sni_not_handshake() {
        // Application data record (type 0x17)
        let mut data = make_client_hello("example.com");
        data[0] = 0x17;
        assert_eq!(extract_sni(&data), None);
    }

    #[test]
    fn extract_sni_not_client_hello() {
        // ServerHello (handshake type 0x02)
        let mut data = make_client_hello("example.com");
        data[5] = 0x02;
        assert_eq!(extract_sni(&data), None);
    }

    #[test]
    fn extract_sni_no_extensions() {
        let data = make_client_hello_no_sni();
        assert_eq!(extract_sni(&data), None);
    }

    #[test]
    fn extract_sni_http_request() {
        let data = b"GET / HTTP/1.1\r\nHost: example.com\r\n\r\n";
        assert_eq!(extract_sni(data), None);
    }

    #[test]
    fn extract_sni_garbage() {
        let data = [0xFF; 100];
        assert_eq!(extract_sni(&data), None);
    }

    #[test]
    fn extract_sni_truncated_hello() {
        let data = make_client_hello("example.com");
        // Truncate in the middle of the extensions
        let truncated = &data[..data.len() - 5];
        // Should still try to parse what it can -- may or may not find SNI
        // depending on where truncation hits. Main thing: no panic.
        let _ = extract_sni(truncated);
    }

    #[test]
    fn extract_sni_truncated_record_header() {
        let data = make_client_hello("example.com");
        // Only the TLS record header, no handshake data
        let _ = extract_sni(&data[..5]);
    }

    // -- Parse SNI extension edge cases --

    #[test]
    fn parse_sni_extension_empty() {
        assert_eq!(parse_sni_extension(&[]), None);
    }

    #[test]
    fn parse_sni_extension_too_short() {
        assert_eq!(parse_sni_extension(&[0x00]), None);
    }

    #[test]
    fn parse_sni_extension_non_hostname_type() {
        // Name type 1 (not host_name = 0)
        let mut data = Vec::new();
        data.extend_from_slice(&4u16.to_be_bytes()); // list length
        data.push(0x01); // name type = not host_name
        data.extend_from_slice(&1u16.to_be_bytes()); // name length
        data.push(b'x');
        assert_eq!(parse_sni_extension(&data), None);
    }
}
