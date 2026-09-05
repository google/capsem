//! The few bytes of DNS wire format the guest forwarder needs: enough of
//! the question section to tell whether an answer belongs to a query, and a
//! SERVFAIL for a query the host never answered.
//!
//! No general parser lives here on purpose. The host owns DNS semantics; the
//! guest only refuses to hand a client an answer to a different question.

/// A DNS message header is twelve bytes.
const HEADER_LEN: usize = 12;
/// RFC 1035 caps a label at 63 octets and a name at 255.
const MAX_LABEL: usize = 63;
const MAX_NAME: usize = 255;

/// The question one query asked, as the forwarder remembers it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Question {
    id: u16,
    opcode: u8,
    /// Lowercase wire labels, including lengths. A dot inside one label
    /// must not compare equal to a boundary between two labels.
    name: Vec<u8>,
    qtype: u16,
    qclass: u16,
    /// Offset one past the question section, for `servfail_for`.
    end: usize,
}

/// Parse the header id and first question of `msg`. `None` for anything
/// that is not a well-formed single-question message: a compression
/// pointer in the question (legitimate answers echo the question
/// verbatim), an over-long label or name, a truncated section, or a
/// question count other than one.
pub(crate) fn parse_question(msg: &[u8]) -> Option<Question> {
    if msg.len() < HEADER_LEN {
        return None;
    }
    let id = u16::from_be_bytes([msg[0], msg[1]]);
    let qdcount = u16::from_be_bytes([msg[4], msg[5]]);
    if qdcount != 1 {
        return None;
    }
    let mut pos = HEADER_LEN;
    let mut name = Vec::new();
    let mut name_len = 1usize; // Include the root label in the 255-byte limit.
    loop {
        let len = *msg.get(pos)? as usize;
        pos += 1;
        if len == 0 {
            break;
        }
        if len > MAX_LABEL {
            return None; // a compression pointer (>= 0xC0) or a junk length
        }
        name_len += len + 1;
        if name_len > MAX_NAME {
            return None;
        }
        let label = msg.get(pos..pos + len)?;
        name.push(len as u8);
        for byte in label {
            name.push(byte.to_ascii_lowercase());
        }
        pos += len;
    }
    let qtype = u16::from_be_bytes([*msg.get(pos)?, *msg.get(pos + 1)?]);
    let qclass = u16::from_be_bytes([*msg.get(pos + 2)?, *msg.get(pos + 3)?]);
    Some(Question {
        id,
        opcode: msg[2] & 0x78,
        name,
        qtype,
        qclass,
        end: pos + 4,
    })
}

impl Question {
    /// Does `datagram` answer this question: same transaction id, the
    /// response bit set, and the same name, type and class? Anything
    /// else is not this query's answer, whoever produced it.
    pub(crate) fn is_answered_by(&self, datagram: &[u8]) -> bool {
        if datagram.len() < HEADER_LEN || datagram[2] & 0x80 == 0 {
            return false;
        }
        match parse_question(datagram) {
            Some(answered) => {
                answered.id == self.id
                    && answered.opcode == self.opcode
                    && answered.name == self.name
                    && answered.qtype == self.qtype
                    && answered.qclass == self.qclass
            }
            None => false,
        }
    }
}

/// A SERVFAIL response to `query`, so a client whose query the host never
/// answered fails at once instead of sitting in its resolver's retransmit
/// timer. The header keeps the id, opcode and recursion-desired bit; the
/// question is echoed; no answer records are invented.
pub(crate) fn servfail_for(query: &[u8]) -> Option<Vec<u8>> {
    let question = parse_question(query)?;
    let mut out = Vec::with_capacity(question.end);
    out.extend_from_slice(&query[..question.end]);
    out[2] = (query[2] & 0x79) | 0x80; // QR set, opcode and RD kept
    out[3] = 0x02; // SERVFAIL, no AA/TC/RA
    out[6..12].fill(0); // no answer, authority or additional records
    Some(out)
}

/// Query and answer builders shared by the forwarder's test modules.
#[cfg(test)]
pub(crate) mod fixtures {
    /// A query for `name` (A, IN) with transaction id `id`.
    pub(crate) fn query(id: u16, name: &str) -> Vec<u8> {
        let mut msg = vec![0u8; 12];
        msg[0..2].copy_from_slice(&id.to_be_bytes());
        msg[2] = 0x01; // RD
        msg[5] = 1; // qdcount
        for label in name.trim_end_matches('.').split('.') {
            msg.push(label.len() as u8);
            msg.extend_from_slice(label.as_bytes());
        }
        msg.push(0);
        msg.extend_from_slice(&[0, 1, 0, 1]); // A, IN
        msg
    }

    /// `query` turned into a response: QR set, one answer record claimed.
    pub(crate) fn answer_to(query: &[u8]) -> Vec<u8> {
        let mut msg = query.to_vec();
        msg[2] |= 0x80;
        msg[7] = 1;
        msg
    }
}

#[cfg(test)]
#[path = "wire/tests.rs"]
mod tests;
