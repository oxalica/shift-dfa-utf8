#![feature(array_chunks)]
#![feature(slice_split_once)]
#![feature(core_intrinsics)]
#![feature(select_unpredictable)]
#![expect(internal_features, reason = "TODO")]
use std::intrinsics::unlikely;

#[cfg(test)]
mod tests;

// TODO: For inspecting assembly.
#[no_mangle]
fn validate_utf8(bytes: &[u8]) -> Result<(), Utf8Error> {
    run_utf8_validation::<16, 16>(bytes)
}

#[derive(Debug, PartialEq)]
pub struct Utf8Error {
    pub valid_up_to: usize,
    pub error_len: Option<u8>,
}

// The shift-based DFA algorithm for UTF-8 validation.
// Ref: <https://gist.github.com/pervognsen/218ea17743e1442e59bb60d29b1aa725>
//
// In short, we encode DFA transitions in an array `TRANS_TABLE` such that:
// ```
// TRANS_TABLE[next_byte] =
//     OFFSET[target_state1] << OFFSET[source_state1] |
//     OFFSET[target_state2] << OFFSET[source_state2] |
//     ...
// ```
// Where `OFFSET[]` is a compile-time map from each state to a distinct 0..32 value.
//
// To execute the DFA:
// ```
// let state = OFFSET[initial_state];
// for byte in .. {
//     state = TRANS_TABLE[byte] >> (state & ((1 << BITS_PER_STATE) - 1));
// }
// ```
// By choosing `BITS_PER_STATE = 5` and `state: u32`, we can replace the masking by `wrapping_shr`
// and it becomes free on modern ISAs, including x86, x86_64 and ARM.
//
// ```
// // shrx state, qword ptr [table_addr + 8 * byte], state   # On x86-64-v3
// state = TRANS_TABLE[byte].wrapping_shr(state);
// ```
//
// The DFA is directly derived from UTF-8 syntax from the RFC3629:
// <https://datatracker.ietf.org/doc/html/rfc3629#section-4>.
// We assign S0 as ERROR and S1 as ACCEPT. DFA starts at S1.
// Syntax are annotated with DFA states in angle bracket as following:
//
// UTF8-char   = <S1> (UTF8-1 / UTF8-2 / UTF8-3 / UTF8-4)
// UTF8-1      = <S1> %x00-7F
// UTF8-2      = <S1> %xC2-DF                <S2> UTF8-tail
// UTF8-3      = <S1> %xE0                   <S3> %xA0-BF <S2> UTF8-tail /
//               <S1> (%xE1-EC / %xEE-EF)    <S4> 2( UTF8-tail ) /
//               <S1> %xED                   <S5> %x80-9F <S2> UTF8-tail
// UTF8-4      = <S1> %xF0    <S6> %x90-BF   <S4> 2( UTF8-tail ) /
//               <S1> %xF1-F3 <S7> UTF8-tail <S4> 2( UTF8-tail ) /
//               <S1> %xF4    <S8> %x80-8F   <S4> 2( UTF8-tail )
// UTF8-tail   = %x80-BF   # Inlined into above usages.
//
// You may notice that encoding 9 states with 5bits per state into 32bit seems impossible,
// but we exploit overlapping bits to find a possible `OFFSET[]` and `TRANS_TABLE[]` solution.
// The SAT solver to find such (minimal) solution is in `./solve_dfa.py`.
// The solution is also appended to the end of that file and is verifiable.
const BITS_PER_STATE: u32 = 5;
const STATE_MASK: u32 = (1 << BITS_PER_STATE) - 1;
const STATE_CNT: usize = 9;
const ST_ERROR: u32 = OFFSETS[0];
const ST_ACCEPT: u32 = OFFSETS[1];
// See the end of `./solve_dfa.py`.
const OFFSETS: [u32; STATE_CNT] = [0, 6, 16, 19, 1, 25, 11, 18, 24];

// Keep it in a single page.
#[repr(align(1024))]
struct TransitionTable([u32; 256]);

#[no_mangle]
static TRANS_TABLE: TransitionTable = {
    let mut table = [0u32; 256];
    let mut b = 0;
    while b < 256 {
        // See the end of `./solve_dfa.py`.
        table[b] = match b as u8 {
            0x00..=0x7F => 0x180,
            0xC2..=0xDF => 0x400,
            0xE0 => 0x4C0,
            0xE1..=0xEC | 0xEE..=0xEF => 0x40,
            0xED => 0x640,
            0xF0 => 0x2C0,
            0xF1..=0xF3 => 0x480,
            0xF4 => 0x600,
            0x80..=0x8F => 0x21060020,
            0x90..=0x9F => 0x20060820,
            0xA0..=0xBF => 0x860820,
            0xC0..=0xC1 | 0xF5..=0xFF => 0x0,
        };
        b += 1;
    }
    TransitionTable(table)
};

#[inline(always)]
const fn next_state(st: u32, byte: u8) -> u32 {
    TRANS_TABLE.0[byte as usize].wrapping_shr(st)
}

/// Check if `byte` is a valid UTF-8 first byte, assuming it must be a valid first or
/// continuation byte.
#[inline(always)]
const fn is_utf8_first_byte(byte: u8) -> bool {
    byte as i8 >= 0b1100_0000u8 as i8
}

