#![allow(non_snake_case)]
#[macro_use]
extern crate criterion;
use criterion::Criterion;

use rand;
use rand::Rng;

use curve25519_dalek::scalar::Scalar;

use merlin::Transcript;

use bulletproofs::RangeProof;
use bulletproofs::{BulletproofGens, PedersenGens};

static AGGREGATION_SIZES: [usize; 6] = [1, 2, 4, 8, 16, 32];

fn create_aggregated_rangeproof_helper(n: usize, c: &mut Criterion) {
    let label = format!("Aggregated {}-bit rangeproof creation", n);

    c.bench_function_over_inputs(
        &label,
        move |b, &&m| {
            let pc_gens = PedersenGens::default();
            let bp_gens = BulletproofGens::new(n, m);
            let mut rng = rand::rng();

            let (min, max) = (0u64, ((1u128 << n) - 1) as u64);
            let values: Vec<u64> = (0..m).map(|_| rng.random_range(min..max)).collect();
            let blindings: Vec<Scalar> = (0..m).map(|_| Scalar::random(&mut rng)).collect();

            b.iter(|| {
                // Each proof creation requires a clean transcript.
                let mut transcript = Transcript::new(b"AggregateRangeProofBenchmark");

                RangeProof::prove_multiple(
                    &bp_gens,
                    &pc_gens,
                    &mut transcript,
                    &values,
                    &blindings,
                    n,
                )
            })
        },
        &AGGREGATION_SIZES,
    );
}

fn create_aggregated_rangeproof_n_8(c: &mut Criterion) {
    create_aggregated_rangeproof_helper(8, c);
}

fn create_aggregated_rangeproof_n_16(c: &mut Criterion) {
    create_aggregated_rangeproof_helper(16, c);
}

fn create_aggregated_rangeproof_n_32(c: &mut Criterion) {
    create_aggregated_rangeproof_helper(32, c);
}

fn create_aggregated_rangeproof_n_64(c: &mut Criterion) {
    create_aggregated_rangeproof_helper(64, c);
}

criterion_group! {
    name = create_rp;
    config = Criterion::default().sample_size(10);
    targets =
    create_aggregated_rangeproof_n_8,
    create_aggregated_rangeproof_n_16,
    create_aggregated_rangeproof_n_32,
    create_aggregated_rangeproof_n_64,
}

fn verify_aggregated_rangeproof_helper(n: usize, c: &mut Criterion) {
    let label = format!("Aggregated {}-bit rangeproof verification", n);

    c.bench_function_over_inputs(
        &label,
        move |b, &&m| {
            let pc_gens = PedersenGens::default();
            let bp_gens = BulletproofGens::new(n, m);
            let mut rng = rand::rng();

            let (min, max) = (0u64, ((1u128 << n) - 1) as u64);
            let values: Vec<u64> = (0..m).map(|_| rng.random_range(min..max)).collect();
            let blindings: Vec<Scalar> = (0..m).map(|_| Scalar::random(&mut rng)).collect();

            let mut transcript = Transcript::new(b"AggregateRangeProofBenchmark");
            let (proof, value_commitments) = RangeProof::prove_multiple(
                &bp_gens,
                &pc_gens,
                &mut transcript,
                &values,
                &blindings,
                n,
            )
            .unwrap();

            b.iter(|| {
                // Each proof creation requires a clean transcript.
                let mut transcript = Transcript::new(b"AggregateRangeProofBenchmark");

                proof.verify_multiple(&bp_gens, &pc_gens, &mut transcript, &value_commitments, n)
            });
        },
        &AGGREGATION_SIZES,
    );
}

fn verify_aggregated_rangeproof_n_8(c: &mut Criterion) {
    verify_aggregated_rangeproof_helper(8, c);
}

fn verify_aggregated_rangeproof_n_16(c: &mut Criterion) {
    verify_aggregated_rangeproof_helper(16, c);
}

fn verify_aggregated_rangeproof_n_32(c: &mut Criterion) {
    verify_aggregated_rangeproof_helper(32, c);
}

fn verify_aggregated_rangeproof_n_64(c: &mut Criterion) {
    verify_aggregated_rangeproof_helper(64, c);
}

criterion_group! {
    name = verify_rp;
    config = Criterion::default();
    targets =
    verify_aggregated_rangeproof_n_8,
    verify_aggregated_rangeproof_n_16,
    verify_aggregated_rangeproof_n_32,
    verify_aggregated_rangeproof_n_64,
}

fn verify_aggregated_rangeproof_batch_helper(
    name: &'static str,
    nm: &[(usize, usize)],
    c: &mut Criterion,
) {
    let label = format!("Batch rangeproof verify {}", name);

    c.bench_function(&label, move |b| {
        let max_bitsize = 64;
        let max_parties = 8;
        let pc_gens = PedersenGens::default();
        let bp_gens = BulletproofGens::new(max_bitsize, max_parties);

        // Provers
        let proofs: Vec<_> = nm
            .iter()
            .map(|&(n, m)| {
                let mut rng = rand::rng();

                let (min, max) = (0u64, ((1u128 << n) - 1) as u64);
                let values: Vec<u64> = (0..m).map(|_| rng.random_range(min..max)).collect();
                let blindings: Vec<Scalar> = (0..m).map(|_| Scalar::random(&mut rng)).collect();

                let mut transcript = Transcript::new(b"AggregatedRangeProofTest");
                let (proof, value_commitments) = RangeProof::prove_multiple(
                    &bp_gens,
                    &pc_gens,
                    &mut transcript,
                    &values,
                    &blindings,
                    n,
                )
                .unwrap();

                (proof, value_commitments, n)
            })
            .collect();

        let mut rng = rand::rng();
        b.iter(|| {
            let mut transcripts = proofs
                .iter()
                .map(|_| Transcript::new(b"AggregatedRangeProofTest"))
                .collect::<Vec<_>>();

            assert!(RangeProof::verify_batch_with_rng(
                proofs
                    .iter()
                    .zip(&mut transcripts)
                    .map(|((proof, commitments, n), transcript)| {
                        proof.verification_view(transcript, commitments, *n)
                    }),
                &bp_gens,
                &pc_gens,
                &mut rng
            )
            .is_ok());
        });
    });
}

fn verify_batch_32_1_64_4_64_2_64_1(c: &mut Criterion) {
    verify_aggregated_rangeproof_batch_helper(
        "32_1_64_4_64_2_64_1",
        &[(32, 1), (64, 4), (64, 2), (64, 1)],
        c,
    );
}

fn verify_batch_64_2_x32(c: &mut Criterion) {
    verify_aggregated_rangeproof_batch_helper("64_2_x32", &vec![(64, 2); 32], c);
}

criterion_group! {
    name = batch_verify;
    config = Criterion::default();
    targets =
    verify_batch_64_2_x32,
    verify_batch_32_1_64_4_64_2_64_1,
}

criterion_main!(create_rp, verify_rp, batch_verify);
