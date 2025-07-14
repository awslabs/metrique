// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::io;

use smallvec::SmallVec;

// Utility for string buffers that always have the same prefix
#[derive(Debug, Clone)]
pub(super) struct PrefixedStringBuf {
    prefix_len: usize,
    buf: String,
}

impl PrefixedStringBuf {
    pub fn new(prefix: &str, capacity: usize) -> Self {
        let prefix_len = prefix.len();
        let mut buf = String::with_capacity(capacity);
        buf.push_str(prefix);
        Self { prefix_len, buf }
    }

    pub fn from_prefix(buf: String) -> Self {
        Self {
            prefix_len: buf.len(),
            buf,
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.buf.len() == self.prefix_len
    }

    pub fn clear(&mut self) {
        self.buf.truncate(self.prefix_len);
        // don't let a large entry cause us to hold on to many MB of buffer data forever!
        self.buf.shrink_to(1024 * 1024);
    }

    #[inline]
    pub fn push(&mut self, c: char) -> &mut Self {
        self.buf.push(c);
        self
    }

    #[inline]
    pub fn push_raw_str<'a>(&'a mut self, s: &str) -> &'a mut Self {
        self.buf.push_str(s);
        self
    }

    #[inline]
    pub fn push_integer(&mut self, s: impl itoa::Integer) -> &mut Self {
        self.buf.push_str(itoa::Buffer::new().format(s));
        self
    }

    #[inline]
    pub fn extend_from_within_range(&mut self, start: usize, end: usize) -> &mut Self {
        self.buf.extend_from_within(start..end);
        self
    }

    pub fn as_str(&self) -> &str {
        &self.buf
    }
}

impl crate::json_string::JsonString for PrefixedStringBuf {
    fn json_string(&mut self, value: &str) -> &mut Self {
        crate::json_string::JsonString::json_string(&mut self.buf, value);
        self
    }
}

impl AsRef<[u8]> for PrefixedStringBuf {
    fn as_ref(&self) -> &[u8] {
        self.buf.as_bytes()
    }
}

// We'll use vectored IO mainly to avoid an extra mem copy when writing to an output. In most cases, formats will be
// attached to a buffered writer or some compression codec. If we have several buffers that need to be concatenated,
// using the vector IO interface will concatenate them into the downstream buffer rather than needing to merge them into
// some temporary buffer.

// Note that this is mostly copied as is from the std library. We need this until
// https://doc.rust-lang.org/nightly/src/std/io/mod.rs.html#1732 is stable. No dark magic is occurring, just the current
// IoVec type doesn't expose the underying &[u8] with the right lifetime through a fn yet.

pub(crate) fn write_all_vectored<V: AsRef<[u8]>, const N: usize>(
    bufs: SmallVec<[V; N]>,
    output: &mut impl io::Write,
) -> io::Result<()> {
    // Only a debug assert because this will still work with bufs.len() > N, but to avoid heap allocations, we should
    // avoid it.
    debug_assert!(!bufs.is_empty() && bufs.len() <= N);

    let mut slices: SmallVec<[_; N]> = bufs.iter().map(AsRef::as_ref).collect();
    let mut slices = &mut slices[..];

    // Until the IoSlice APIs are expanded, there's no way to get back a &'a [u8]. We'll reconstruct the slices on
    // each loop attempt
    let mut io_slices = SmallVec::<[io::IoSlice<'_>; N]>::new();

    advance_slices(&mut slices, 0); // this clears out any empty slices to prevent a write([]) call
    while !slices.is_empty() {
        io_slices.extend(slices.iter().map(|&s| io::IoSlice::new(s)));
        match output.write_vectored(&io_slices) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => advance_slices(&mut slices, n),
            Err(ref e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
        io_slices.clear();
    }

    Ok(())
}

// Also copied out of std. This "advances" the slices forward by count bytes. If all of the bytes were written
// (count == sum(len(s) for s in slices)), then slices will be empty after. Some examples:
//  * advance(&mut [&[0, 1, 2]], 2) will cause slices to point to &mut [&[2]]
//  * advance(&mut [&[0, 1], &[2, 3]], 3) will cause slices to point to &mut [&[3]]
//
// We need this fn to handle when write_vectored doesn't write out the entire set of slices given to it.

fn advance_slices(slices: &mut &mut [&[u8]], count: usize) {
    let mut remaining = count;

    while let Some(first) = slices.first_mut() {
        if let Some(remainder) = remaining.checked_sub(first.len()) {
            *slices = &mut std::mem::take(slices)[1..];
            remaining = remainder;
        } else {
            *first = &first[remaining..];
            return;
        }
    }

    assert_eq!(remaining, 0);
}

#[cfg(test)]
mod test {
    use super::PrefixedStringBuf;

    #[test]
    fn test_extend_from_within() {
        let mut buf = PrefixedStringBuf::from_prefix("0123".into());
        buf.push_raw_str("4567")
            .extend_from_within_range(0, 2)
            .extend_from_within_range(2, 8)
            .extend_from_within_range(0, 0);
        assert_eq!(buf.as_str(), "0123456701234567");
    }
}