/// # Safety
/// The caller must ensure `bytes[..i]` is a valid UTF-8 prefix and `st` is the DFA state after
/// executing on `bytes[..i]`.
#[inline]
const unsafe fn resolve_error_location(st: u32, bytes: &[u8], i: usize) -> (usize, u8) {
    // There are two cases:
    // 1. [valid UTF-8..] | *here
    //    The previous state must be ACCEPT for the case 1, and `valid_up_to = i`.
    // 2. [valid UTF-8..] | valid first byte, [valid continuation byte...], *here
    //    `valid_up_to` is at the latest non-continuation byte, which must exist and
    //    be in range `(i-3)..i`.
    if st & STATE_MASK == ST_ACCEPT {
        (i, 1)
    // SAFETY: UTF-8 first byte must exist if we are in an intermediate state.
    // We use pointer here because `get_unchecked` is not const fn.
    } else if is_utf8_first_byte(unsafe { bytes.as_ptr().add(i - 1).read() }) {
        (i - 1, 1)
    // SAFETY: Same as above.
    } else if is_utf8_first_byte(unsafe { bytes.as_ptr().add(i - 2).read() }) {
        (i - 2, 2)
    } else {
        (i - 3, 3)
    }
}

// The simpler but slower algorithm to run DFA with error handling.
//
// # Safety
// The caller must ensure `bytes[..i]` is a valid UTF-8 prefix and `st` is the DFA state after
// executing on `bytes[..i]`.
const unsafe fn run_with_error_handling(
    st: &mut u32,
    bytes: &[u8],
    mut i: usize,
) -> Result<(), Utf8Error> {
    while i < bytes.len() {
        let new_st = next_state(*st, bytes[i]);
        if unlikely(new_st & STATE_MASK == ST_ERROR) {
            // SAFETY: Guaranteed by the caller.
            let (valid_up_to, error_len) = unsafe { resolve_error_location(*st, bytes, i) };
            return Err(Utf8Error {
                valid_up_to,
                error_len: Some(error_len),
            });
        }
        *st = new_st;
        i += 1;
    }
    Ok(())
}

pub const fn run_utf8_validation_const(bytes: &[u8]) -> Result<(), Utf8Error> {
    let mut st = ST_ACCEPT;
    // SAFETY: Start at empty string with valid state ACCEPT.
    match unsafe { run_with_error_handling(&mut st, bytes, 0) } {
        Err(err) => Err(err),
        Ok(()) => {
            if st & STATE_MASK == ST_ACCEPT {
                Ok(())
            } else {
                // SAFETY: `st` is the last state after execution without encountering any error.
                let (valid_up_to, _) = unsafe { resolve_error_location(st, bytes, bytes.len()) };
                Err(Utf8Error {
                    valid_up_to,
                    error_len: None,
                })
            }
        }
    }
}

pub fn run_utf8_validation<const MAIN_CHUNK_SIZE: usize, const ASCII_CHUNK_SIZE: usize>(
    bytes: &[u8],
) -> Result<(), Utf8Error> {
    const { assert!(ASCII_CHUNK_SIZE % MAIN_CHUNK_SIZE == 0) };

    let mut st = ST_ACCEPT;
    let mut i = 0usize;

    while i + MAIN_CHUNK_SIZE <= bytes.len() {
        // Fast path: if the current state is ACCEPT, we can skip to the next non-ASCII chunk.
        // We also did a quick inspection on the first byte to avoid getting into this path at all
        // when handling strings with almost no ASCII, eg. Chinese scripts.
        // SAFETY: `i` is inbound.
        if st == ST_ACCEPT && unsafe { *bytes.get_unchecked(i) } < 0x80 {
            // SAFETY: `i` is inbound.
            let rest = unsafe { bytes.get_unchecked(i..) };
            let mut ascii_chunks = rest.array_chunks::<ASCII_CHUNK_SIZE>();
            let ascii_rest_chunk_cnt = ascii_chunks.len();
            let pos = ascii_chunks
                .position(|chunk| {
                    // NB. Always traverse the whole chunk to enable vectorization, instead of `.any()`.
                    // LLVM will be fear of memory traps and fallback if loop has short-circuit.
                    #[expect(clippy::unnecessary_fold)]
                    let has_non_ascii = chunk.iter().fold(false, |acc, &b| acc || (b >= 0x80));
                    has_non_ascii
                })
                .unwrap_or(ascii_rest_chunk_cnt);
            i += pos * ASCII_CHUNK_SIZE;
            if i + MAIN_CHUNK_SIZE > bytes.len() {
                break;
            }
        }

        // SAFETY: `i` and `i + MAIN_CHUNK_SIZE` are inbound by loop invariant.
        let chunk = unsafe { &*bytes.as_ptr().add(i).cast::<[u8; MAIN_CHUNK_SIZE]>() };
        let mut new_st = st;
        for &b in chunk {
            new_st = next_state(new_st, b);
        }
        if unlikely(new_st & STATE_MASK == ST_ERROR) {
            // Discard the current chunk erronous result, and reuse the trailing chunk handling to
            // report the error location.
            break;
        }

        st = new_st;
        i += MAIN_CHUNK_SIZE;
    }

    // SAFETY: `st` is the last state after executing `bytes[..i]` without encountering any error.
    unsafe { run_with_error_handling(&mut st, bytes, i)? };

    if unlikely(st & STATE_MASK != ST_ACCEPT) {
        // SAFETY: Same as above.
        let (valid_up_to, _) = unsafe { resolve_error_location(st, bytes, bytes.len()) };
        return Err(Utf8Error {
            valid_up_to,
            error_len: None,
        });
    }

    Ok(())
}
