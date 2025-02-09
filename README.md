See: <https://github.com/rust-lang/rust/pull/136693>

To eyeball some numbers, see `./result-*.txt`.

Run throughput and latency benchmark for UTF-8 validation:

```console
$ ./run_benchmark.sh | tee ./result.txt
```

CBOR deserialization benchmark for mixed workload with small-string verifications:
(You need to build rustc locally on the PR above first.)

```console
$ cd ./bench-de
$ cargo clean # Do not use previous artifacts when using a new rustc dev build.
$ RUSTFLAGS='--sysroot=<dev-rustc-sysroot>' RUSTC='<dev-rustc>' cargo bench
```
