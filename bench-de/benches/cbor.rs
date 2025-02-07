use std::fs;

use criterion::{Criterion, criterion_group, criterion_main};

fn bench_cbor_de(c: &mut Criterion) {
    for ent in fs::read_dir("./dag-cbor-benchmark/data").unwrap() {
        let path = ent.unwrap().path();
        if path.extension().is_none_or(|ext| ext != "dagcbor") {
            continue;
        }
        let file_stem = path.file_stem().unwrap();
        if file_stem.to_string_lossy().contains("nested") {
            continue;
        }

        let data = fs::read(&path).unwrap();
        c.bench_function(&format!("cbor-de-{}", file_stem.to_string_lossy()), |b| {
            b.iter(|| {
                ciborium::de::from_reader::<serde::de::IgnoredAny, _>(&mut &data[..]).unwrap()
            });
        });
    }
}

criterion_group!(benches, bench_cbor_de);
criterion_main!(benches);
