See: <https://github.com/rust-lang/rust/pull/136693>

My own benchmark result in x86\_64 is in `./benchmark.txt`.

Benchmark with 64-bit-shift:

`cargo bench --bench=validate_utf8`

Benchmark on 32-bit platforms with the only-32-bit-shift optimization:

`cargo bench --features=shift32 --bench=validate_utf8`

