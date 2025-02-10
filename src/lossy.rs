use std::str::from_utf8_unchecked;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Utf8Chunk<'a> {
    pub valid: &'a str,
    pub invalid: &'a [u8],
}

pub fn to_utf8_chunks<const MAIN_CHUNK_SIZE: usize, const ASCII_CHUNK_SIZE: usize>(
    source: &[u8],
) -> Utf8Chunks<'_, MAIN_CHUNK_SIZE, ASCII_CHUNK_SIZE> {
    Utf8Chunks { source }
}

#[derive(Clone)]
pub struct Utf8Chunks<'a, const MAIN_CHUNK_SIZE: usize, const ASCII_CHUNK_SIZE: usize> {
    source: &'a [u8],
}

impl<'a, const MAIN_CHUNK_SIZE: usize, const ASCII_CHUNK_SIZE: usize> Iterator
    for Utf8Chunks<'a, MAIN_CHUNK_SIZE, ASCII_CHUNK_SIZE>
{
    type Item = Utf8Chunk<'a>;

    fn next(&mut self) -> Option<Utf8Chunk<'a>> {
        if self.source.is_empty() {
            return None;
        }

        match super::run_utf8_validation::<MAIN_CHUNK_SIZE, ASCII_CHUNK_SIZE>(self.source) {
            Ok(()) => {
                // Truncate the slice, no need to touch the pointer.
                self.source = &self.source[..0];
                Some(Utf8Chunk {
                    // SAFETY: `source` is valid UTF-8.
                    valid: unsafe { std::str::from_utf8_unchecked(self.source) },
                    invalid: &[],
                })
            }
            Err(err) => {
                let valid_up_to = err.valid_up_to();
                let error_len = err.error_len().unwrap_or(self.source.len() - valid_up_to);
                // SAFETY: `valid_up_to` is the valid UTF-8 string length, so is in bound.
                let (valid, remaining) = unsafe { self.source.split_at_unchecked(valid_up_to) };
                let (invalid, after_invalid) = unsafe { remaining.split_at_unchecked(error_len) };
                self.source = after_invalid;
                Some(Utf8Chunk {
                    // SAFETY: All bytes up to `valid_up_to` are valid UTF-8.
                    valid: unsafe { from_utf8_unchecked(valid) },
                    invalid,
                })
            }
        }
    }
}
