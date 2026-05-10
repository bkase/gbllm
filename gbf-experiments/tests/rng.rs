mod common;

use common::injectable_rng::ScriptedRng;
use gbf_experiments::s1::rng::{
    BatchRng, InitRng, Pcg64Mcg, S1Rng, ShuffleRng, seed128, uniform_u64_inclusive,
};
use proptest::prelude::*;
use sha2::{Digest, Sha256};

const SNAPSHOT_SEEDS: [u64; 5] = [0, 1, 4, 42, 0xDEAD_BEEF];

impl S1Rng for ScriptedRng {
    fn next_u64(&mut self) -> u64 {
        ScriptedRng::next_u64(self)
    }

    fn fill_bytes(&mut self, out: &mut [u8]) {
        ScriptedRng::fill_bytes(self, out);
    }
}

#[derive(Debug)]
#[allow(dead_code)]
struct StreamSnapshot {
    stream: &'static str,
    seed: u64,
    draw_count: usize,
    sha256_le_digest: String,
    first: Vec<u64>,
    last: Vec<u64>,
}

#[test]
fn seed128_matches_sha256_little_endian_reference() {
    for (domain, seed) in [
        ("init", 0),
        ("batch", 1),
        ("shuffle", 42),
        ("custom-domain", 0xDEAD_BEEF),
    ] {
        let digest = Sha256::digest(format!("gbf:s1:{domain}:{seed}"));
        let mut expected = [0_u8; 16];
        expected.copy_from_slice(&digest[..16]);
        assert_eq!(seed128(domain, seed), u128::from_le_bytes(expected));
    }
}

#[test]
fn stream_constructors_use_disjoint_domain_seeds() {
    for seed in SNAPSHOT_SEEDS {
        let init = InitRng::new(seed);
        let batch = BatchRng::new(seed);
        let shuffle = ShuffleRng::new(seed);

        assert_eq!(init.state(), seed128("init", seed) | 1);
        assert_eq!(batch.state(), seed128("batch", seed) | 1);
        assert_eq!(shuffle.state(), seed128("shuffle", seed) | 1);
        assert_ne!(init.state(), batch.state());
        assert_ne!(init.state(), shuffle.state());
        assert_ne!(batch.state(), shuffle.state());
    }
}

#[test]
fn pcg64_mcg_forces_odd_state_for_zero_seed() {
    assert_eq!(Pcg64Mcg::new(0).state(), 1);
    assert_eq!(Pcg64Mcg::new(2).state(), 3);
    assert_eq!(Pcg64Mcg::new(3).state(), 3);
}

