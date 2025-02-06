use crate::Utf8Error;

use super::validate_utf8 as parse;

fn cvt(err: std::str::Utf8Error) -> Utf8Error {
    error(err.valid_up_to(), err.error_len().map(|x| x as _))
}

fn error(valid_up_to: usize, error_len: Option<u8>) -> Utf8Error {
    Utf8Error {
        valid_up_to,
        error_len,
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
    let s = "\x00\u{80}\u{800}\u{10000}";
    for i in 0..=s.len() {
        let (lhs, rhs) = s.as_bytes().split_at(i);
        if s.is_char_boundary(i) {
            parse(lhs).unwrap();
            parse(rhs).unwrap();
        } else {
            assert_eq!(
                parse(lhs).unwrap_err(),
                cvt(std::str::from_utf8(lhs).unwrap_err()),
            );
            assert_eq!(
                parse(rhs).unwrap_err(),
                cvt(std::str::from_utf8(rhs).unwrap_err()),
            );
        }
    }
}
