#![feature(array_chunks)]
#![feature(slice_split_once)]
#![feature(core_intrinsics)]
#![feature(select_unpredictable)]
#![feature(const_eval_select)]
#![expect(internal_features, reason = "TODO")]

#[cfg(test)]
mod tests;

pub mod lossy;

#[derive(Debug, PartialEq)]
pub struct Utf8Error {
    pub valid_up_to: usize,
    // Use a single value instead of tagged enum `Option<u8>` to make `Result<(), Utf8Error>` fits
    // in two machine words, so `run_utf8_validation` does not need to returns values on stack on
    // x86(_64). Register spill is very expensive on `run_utf8_validation` and can give up to 200%
    // latency penalty on the error path.
    pub error_len: Utf8ErrorLen,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u8)]
pub enum Utf8ErrorLen {
    Eof = 0,
    One,
    Two,
    Three,
}

impl Utf8Error {
    #[inline]
    pub fn error_len(&self) -> Option<usize> {
        match self.error_len {
            Utf8ErrorLen::Eof => None,
            // See: <https://github.com/rust-lang/rust/issues/136972>
            len => Some(len as usize),
        }
    }

    pub fn valid_up_to(&self) -> usize {
        self.valid_up_to
    }
}

// TODO: For inspecting assembly.
#[no_mangle]
pub const fn from_utf8(bytes: &[u8]) -> Result<&str, Utf8Error> {
    let ret = std::intrinsics::const_eval_select(
        (bytes,),
        run_utf8_validation_const,
        run_utf8_validation::<16, 16>,
    );
    match ret {
        // SAFETY: Verified.
        Ok(()) => Ok(unsafe { std::str::from_utf8_unchecked(bytes) }),
        Err(err) => Err(err),
    }
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
// // On x86-64-v3: (more instructions on ordinary x86_64 but with same cycles-per-byte)
// //   shrx state, qword ptr [TRANS_TABLE + 4 * byte], state
// // On aarch64/ARMv8:
// //   ldr temp, [TRANS_TABLE, byte, lsl 2]
// //   lsr state, temp, state
// state = TRANS_TABLE[byte].wrapping_shr(state);
// ```
//
// The DFA is directly derived from UTF-8 syntax from the RFC3629
// <https://datatracker.ietf.org/doc/html/rfc3629#section-4>,
// by assigning states between bytes.
// We assign S0 as ERROR and S1 as ACCEPT. DFA starts at S1.
// Syntax are annotated with DFA states in angle bracket as following:
//
// UTF8-char   = <S1> (UTF8-1 / UTF8-2 / UTF8-3 / UTF8-4)
// UTF8-1      = <S1> %x00-7F
// UTF8-2      = <S1> %xC2-DF             <S2> UTF8-tail
// UTF8-3      = <S1> %xE0                <S3> %xA0-BF   <S2> UTF8-tail /
//               <S1> (%xE1-EC / %xEE-EF) <S4> UTF8-tail <S2> UTF8-tail /
//               <S1> %xED                <S5> %x80-9F   <S2> UTF8-tail
// UTF8-4      = <S1> %xF0                <S6> %x90-BF   <S9> UTF8-tail <S2> UTF8-tail /
//               <S1> %xF1-F3             <S7> UTF8-tail <S9> UTF8-tail <S2> UTF8-tail /
//               <S1> %xF4                <S8> %x80-8F   <S9> UTF8-tail <S2> UTF8-tail
// UTF8-tail   = %x80-BF   # Inlined into above usages.
//
// S9 and S4 are really the same state, but splited for `error_len` calculation because they have
// different prefix lengths. See details in `resolve_error_location`.
//
// You may notice that encoding 9 states with 5bits per state into 32bit seems impossible,
// but we exploit overlapping bits to find a possible `OFFSET[]` and `TRANS_TABLE[]` solution.
// The SAT solver to find such (minimal) solution is in `./solve_dfa.py`.
// The solution is also appended to the end of that file and is verifiable.
const BITS_PER_STATE: u32 = 5;
const STATE_MASK: u32 = (1 << BITS_PER_STATE) - 1;
const STATE_CNT: usize = 10;
const ST_ERROR: u32 = OFFSETS[0];
const ST_ACCEPT: u32 = OFFSETS[1];
// See the end of `./solve_dfa.py`.
const OFFSETS: [u32; STATE_CNT] = [0, 6, 16, 19, 13, 25, 11, 18, 24, 1];
const OFFSET_ERROR_LEN_DISCR: u32 = 25;
const CVT_ERROR_LEN: u32 = 0x30302;

// Keep it in a single page.
#[repr(align(1024))]
struct TransitionTable([u32; 256]);

static TRANS_TABLE: TransitionTable = {
    let mut table = [0u32; 256];
    let mut b = 0;
    while b < 256 {
        // See the end of `./solve_dfa.py`.
        table[b] = match b as u8 {
            0x00..=0x7F => 0x180,
            0xC2..=0xDF => 0x80000400,
            0xE0 => 0x4C0,
            0xE1..=0xEC | 0xEE..=0xEF => 0x340,
            0xED => 0x640,
            0xF0 => 0x2C0,
            0xF1..=0xF3 => 0x480,
            0xF4 => 0x600,
            0x80..=0x8F => 0x21060020,
            0x90..=0x9F => 0x20060820,
            0xA0..=0xBF => 0x40860820,
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

// # Safety
// `full_st` must be the last non-ERROR state after transition, without masking.
#[inline]
const unsafe fn resolve_error_location(full_st: u32, mut i: usize) -> Utf8Error {
    let st = full_st & STATE_MASK;
    // S2 and S9 require inspecting higher bits to decide prefix length.
    let error_len_s2_s9 = CVT_ERROR_LEN.wrapping_shr(full_st >> OFFSET_ERROR_LEN_DISCR) as u8 & 3;
    i += (st == ST_ACCEPT) as usize;
    let error_len = if st == OFFSETS[2] || st == OFFSETS[9] {
        error_len_s2_s9
    } else {
        1
    };
    Utf8Error {
        valid_up_to: i - error_len as usize,
        error_len: std::mem::transmute::<u8, Utf8ErrorLen>(error_len),
    }
}

// The simpler but slower algorithm to run DFA with error handling.
// Returns the final state after execution on the whole slice.
//
// # Safety
// The caller must ensure `bytes[..i]` is a valid UTF-8 prefix and `st` is the DFA state after
// executing on `bytes[..i]`.
#[inline]
const unsafe fn run_with_error_handling(
    mut st: u32,
    bytes: &[u8],
    mut i: usize,
) -> Result<u32, Utf8Error> {
    while i < bytes.len() {
        let new_st = next_state(st, bytes[i]);
        if new_st & STATE_MASK == ST_ERROR {
            // SAFETY: Guaranteed by the caller.
            return Err(unsafe { resolve_error_location(st, i) });
        }
        st = new_st;
        i += 1;
    }
    Ok(st)
}

pub const fn run_utf8_validation_const(bytes: &[u8]) -> Result<(), Utf8Error> {
    // SAFETY: Start at empty string with valid state ACCEPT.
    match unsafe { run_with_error_handling(ST_ACCEPT, bytes, 0) } {
        Err(err) => Err(err),
        Ok(st) if st & STATE_MASK == ST_ACCEPT => Ok(()),
        Ok(st) => {
            // SAFETY: `st` is the last state after execution without encountering any error.
            let mut err = unsafe { resolve_error_location(st, bytes.len()) };
            err.error_len = Utf8ErrorLen::Eof;
            Err(err)
        }
    }
}

#[inline(always)]
pub fn run_utf8_validation<const MAIN_CHUNK_SIZE: usize, const ASCII_CHUNK_SIZE: usize>(
    bytes: &[u8],
) -> Result<(), Utf8Error> {
    const { assert!(ASCII_CHUNK_SIZE % MAIN_CHUNK_SIZE == 0) };

    let mut i = bytes.len() % MAIN_CHUNK_SIZE;
    // SAFETY: Start at initial state ACCEPT.
    let mut st = unsafe { run_with_error_handling(ST_ACCEPT, &bytes[..i], 0)? };

    while i < bytes.len() {
        // Fast path: if the current state is ACCEPT, we can skip to the next non-ASCII chunk.
        // We also did a quick inspection on the first byte to avoid getting into this path at all
        // when handling strings with almost no ASCII, eg. Chinese scripts.
        // SAFETY: `i` is in bound.
        if st == ST_ACCEPT && unsafe { bytes.get_unchecked(i).is_ascii() } {
            // SAFETY: `i` is in bound.
            let rest = unsafe { bytes.get_unchecked(i..) };
            let mut ascii_chunks = rest.array_chunks::<ASCII_CHUNK_SIZE>();
            let ascii_rest_chunk_cnt = ascii_chunks.len();
            let pos = ascii_chunks
                .position(|chunk| {
                    // NB. Always traverse the whole chunk instead of `.all()`, to persuade LLVM to
                    // vectorize this check.
                    // We also do not use `<[u8]>::is_ascii` which is unnecessarily complex here.
                    #[expect(clippy::unnecessary_fold)]
                    let all_ascii = chunk.iter().fold(true, |acc, b| acc && b.is_ascii());
                    !all_ascii
                })
                .unwrap_or(ascii_rest_chunk_cnt);
            i += pos * ASCII_CHUNK_SIZE;
            if i >= bytes.len() {
                break;
            }
        }

        // SAFETY: `i` and `i + MAIN_CHUNK_SIZE` are in bound by loop invariant.
        let chunk = unsafe { &*bytes.as_ptr().add(i).cast::<[u8; MAIN_CHUNK_SIZE]>() };
        let mut new_st = st;
        for &b in chunk {
            new_st = next_state(new_st, b);
        }
        if new_st & STATE_MASK == ST_ERROR {
            // SAFETY: `st` is the last state after executing `bytes[..i]` without encountering any error.
            // And we know the next chunk must fail the validation.
            return Err(unsafe { run_with_error_handling(st, bytes, i).unwrap_err_unchecked() });
        }

        st = new_st;
        i += MAIN_CHUNK_SIZE;
    }

    if st & STATE_MASK != ST_ACCEPT {
        // SAFETY: Same as above.
        let mut err = unsafe { resolve_error_location(st, bytes.len()) };
        err.error_len = Utf8ErrorLen::Eof;
        return Err(err);
    }

    Ok(())
}
