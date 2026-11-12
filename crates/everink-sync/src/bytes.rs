//! Reading and writing the canonical bytes of objects. One value has exactly one encoding, so two copies can be
//! compared by comparing bytes.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    Truncated,
    /// A number written in more bytes than it needs.
    NotCanonical,
    TrailingBytes,
    /// A value that is well formed but not allowed; the text says which.
    Invalid(&'static str),
    Position(everink_core::DecodeError),
}

pub(crate) struct Writer {
    pub out: Vec<u8>,
}

impl Writer {
    pub fn new() -> Writer {
        Writer { out: Vec::new() }
    }

    pub fn u8(&mut self, v: u8) {
        self.out.push(v);
    }

    pub fn bool(&mut self, v: bool) {
        self.out.push(u8::from(v));
    }

    pub fn varint(&mut self, mut v: u64) {
        while v >= 0x80 {
            self.out.push((v as u8) | 0x80);
            v >>= 7;
        }
        self.out.push(v as u8);
    }

    pub fn raw(&mut self, b: &[u8]) {
        self.out.extend_from_slice(b);
    }

    /// A length, then the bytes.
    pub fn blob(&mut self, b: &[u8]) {
        self.varint(b.len() as u64);
        self.raw(b);
    }

    pub fn i32(&mut self, v: i32) {
        self.raw(&v.to_le_bytes());
    }

    pub fn u32(&mut self, v: u32) {
        self.raw(&v.to_le_bytes());
    }
}

pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Reader<'a> {
        Reader { bytes, at: 0 }
    }

    pub fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let end = self.at.checked_add(n).ok_or(DecodeError::Truncated)?;
        let s = self.bytes.get(self.at..end).ok_or(DecodeError::Truncated)?;
        self.at = end;
        Ok(s)
    }

    pub fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }

    pub fn bool(&mut self) -> Result<bool, DecodeError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(DecodeError::Invalid("a flag that is not 0 or 1")),
        }
    }

    pub fn varint(&mut self) -> Result<u64, DecodeError> {
        let mut v: u64 = 0;
        for i in 0..10 {
            let b = self.u8()?;
            let part = u64::from(b & 0x7f);
            if i == 9 && part > 1 {
                return Err(DecodeError::NotCanonical); // would not fit 64 bits
            }
            v |= part << (7 * i);
            if b & 0x80 == 0 {
                if i > 0 && b == 0 {
                    return Err(DecodeError::NotCanonical); // written longer than needed
                }
                return Ok(v);
            }
        }
        Err(DecodeError::NotCanonical)
    }

    pub fn varint_u32(&mut self) -> Result<u32, DecodeError> {
        u32::try_from(self.varint()?).map_err(|_| DecodeError::Invalid("a number above 32 bits"))
    }

    /// A length, then the bytes. The length is checked against `max` before anything is taken.
    pub fn blob(&mut self, max: usize) -> Result<&'a [u8], DecodeError> {
        let len = self.varint()?;
        if len > max as u64 {
            return Err(DecodeError::Invalid("a field longer than its limit"));
        }
        self.take(len as usize)
    }

    pub fn array<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        Ok(self.take(N)?.try_into().expect("N bytes were taken"))
    }

    pub fn i32(&mut self) -> Result<i32, DecodeError> {
        Ok(i32::from_le_bytes(self.array()?))
    }

    pub fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    /// Bytes not yet read.
    pub fn left(&self) -> usize {
        self.bytes.len() - self.at
    }

    pub fn is_empty(&self) -> bool {
        self.at == self.bytes.len()
    }

    pub fn finish(&self) -> Result<(), DecodeError> {
        if self.is_empty() {
            Ok(())
        } else {
            Err(DecodeError::TrailingBytes)
        }
    }
}
