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

const BITS_PER_STATE: u32 = 6;
const STATE_MASK: u32 = (1 << BITS_PER_STATE) - 1;
const STATE_CNT: usize = 10;
#[allow(clippy::all)]
const ST_ERROR: u32 = 0 * BITS_PER_STATE as u32;
#[allow(clippy::all)]
const ST_ACCEPT: u32 = 1 * BITS_PER_STATE as u32;
// The only states that are after eating 2 bytes. All other intermediate states (other than ERROR
// and ACCEPT) are after eating 1 byte.
const ST_EAT_2BYTES_1: u32 = 4 * BITS_PER_STATE;
const ST_EAT_2BYTES_2: u32 = 9 * BITS_PER_STATE;

// The transition table of shift-based DFA for UTF-8 validation.
// Ref: <https://gist.github.com/pervognsen/218ea17743e1442e59bb60d29b1aa725>
//
// In short, we encode DFA transitions in an array `DFA_TRANS` such that:
// ```
// DFA_TRANS[next_byte] =
//     (target_state1 * BITS_PER_STATE) << (source_state1 * BITS_PER_STATE) |
//     (target_state2 * BITS_PER_STATE) << (source_state2 * BITS_PER_STATE) |
//     ...
// ```
// Thanks to pre-multiplication, we can execute the DFA with one statement per byte:
// ```
// let state = initial_state * BITS_PER_STATE;
// for byte in .. {
//     state = DFA_TRANS[byte] >> (state & ((1 << BITS_PER_STATE) - 1));
// }
// ```
// By choosing `BITS_PER_STATE = 6` and `state: u64`, we can replace the masking by `wrapping_shr`.
// ```
// // shrx state, qword ptr [table_addr + 8 * byte], state   # On x86-64-v3
// state = DFA_TRANS[byte].wrapping_shr(state);
// ```
//
// On platform without 64-bit shift, especially i686, we split the `u64` next-state into
// `[u32; 2]`, and each `u32` stores 5 * BITS_PER_STATE = 30 bits. In this way, state transition
// can be done in only 32-bit shifts and a conditional move, which is several times faster
// (in latency) than ordinary 64-bit shift (SHRD).
//
// The DFA is directly derived from UTF-8 syntax from the RFC <https://tools.ietf.org/html/rfc3629>.
// We assign S0 as ERROR and S1 as ACCEPT. DFA starts at S1.
// Syntax are annotated with DFA states in angle bracket as following:
//
// UTF8-1      = <S1> %x00-7F
// UTF8-2      = <S1> %xC2-DF <S2> UTF8-tail
// UTF8-3      = <S1> %xE0 <S3> %xA0-BF <S9> UTF8-tail /
//               <S1> (%xE1-EC / %xEE-EF) <S4> 2( UTF8-tail ) /
//               <S1> %xED <S5> %x80-9F <S9> UTF8-tail /
// UTF8-4      = <S1> %xF0 <S6> %x90-BF <S4> 2( UTF8-tail ) /
//               <S1> %xF4 <S7> %x80-8F <S4> 2( UTF8-tail ) /
//               <S1> %xF1-F3 <S8> UTF8-tail <S9> 2( UTF8-tail )
//
// UTF8-tail   = %x80-BF   // Inlined into above usages.
static DFA_TRANS: [u64; 256] = {
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
            0xA0..=0xBF => 9,
            _ => 0,
        };
        to[4] = match b {
            0x80..=0xBF => 2,
            _ => 0,
        };
        to[5] = match b {
            0x80..=0x9F => 9,
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
            0x80..=0xBF => 9,
            _ => 0,
        };
        to[9] = to[2];

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

        table[b] = bits;
        b += 1;
    }
    table
};

/// Bytes between the current state and the latest ACCEPT before.
/// Invariant: the argument must be a valid non-ERROR state.
#[inline]
fn eaten_len_before_state(st: u32) -> usize {
    match st & STATE_MASK {
        ST_ACCEPT => 0,
        ST_EAT_2BYTES_1 | ST_EAT_2BYTES_2 => 2,
        _ => 1,
    }
}

#[cfg(not(feature = "shift32"))]
#[inline(always)]
fn next_state(st: u32, byte: u8) -> u32 {
    DFA_TRANS[byte as usize].wrapping_shr(st as _) as _
}

#[cfg(feature = "shift32")]
#[inline(always)]
fn next_state(st: u32, byte: u8) -> u32 {
    // SAFETY: `u64` is more aligned than `u32`, and has the same repr as `[u32; 2]`.
    let [lo, hi] = unsafe { std::mem::transmute::<u64, [u32; 2]>(DFA_TRANS[byte as usize]) };
    #[cfg(target_endian = "big")]
    let (lo, hi) = (hi, lo);
    (st & 32 == 0).select_unpredictable(lo, hi).wrapping_shr(st)
}

fn run_with_error_handling(st: &mut u32, prefix_len: usize, chunk: &[u8]) -> Result<(), Utf8Error> {
    for (i, b) in chunk.iter().enumerate() {
        let new_st = next_state(*st, *b);
        if unlikely(new_st & STATE_MASK == ST_ERROR) {
            return Err(Utf8Error {
                valid_up_to: prefix_len + i - eaten_len_before_state(*st),
                error_len: Some(1),
            });
        }
        *st = new_st;
    }
    Ok(())
}

pub fn run_utf8_validation<const MAIN_CHUNK_SIZE: usize, const ASCII_CHUNK_SIZE: usize>(
    bytes: &[u8],
) -> Result<(), Utf8Error> {
    // // Some sane main loop chunk size.
    // // This should also be small enough to fully unroll the inner loop on DFA path.
    // const MAIN_CHUNK_SIZE: usize = 16;

    // // Chunk size of bulk ASCII skip path. Must be multiple or main chunk size.
    // const ASCII_CHUNK_SIZE: usize = 32;
    const { assert!(ASCII_CHUNK_SIZE % MAIN_CHUNK_SIZE == 0) };

    let mut st = ST_ACCEPT;
    let mut i = 0usize;

    while i + MAIN_CHUNK_SIZE <= bytes.len() {
        // Fast path: if the current state is ACCEPT, we can skip to the next non-ASCII chunk.
        if st == ST_ACCEPT {
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

        let mut new_st = st;
        let chunk = unsafe { &*bytes.as_ptr().add(i).cast::<[u8; MAIN_CHUNK_SIZE]>() };
        for &b in chunk {
            new_st = next_state(new_st, b);
        }
        if unlikely(new_st & STATE_MASK == ST_ERROR) {
            return run_with_error_handling(&mut st, i, chunk);
        }

        st = new_st;
        i += MAIN_CHUNK_SIZE;
    }

    let tail_chunk = unsafe { bytes.get_unchecked(i..) };
    run_with_error_handling(&mut st, i, tail_chunk)?;

    if unlikely(st & STATE_MASK != ST_ACCEPT) {
        return Err(Utf8Error {
            valid_up_to: bytes.len() - eaten_len_before_state(st),
            error_len: None,
        });
    }

    Ok(())
}
