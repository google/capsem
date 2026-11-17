use std::io::{self, BufRead, Write};

use capsem_proto::framing::FinalBuffer;

pub enum LineRead {
    Message(Vec<u8>),
    TooLarge,
    End,
}

pub struct McpLines<R> {
    reader: R,
    limit: usize,
    discarding: bool,
}

impl<R: BufRead> McpLines<R> {
    pub fn new(reader: R, limit: usize) -> Self {
        Self {
            reader,
            limit,
            discarding: false,
        }
    }

    pub fn next_line(&mut self) -> io::Result<LineRead> {
        let mut line = Vec::new();
        loop {
            let chunk = self.reader.fill_buf()?;
            if chunk.is_empty() {
                self.discarding = false;
                return Ok(if line.len() > self.limit {
                    LineRead::TooLarge
                } else if line.is_empty() {
                    LineRead::End
                } else {
                    LineRead::Message(line)
                });
            }
            let newline = chunk.iter().position(|byte| *byte == b'\n');
            let count = newline.unwrap_or(chunk.len());
            if self.discarding {
                self.reader.consume(count + usize::from(newline.is_some()));
                self.discarding = newline.is_none();
                continue;
            }
            let bytes = &chunk[..count];
            let new_len = line.len().checked_add(bytes.len());
            let trailing_cr = bytes.last().or_else(|| line.last()) == Some(&b'\r');
            let allowed = self.limit.saturating_add(usize::from(trailing_cr));
            if new_len.is_none_or(|len| len > allowed) {
                self.reader.consume(count + usize::from(newline.is_some()));
                self.discarding = newline.is_none();
                return Ok(LineRead::TooLarge);
            }
            FinalBuffer::new(&mut line, allowed).write_all(bytes)?;
            self.reader.consume(count + usize::from(newline.is_some()));
            if newline.is_some() {
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                return Ok(LineRead::Message(line));
            }
        }
    }
}

#[cfg(test)]
#[path = "lines/tests.rs"]
mod tests;
