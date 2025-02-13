use std::fs;

use crate::Utf8Error;

use super::from_utf8 as parse;
use super::*;

fn cvt<T>(ret: Result<T, std::str::Utf8Error>) -> Result<T, Utf8Error> {
    ret.map_err(cvt_err)
}

fn cvt_err(err: std::str::Utf8Error) -> Utf8Error {
    error(err.valid_up_to(), err.error_len().map(|x| x as _))
}

fn error(valid_up_to: usize, error_len: Option<u8>) -> Utf8Error {
    Utf8Error {
        valid_up_to,
        error_len: match error_len {
            None => Utf8ErrorLen::Eof,
            Some(1) => Utf8ErrorLen::One,
            Some(2) => Utf8ErrorLen::Two,
            Some(3) => Utf8ErrorLen::Three,
            _ => unreachable!(),
        },
    }
}

#[test]
fn state_distribution() {
    let s = fs::read_to_string("./test_data/zh.txt").unwrap();

    let mut counts = [0usize; STATE_CNT];
    let mut st = ST_ACCEPT;
    for b in s.bytes() {
        st = next_state(st, b);
        assert_ne!(st & STATE_MASK, ST_ERROR);
        counts[((st & STATE_MASK) / BITS_PER_STATE) as usize] += 1;
    }

    for (st, &cnt) in counts.iter().enumerate() {
        let frac = cnt as f64 / s.len() as f64;
        println!("S{st}: {cnt:6}, {frac:.6}");
    }
}

#[test]
fn empty() {
    parse(&[]).unwrap();
}

#[test]
fn valid() {
    let s = "\x00\u{80}\u{800}\u{10000}";
    parse(s.as_bytes()).unwrap();
}

#[test]
fn truncated() {
    let data = [
        "\x00\u{80}\u{800}\u{10000}".to_owned(),
        fs::read_to_string("./test_data/es.txt").unwrap(),
        fs::read_to_string("./test_data/zh.txt").unwrap(),
    ];

    for s in data {
        for i in 0..=s.len() {
            let (lhs, rhs) = s.as_bytes().split_at(i);
            assert_eq!(parse(lhs), cvt(std::str::from_utf8(lhs)));
            assert_eq!(parse(rhs), cvt(std::str::from_utf8(rhs)));
        }
    }
}

#[test]
#[cfg_attr(debug_assertions, ignore = "too slow on debug profile")]
fn mutated() {
    let data = [
        "\x00\u{80}\u{800}\u{10000}".to_owned(),
        fs::read_to_string("./test_data/es.txt").unwrap(),
        fs::read_to_string("./test_data/zh.txt").unwrap(),
    ];

    for s in data {
        let mut s = s.into_bytes();
        for i in 0..s.len() {
            let orig_byte = s[i];
            for new_byte in 0..=u8::MAX {
                s[i] = new_byte;
                assert_eq!(parse(&s), cvt(std::str::from_utf8(&s)));
            }
            s[i] = orig_byte;
        }
    }
}