#[test]
fn fixed_seed_stream_outputs_are_pinned() {
    let mut snapshots = Vec::new();
    for seed in SNAPSHOT_SEEDS {
        let mut init = InitRng::new(seed);
        snapshots.push(summarize_stream(
            "init",
            seed,
            collect_draws(&mut init, 1024),
        ));

        let mut batch = BatchRng::new(seed);
        snapshots.push(summarize_stream(
            "batch",
            seed,
            collect_draws(&mut batch, 1024),
        ));

        let mut shuffle = ShuffleRng::new(seed);
        snapshots.push(summarize_stream(
            "shuffle",
            seed,
            collect_draws(&mut shuffle, 1024),
        ));
    }

    insta::assert_debug_snapshot!(snapshots, @r###"
    [
        StreamSnapshot {
            stream: "init",
            seed: 0,
            draw_count: 1024,
            sha256_le_digest: "e0b53051b57c3ac992bf9462d2c3e6c3a3dea457018b5b194fd31670fb9547d8",
            first: [
                6216604938433530357,
                12838686958887888017,
                889821623587044106,
                12053095543263957786,
            ],
            last: [
                12271724697316536221,
                16584414468582470721,
                9829915709861100917,
                6128131703871666250,
            ],
        },
        StreamSnapshot {
            stream: "batch",
            seed: 0,
            draw_count: 1024,
            sha256_le_digest: "07780ae72ce1a505895087159a3bb795254b8fc74859287cd47668791c9e72cb",
            first: [
                7776307122298849046,
                3152406331855411047,
                15609183647420967322,
                5738581704891052229,
            ],
            last: [
                11832627799779659431,
                15818604066397041827,
                5178282163541361359,
                4374621236550939520,
            ],
        },
        StreamSnapshot {
            stream: "shuffle",
            seed: 0,
            draw_count: 1024,
            sha256_le_digest: "e5834c0f750e9665197d3d96e2a71aa86f214677bc283419f0f6edb8702b2eca",
            first: [
                3202699813813394664,
                12377474701496660103,
                9837338997947609328,
                16892090573050414767,
            ],
            last: [
                611028305152899046,
                8960083977473099020,
                17458764914580112030,
                17650712388713172792,
            ],
        },
        StreamSnapshot {
            stream: "init",
            seed: 1,
            draw_count: 1024,
            sha256_le_digest: "7b5c4c93bbf86a41b9625faa0b2c65e1f93099bde75f6a4897f40ccd84be2822",
            first: [
                13220202712002818811,
                822535040110747018,
                16562897356970965997,
                16724039056136709912,
            ],
            last: [
                14643154739206163008,
                16591271713728000579,
                15264413629598782491,
                11030045951098958493,
            ],
        },
        StreamSnapshot {
            stream: "batch",
            seed: 1,
            draw_count: 1024,
            sha256_le_digest: "1108845358a845705ad027a81f889e7d824e40684823c79814bec1130000f079",
            first: [
                4309310533597911016,
                14143460122306565798,
                15552242299798930461,
                15952015413251986552,
            ],
            last: [
                1549240464139314224,
                15933979201261550796,
                3207980615226095141,
                10413224290015531608,
            ],
        },
        StreamSnapshot {
            stream: "shuffle",
            seed: 1,
            draw_count: 1024,
            sha256_le_digest: "1de33b9f1af9520555bda65b754f2f9a3ced667e6c2fec3536f52f53b7723b72",
            first: [
                1530083042249108651,
                914795511259491679,
                6169875805443504538,
                3711131040895224839,
            ],
            last: [
                16014706113477773590,
                11841564430816372140,
                3938671116277206758,
                10216104459163744268,
            ],
        },
        StreamSnapshot {
            stream: "init",
            seed: 4,
            draw_count: 1024,
            sha256_le_digest: "2c64734b259a4edc243e5a2462827d9c1303eed94799b043247ce804e97e8527",
            first: [
                4141852248788112082,
                18437372369234376245,
                4354485670020643,
                9608192024816523168,
            ],
            last: [
                2450782887711383180,
                9362730923240093589,
                5129145041830578515,
                393073243508400251,
            ],
        },
        StreamSnapshot {
            stream: "batch",
            seed: 4,
            draw_count: 1024,
            sha256_le_digest: "46335279424b18bca52207af16db78c72f50036f0d4697546cbb4cf079035a1b",
            first: [
                15272588928930054767,
                5194377002424249385,
                359403891374443160,
                9883561705505546315,
            ],
            last: [
                15210322117095514501,
                10147756756585356825,
                14280029007619768251,
                9099280868138098355,
            ],
        },
        StreamSnapshot {
            stream: "shuffle",
            seed: 4,
            draw_count: 1024,
            sha256_le_digest: "6765da37c4ad2b09f92a4a755097af0f77de13a6271c4feeed2c61c5f0afdf93",
            first: [
                6173704972793284510,
                17184402825065933211,
                208309485747269974,
                481601535575721087,
            ],
            last: [
                13219560045012953177,
                16244294091294847451,
                155184244821772908,
                9310384645499760518,
            ],
        },
        StreamSnapshot {
            stream: "init",
            seed: 42,
            draw_count: 1024,
            sha256_le_digest: "f41660b87368b260e323928f8d95c5750e7724a83f5d547a49056539e9da6053",
            first: [
                17971288067987998182,
                3820546206650518565,
                13925373125437052271,
                15120906475495481354,
            ],
            last: [
                7329070936740256953,
                1689647886503680435,
                14480405122056892148,
                13039242143826682644,
            ],
        },
        StreamSnapshot {
            stream: "batch",
            seed: 42,
            draw_count: 1024,
            sha256_le_digest: "a4f291a1b03e67e59ad8776bd611005a8c47ff35182c0f41050484f75b629b65",
            first: [
                7543160457327361596,
                7997051245080752099,
                12619279642853832059,
                18185248253211668289,
            ],
            last: [
                14187064480591722493,
                5613573611771313143,
                10889234447115231112,
                14133393776002319414,
            ],
        },
        StreamSnapshot {
            stream: "shuffle",
            seed: 42,
            draw_count: 1024,
            sha256_le_digest: "4931ab9c1934eb380125fbd37bcbfae1d7cbdc75f90fcba7ce6f29c550099bd7",
            first: [
                11314660207019047345,
                9810682379147077953,
                12584529177172025531,
                1772192960209840963,
            ],
            last: [
                17894650863902826876,
                3917995003056936811,
                1903128305478220328,
                8586469792005802546,
            ],
        },
        StreamSnapshot {
            stream: "init",
            seed: 3735928559,
            draw_count: 1024,
            sha256_le_digest: "40d85aa7b309eda17d35f1d0a9cea2db78a29633e920e2629c0219cb0029102e",
            first: [
                7941959984974728677,
                14077722681502964019,
                17507758280294360617,
                4577770569103486109,
            ],
            last: [
                869267220529011219,
                949184002107738221,
                11823776106976209220,
                12911684596325317532,
            ],
        },
        StreamSnapshot {
            stream: "batch",
            seed: 3735928559,
            draw_count: 1024,
            sha256_le_digest: "47d6885a1a05d3f5189a5278202a968f8b183ab0fc677e8cbf9bcfd4e964d01e",
            first: [
                10059284353062888330,
                17402330874302812300,
                13459919612084976934,
                5033381520221784419,
            ],
            last: [
                3790174418000064975,
                2446449607297821524,
                7224852787866379743,
                6397300755831460419,
            ],
        },
        StreamSnapshot {
            stream: "shuffle",
            seed: 3735928559,
            draw_count: 1024,
            sha256_le_digest: "34f6b42163d6a752877144a96855e3efc6409ececc28a9f0804d78ff7ea1e961",
            first: [
                8269031419255898846,
                2570497861174077686,
                14192497315204650530,
                7391619590163608546,
            ],
            last: [
                15818580913797494868,
                5863887535642945900,
                17612765770106050024,
                13410524962428802382,
            ],
        },
    ]
    "###);
}

#[test]
fn consuming_one_stream_does_not_perturb_the_others() {
    let seed = 42;

    let mut init = InitRng::new(seed);
    let mut batch = BatchRng::new(seed);
    let mut shuffle = ShuffleRng::new(seed);
    collect_draws(&mut init, 1000);
    assert_eq!(
        collect_draws(&mut batch, 16),
        reference_batch_after(0, seed, 16)
    );
    assert_eq!(
        collect_draws(&mut shuffle, 16),
        reference_shuffle_after(0, seed, 16)
    );

    let mut init = InitRng::new(seed);
    let mut batch = BatchRng::new(seed);
    let mut shuffle = ShuffleRng::new(seed);
    collect_draws(&mut batch, 1000);
    assert_eq!(
        collect_draws(&mut init, 16),
        reference_init_after(0, seed, 16)
    );
    assert_eq!(
        collect_draws(&mut shuffle, 16),
        reference_shuffle_after(0, seed, 16)
    );

    let mut init = InitRng::new(seed);
    let mut batch = BatchRng::new(seed);
    let mut shuffle = ShuffleRng::new(seed);
    collect_draws(&mut shuffle, 1000);
    assert_eq!(
        collect_draws(&mut init, 16),
        reference_init_after(0, seed, 16)
    );
    assert_eq!(
        collect_draws(&mut batch, 16),
        reference_batch_after(0, seed, 16)
    );
}

#[test]
fn uniform_u64_inclusive_rejects_top_partial_bucket_and_redraws() {
    let first_rejected_for_0_to_9 = (u64::MAX / 10) * 10;
    let mut rng = ScriptedRng::new([first_rejected_for_0_to_9, 7]);

    assert_eq!(uniform_u64_inclusive(&mut rng, 0, 9), 7);
    assert!(rng.is_empty(), "sampler must consume the accepted redraw");
}

#[test]
fn uniform_u64_inclusive_handles_full_u64_span() {
    let mut rng = ScriptedRng::new([u64::MAX, 0, 13]);
    assert_eq!(uniform_u64_inclusive(&mut rng, 0, u64::MAX), u64::MAX);
    assert_eq!(uniform_u64_inclusive(&mut rng, 0, u64::MAX), 0);
    assert_eq!(uniform_u64_inclusive(&mut rng, 0, u64::MAX), 13);
}

#[test]
#[should_panic(expected = "lo <= hi")]
fn uniform_u64_inclusive_rejects_empty_interval() {
    let mut rng = Pcg64Mcg::new(1);
    let _ = uniform_u64_inclusive(&mut rng, 10, 9);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn uniform_u64_inclusive_stays_in_range(seed in any::<u64>(), lo in 0_u64..(1_u64 << 32), span in 1_u64..(1_u64 << 20)) {
        let hi = lo + span - 1;
        let mut rng = BatchRng::new(seed);
        for _ in 0..1000 {
            let draw = uniform_u64_inclusive(&mut rng, lo, hi);
            prop_assert!((lo..=hi).contains(&draw));
        }
    }
}

#[test]
fn loose_chi_squared_smoke_test_catches_gross_bias() {
    assert_chi_squared_below(0, 9, 10, 20_000, 30.0);
    assert_chi_squared_below(0, 255, 256, 20_000, 340.0);
    assert_chi_squared_below(0, (1 << 20) - 1, 1024, 20_000, 1_200.0);
}

#[test]
fn rng_module_avoids_raw_percent_range_patterns() {
    let source_path = format!("{}/src/s1/rng.rs", env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(source_path).expect("rng source must be readable");
    for (line_index, line) in source.lines().enumerate() {
        assert!(
            !line.contains("% range") && !line.contains("%range"),
            "raw percent range pattern found on line {}: {line}",
            line_index + 1
        );
    }
}

fn collect_draws(rng: &mut impl S1Rng, count: usize) -> Vec<u64> {
    (0..count).map(|_| rng.next_u64()).collect()
}

fn summarize_stream(stream: &'static str, seed: u64, draws: Vec<u64>) -> StreamSnapshot {
    StreamSnapshot {
        stream,
        seed,
        draw_count: draws.len(),
        sha256_le_digest: sha256_le_digest(&draws),
        first: draws.iter().take(4).copied().collect(),
        last: draws
            .iter()
            .rev()
            .take(4)
            .copied()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect(),
    }
}

fn sha256_le_digest(draws: &[u64]) -> String {
    let mut hasher = Sha256::new();
    for draw in draws {
        hasher.update(draw.to_le_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn reference_init_after(skip: usize, seed: u64, count: usize) -> Vec<u64> {
    let mut rng = InitRng::new(seed);
    collect_draws(&mut rng, skip);
    collect_draws(&mut rng, count)
}

fn reference_batch_after(skip: usize, seed: u64, count: usize) -> Vec<u64> {
    let mut rng = BatchRng::new(seed);
    collect_draws(&mut rng, skip);
    collect_draws(&mut rng, count)
}

fn reference_shuffle_after(skip: usize, seed: u64, count: usize) -> Vec<u64> {
    let mut rng = ShuffleRng::new(seed);
    collect_draws(&mut rng, skip);
    collect_draws(&mut rng, count)
}

fn assert_chi_squared_below(lo: u64, hi: u64, buckets: usize, draws: usize, max_chi_squared: f64) {
    let mut rng = ShuffleRng::new(0xDEAD_BEEF);
    let mut observed = vec![0_usize; buckets];
    let width = hi - lo + 1;
    for _ in 0..draws {
        let value = uniform_u64_inclusive(&mut rng, lo, hi);
        let bucket = ((value - lo) as usize * buckets) / width as usize;
        observed[bucket] += 1;
    }

    let expected = draws as f64 / buckets as f64;
    let chi_squared = observed
        .into_iter()
        .map(|count| {
            let delta = count as f64 - expected;
            delta * delta / expected
        })
        .sum::<f64>();
    assert!(
        chi_squared < max_chi_squared,
        "chi-squared {chi_squared} exceeded threshold {max_chi_squared}"
    );
}
