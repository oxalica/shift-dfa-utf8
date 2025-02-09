use std::hint::black_box;
use std::io::Write;

use criterion::{criterion_group, criterion_main, Criterion};
use shift_dfa_utf8::run_utf8_validation;

#[repr(align(4096))]
struct Aligned<T>(T);

const LEN: usize = 4 << 10;

fn bench_throughput(c: &mut Criterion) {
    let mut buf = Box::new(Aligned([0u8; LEN * 2]));
    let buf = &mut buf.0[..];
    assert_eq!(buf.as_ptr() as usize % 64, 0);

    for lang in ["en", "es", "zh"] {
        let src = std::fs::read_to_string(format!("test_data/{lang}.txt")).unwrap();
        assert_eq!(src.len(), LEN);
        (&mut *buf).write_all(src.as_bytes()).unwrap();
        let first = src.char_indices().nth(1).unwrap().0;
        assert!(0 < first && first <= 4);

        let cases = [
            ("aligned", &buf[..LEN]),
            ("unaligned", &buf[first..][..LEN]),
        ];
        for (label, buf) in cases {
            let mut group = c.benchmark_group(format!("throughput-{label}-{lang}"));
            group
                .throughput(criterion::Throughput::Bytes(LEN as u64))
                .bench_function("std", |b| {
                    b.iter(|| std::str::from_utf8(black_box(buf)).unwrap())
                })
                .bench_function("shift-dfa-m8-a16", |b| {
                    b.iter(|| run_utf8_validation::<8, 16>(black_box(buf)).unwrap())
                })
                .bench_function("shift-dfa-m8-a32", |b| {
                    b.iter(|| run_utf8_validation::<8, 32>(black_box(buf)).unwrap())
                })
                .bench_function("shift-dfa-m16-a16", |b| {
                    b.iter(|| run_utf8_validation::<16, 16>(black_box(buf)).unwrap())
                })
                .bench_function("shift-dfa-m16-a32", |b| {
                    b.iter(|| run_utf8_validation::<16, 32>(black_box(buf)).unwrap())
                });
            group.finish();
        }
    }
}

fn bench_startup(c: &mut Criterion) {
    let err_cases = [
        // Less than one chunk.
        &b"\xFF"[..],
        b"\xC2\x803456\xFF",
        // Worse case.
        b"\xC2\x803456789012345\xFF",
    ];
    for s in err_cases {
        let mut group = c.benchmark_group(format!("latency-err-{}B", s.len()));
        group
            .bench_function("std", |b| {
                b.iter(|| std::str::from_utf8(black_box(s)).unwrap_err())
            })
            .bench_function("shift-dfa-m8-a16", |b| {
                b.iter(|| run_utf8_validation::<8, 16>(black_box(s)).unwrap_err());
            })
            .bench_function("shift-dfa-m8-a32", |b| {
                b.iter(|| run_utf8_validation::<8, 32>(black_box(s)).unwrap_err());
            })
            .bench_function("shift-dfa-m16-a16", |b| {
                b.iter(|| run_utf8_validation::<16, 16>(black_box(s)).unwrap_err());
            })
            .bench_function("shift-dfa-m16-a32", |b| {
                b.iter(|| run_utf8_validation::<16, 32>(black_box(s)).unwrap_err());
            });
        group.finish();
    }

    let ok_cases = [
        // Less than one chunk.
        &b"1"[..],
        b"\xC2\x8034567",
        // Full chunks.
        b"\xC2\x8034567890123456",
    ];
    for s in ok_cases {
        let mut group = c.benchmark_group(format!("latency-ok-{}B", s.len()));
        group
            .bench_function("std", |b| {
                b.iter(|| std::str::from_utf8(black_box(s)).unwrap())
            })
            .bench_function("shift-dfa-m8-a16", |b| {
                b.iter(|| run_utf8_validation::<8, 16>(black_box(s)).unwrap());
            })
            .bench_function("shift-dfa-m8-a32", |b| {
                b.iter(|| run_utf8_validation::<8, 32>(black_box(s)).unwrap());
            })
            .bench_function("shift-dfa-m16-a16", |b| {
                b.iter(|| run_utf8_validation::<16, 16>(black_box(s)).unwrap());
            })
            .bench_function("shift-dfa-m16-a32", |b| {
                b.iter(|| run_utf8_validation::<16, 32>(black_box(s)).unwrap());
            });
        group.finish();
    }
}

criterion_group!(benches, bench_throughput, bench_startup);
criterion_main!(benches);
