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

// The transition table of shift-based DFA for UTF-8 validation.
// Ref: <https://gist.github.com/pervognsen/218ea17743e1442e59bb60d29b1aa725>
//
// In short, we encode DFA transitions in an array `TRANS_TABLE` such that:
// ```
// TRANS_TABLE[next_byte] =
//     (target_state1 * BITS_PER_STATE) << (source_state1 * BITS_PER_STATE) |
//     (target_state2 * BITS_PER_STATE) << (source_state2 * BITS_PER_STATE) |
//     ...
// ```
// Thanks to pre-multiplication, we can execute the DFA with one statement per byte:
// ```
// let state = initial_state * BITS_PER_STATE;
// for byte in .. {
//     state = TRANS_TABLE[byte] >> (state & ((1 << BITS_PER_STATE) - 1));
// }
// ```
// By choosing `BITS_PER_STATE = 6` and `state: u64`, we can replace the masking by `wrapping_shr`.
// ```
// // shrx state, qword ptr [table_addr + 8 * byte], state   # On x86-64-v3
// state = TRANS_TABLE[byte].wrapping_shr(state);
// ```
//
// On platform without 64-bit shift, especially i686, we split the `u64` next-state into
// `[u32; 2]`, and each `u32` stores 5 * BITS_PER_STATE = 30 bits. In this way, state transition
// can be done in only 32-bit shifts and a conditional move, which is several times faster
// (in latency) than ordinary 64-bit shift (SHRD).
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
//               <S1> %xF4    <S7> %x80-8F   <S4> 2( UTF8-tail ) /
//               <S1> %xF1-F3 <S8> UTF8-tail <S4> 2( UTF8-tail )
//
// UTF8-tail   = %x80-BF   # Inlined into above usages.
const BITS_PER_STATE: u32 = 6;
const STATE_MASK: u32 = (1 << BITS_PER_STATE) - 1;
const STATE_CNT: usize = 9;
#[allow(clippy::all)]
const ST_ERROR: u32 = 0 * BITS_PER_STATE as u32;
#[allow(clippy::all)]
const ST_ACCEPT: u32 = 1 * BITS_PER_STATE as u32;

// After storing STATE_CNT * BITS_PER_STATE = 54bits on 64-bit platform, or (STATE_CNT - 5)
// * BITS_PER_STATE = 24bits on 32-bit platform, we still have some high bits left.
// They will never be used via state transition.
// We merge lookup table from first byte -> UTF-8 length, to these highest bits.
const UTF8_LEN_HIBITS: u32 = 4;

static TRANS_TABLE: [u64; 256] = {
    let mut table = [0u64; 256];
    let mut b = 0;
    while b < 256 {
        // Target states indexed by starting states.
        let mut to = [0u64; STATE_CNT];
        to[0] = 0;
        to[1] = match b {
            0x00..=0x7F => 1,
            0xC2..=0xDF => 2,
            0xE0 => 3,
            0xE1..=0xEC | 0xEE..=0xEF => 4,
            0xED => 5,
            0xF0 => 6,
            0xF4 => 7,
            0xF1..=0xF3 => 8,
            _ => 0,
        };
        to[2] = match b {
            0x80..=0xBF => 1,
            _ => 0,
        };
        to[3] = match b {
            0xA0..=0xBF => 2,
            _ => 0,
        };
        to[4] = match b {
            0x80..=0xBF => 2,
            _ => 0,
        };
        to[5] = match b {
            0x80..=0x9F => 2,
            _ => 0,
        };
        to[6] = match b {
            0x90..=0xBF => 4,
            _ => 0,
        };
        to[7] = match b {
            0x80..=0x8F => 4,
            _ => 0,
        };
        to[8] = match b {
            0x80..=0xBF => 4,
            _ => 0,
        };

        // On platforms without 64-bit shift, align states 5..10 to 32-bit boundary.
        // See docs above for details.
        let need_align = cfg!(feature = "shift32");
        let mut bits = 0u64;
        let mut j = 0;
        while j < to.len() {
            let to_off =
                to[j] * BITS_PER_STATE as u64 + if need_align && to[j] >= 5 { 2 } else { 0 };
            let off = j as u32 * BITS_PER_STATE + if need_align && j >= 5 { 2 } else { 0 };
            bits |= to_off << off;
            j += 1;
        }

        let utf8_len = match b {
            0x00..=0x7F => 1,
            0xC2..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF4 => 4,
            _ => 0,
        };
        bits |= utf8_len << (64 - UTF8_LEN_HIBITS);

        table[b] = bits;
        b += 1;
    }
    table
};

#[cfg(not(feature = "shift32"))]
#[inline(always)]
const fn next_state(st: u32, byte: u8) -> u32 {
    TRANS_TABLE[byte as usize].wrapping_shr(st as _) as _
}

#[cfg(feature = "shift32")]
#[inline(always)]
const fn next_state(st: u32, byte: u8) -> u32 {
    // SAFETY: `u64` is more aligned than `u32`, and has the same repr as `[u32; 2]`.
    let [lo, hi] = unsafe { std::mem::transmute::<u64, [u32; 2]>(TRANS_TABLE[byte as usize]) };
    #[cfg(target_endian = "big")]
    let (lo, hi) = (hi, lo);
    if st & 32 == 0 { lo } else { hi }.wrapping_shr(st)
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
        if st == ST_ACCEPT {
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

#[no_mangle]
pub const fn utf8_char_width(b: u8) -> usize {
    // On 32-bit platforms, optimizer is smart enough to only load and operate on the high 32-bits.
    (TRANS_TABLE[b as usize] >> (64 - UTF8_LEN_HIBITS)) as usize
}
