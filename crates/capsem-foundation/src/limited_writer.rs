//! Enforce an output byte ceiling during writing, without a sizing pass.

use std::io::{self, Write};

pub struct LimitedWriter<W> {
    inner: W,
    remaining: usize,
    exceeded: bool,
}

impl<W> LimitedWriter<W> {
    pub fn new(inner: W, limit: usize) -> Self {
        Self {
            inner,
            remaining: limit,
            exceeded: false,
        }
    }

    pub fn exceeded(&self) -> bool {
        self.exceeded
    }

    pub fn remaining(&self) -> usize {
        self.remaining
    }
}

impl<W: Write> Write for LimitedWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.remaining {
            self.exceeded = true;
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "encoded output exceeds byte limit",
            ));
        }
        let written = self.inner.write(bytes)?;
        self.remaining -= written;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests;
