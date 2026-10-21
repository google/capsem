//! Keep split UTF-8 completion separate from subsequent bytes. vte 0.15's
//! partial-codepoint path can consume following ASCII without dispatching it
//! when its four-byte buffer also starts the next codepoint. Bytes still reach
//! the engine immediately and unchanged; only the parser call boundaries move.
#[derive(Default)]
pub(super) struct Utf8Boundary {
    partial: [u8; 3],
    len: usize,
}

impl Utf8Boundary {
    pub fn process(&mut self, mut bytes: &[u8], mut emit: impl FnMut(&[u8])) {
        while self.len != 0 && !bytes.is_empty() {
            let mut prefix = [0; 4];
            prefix[..self.len].copy_from_slice(&self.partial[..self.len]);
            prefix[self.len] = bytes[0];
            emit(&bytes[..1]);
            self.remember(incomplete_tail(&prefix[..=self.len]));
            bytes = &bytes[1..];
        }
        if !bytes.is_empty() {
            emit(bytes);
            self.remember(incomplete_tail(bytes));
        }
    }

    fn remember(&mut self, tail: &[u8]) {
        self.partial[..tail.len()].copy_from_slice(tail);
        self.len = tail.len();
    }
}

fn incomplete_tail(bytes: &[u8]) -> &[u8] {
    // An incomplete UTF-8 scalar occupies at most three bytes. Inspecting the
    // suffix keeps the normal path allocation-free and independent of batch size.
    let offset = bytes.len().saturating_sub(3);
    let Some(start) = bytes[offset..].iter().rposition(|byte| byte & 0xc0 != 0x80) else {
        return &[];
    };
    let tail = &bytes[offset + start..];
    match std::str::from_utf8(tail) {
        Err(error) if error.error_len().is_none() => tail,
        _ => &[],
    }
}

#[cfg(test)]
mod tests;
