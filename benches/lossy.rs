use std::hint::black_box;
use std::io::Write;

use criterion::{criterion_group, criterion_main, Criterion};
use shift_dfa_utf8::lossy::to_utf8_chunks;

#[repr(align(4096))]
struct Aligned<T>(T);

fn bench_lossy(c: &mut Criterion) {
    let cases = [
        ("mixed", b"Hello\xC0\x80 There\xE6\x83 Goodbye".to_vec()),
        ("invalid", std::iter::repeat_n(0xF5, 100).collect()),
    ];

    for (label, bytes) in cases {
        let mut buf = Box::new(Aligned([0u8; 4096]));
        let buf = &mut buf.0[..];
        (&mut *buf).write_all(&bytes).unwrap();
        let bytes = &buf[..bytes.len()];
        let mut group = c.benchmark_group(format!("lossy-iter-{label}"));
        group
            .bench_function("std", |b| {
                b.iter(|| black_box(bytes).utf8_chunks().count());
            })
            .bench_function("shift-dfa-m8-a16", |b| {
                b.iter(|| to_utf8_chunks::<8, 16>(bytes).count());
            })
            .bench_function("shift-dfa-m8-a32", |b| {
                b.iter(|| to_utf8_chunks::<8, 32>(bytes).count());
            })
            .bench_function("shift-dfa-m16-a16", |b| {
                b.iter(|| to_utf8_chunks::<16, 16>(bytes).count());
            })
            .bench_function("shift-dfa-m16-a32", |b| {
                b.iter(|| to_utf8_chunks::<16, 32>(bytes).count());
            });
        group.finish();
    }
}

criterion_group!(benches, bench_lossy);
criterion_main!(benches);
