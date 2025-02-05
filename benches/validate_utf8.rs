use std::hint::black_box;
use std::io::Write;

use criterion::{criterion_group, criterion_main, Criterion};
use shift_dfa_utf8::{run_utf8_validation, run_utf8_validation_2};

#[repr(align(32))]
struct Aligned<T>(T);

const LEN: usize = 4 << 10;

fn bench_throughput(c: &mut Criterion) {
    let mut buf = Box::new(Aligned([0u8; LEN * 2]));
    let mut buf2 = Box::new(Aligned([0u8; LEN * 2]));
    let buf = &mut buf.0[..];
    let buf2 = &mut buf2.0[..];
    assert_eq!(buf.as_ptr() as usize % 32, 0);
    assert_eq!(buf2.as_ptr() as usize % 32, 0);

    for lang in ["en", "es", "zh"] {
        let src = std::fs::read_to_string(format!("test_data/{lang}.txt")).unwrap();
        assert_eq!(src.len(), LEN);
        (&mut *buf).write_all(src.as_bytes()).unwrap();
        (&mut *buf2).write_all(src.as_bytes()).unwrap();
        let first = src.char_indices().nth(1).unwrap().0;
        assert!(0 < first && first <= 4);

        for (label, buf, buf2) in [
            ("aligned", &buf[..LEN], &buf2[..LEN]),
            // ("unaligned", &buf[first..][..LEN], &buf2[first..][..LEN]),
        ] {
            let mut group = c.benchmark_group(format!("validate-utf8-{lang}-{label}"));
            group
                .throughput(criterion::Throughput::Bytes(LEN as u64))
                .bench_function("std", |b| {
                    b.iter(|| std::str::from_utf8(black_box(buf)).is_ok())
                })
                .bench_function("shift-dfa-m8-a8", |b| {
                    b.iter(|| run_utf8_validation::<8, 8>(black_box(buf)).unwrap())
                })
                .bench_function("shift-dfa-m8-a16", |b| {
                    b.iter(|| run_utf8_validation::<8, 16>(black_box(buf)).unwrap())
                })
                .bench_function("shift-dfa-m16-a16", |b| {
                    b.iter(|| run_utf8_validation::<16, 16>(black_box(buf)).unwrap())
                })
                .bench_function("shift-dfa-m16-a32", |b| {
                    b.iter(|| run_utf8_validation::<16, 32>(black_box(buf)).unwrap())
                });
            group.finish();
            let mut group = c.benchmark_group(format!("validate-utf8-2-{lang}-{label}"));
            group
                .throughput(criterion::Throughput::Bytes(2 * LEN as u64))
                .bench_function("std", |b| {
                    b.iter(|| std::str::from_utf8(black_box(buf)).and_then(|_| std::str::from_utf8(buf2)).is_ok())
                })
                .bench_function("shift-dfa-m8-a8", |b| {
                    b.iter(|| run_utf8_validation_2::<8, 8>(black_box(buf), black_box(buf2)).unwrap())
                })
                .bench_function("shift-dfa-m8-a16", |b| {
                    b.iter(|| run_utf8_validation_2::<8, 16>(black_box(buf), black_box(buf2)).unwrap())
                })
                .bench_function("shift-dfa-m16-a16", |b| {
                    b.iter(|| run_utf8_validation_2::<16, 16>(black_box(buf), black_box(buf2)).unwrap())
                })
                .bench_function("shift-dfa-m16-a32", |b| {
                    b.iter(|| run_utf8_validation_2::<16, 32>(black_box(buf), black_box(buf2)).unwrap())
                });
            group.finish();
        }
    }
}

criterion_group!(benches, bench_throughput);
criterion_main!(benches);
