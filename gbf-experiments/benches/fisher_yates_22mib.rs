use criterion::{Criterion, criterion_group, criterion_main};
use gbf_experiments::s1::neg_test::{NEGATIVE_TEST_SHUFFLE_SEED, fisher_yates};

const VAL_22_MIB: usize = 22 * 1024 * 1024;

fn bench_fisher_yates_22mib(c: &mut Criterion) {
    let val = (0..VAL_22_MIB)
        .map(|index| (index % 251) as u8)
        .collect::<Vec<_>>();

    c.bench_function("s1_fisher_yates_22mib_single_thread", |b| {
        b.iter(|| fisher_yates(&val, NEGATIVE_TEST_SHUFFLE_SEED));
    });
}

criterion_group!(benches, bench_fisher_yates_22mib);
criterion_main!(benches);
