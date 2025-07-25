#![allow(non_snake_case)]
#![cfg_attr(feature = "docs", doc(include = "../../docs/range-proof-protocol.md"))]

use alloc::vec::Vec;
use curve25519_dalek::traits::{IsIdentity, VartimeMultiscalarMul};

use core::iter;

use curve25519_dalek::ristretto::{CompressedRistretto, RistrettoPoint};
use curve25519_dalek::scalar::Scalar;
use merlin::Transcript;

use crate::errors::ProofError;
use crate::generators::{BulletproofGens, PedersenGens};
use crate::inner_product_proof::InnerProductProof;
use crate::transcript::TranscriptProtocol;
use crate::util;

use rand_core::{CryptoRng, RngCore};
use serde::{self, Deserialize, Deserializer, Serialize, Serializer};

// Modules for MPC protocol

pub mod dealer;
pub mod messages;
pub mod party;

/// The `RangeProof` struct represents a proof that one or more values
/// are in a range.
///
/// The `RangeProof` struct contains functions for creating and
/// verifying aggregated range proofs.  The single-value case is
/// implemented as a special case of aggregated range proofs.
///
/// The bitsize of the range, as well as the list of commitments to
/// the values, are not included in the proof, and must be known to
/// the verifier.
///
/// This implementation requires that both the bitsize `n` and the
/// aggregation size `m` be powers of two, so that `n = 8, 16, 32, 64`
/// and `m = 1, 2, 4, 8, 16, ...`.  Note that the aggregation size is
/// not given as an explicit parameter, but is determined by the
/// number of values or commitments passed to the prover or verifier.
///
/// # Note
///
/// For proving, these functions run the multiparty aggregation
/// protocol locally.  That API is exposed in the [`aggregation`](::range_proof_mpc)
/// module and can be used to perform online aggregation between
/// parties without revealing secret values to each other.
#[derive(Clone, Debug)]
pub struct RangeProof {
    /// Commitment to the bits of the value
    A: CompressedRistretto,
    /// Commitment to the blinding factors
    S: CompressedRistretto,
    /// Commitment to the \\(t_1\\) coefficient of \\( t(x) \\)
    T_1: CompressedRistretto,
    /// Commitment to the \\(t_2\\) coefficient of \\( t(x) \\)
    T_2: CompressedRistretto,
    /// Evaluation of the polynomial \\(t(x)\\) at the challenge point \\(x\\)
    t_x: Scalar,
    /// Blinding factor for the synthetic commitment to \\(t(x)\\)
    t_x_blinding: Scalar,
    /// Blinding factor for the synthetic commitment to the inner-product arguments
    e_blinding: Scalar,
    /// Proof data for the inner-product argument.
    ipp_proof: InnerProductProof,
}

pub trait ValueCommitment: Copy {
    fn decompress(&self) -> Option<RistrettoPoint>;
    fn compress(&self) -> CompressedRistretto;
}

impl ValueCommitment for (RistrettoPoint, CompressedRistretto) {
    fn decompress(&self) -> Option<RistrettoPoint> {
        Some(self.0)
    }
    fn compress(&self) -> CompressedRistretto {
        self.1
    }
}

impl ValueCommitment for RistrettoPoint {
    fn decompress(&self) -> Option<RistrettoPoint> {
        Some(*self)
    }
    fn compress(&self) -> CompressedRistretto {
        self.compress()
    }
}

impl ValueCommitment for CompressedRistretto {
    fn decompress(&self) -> Option<RistrettoPoint> {
        self.decompress()
    }
    fn compress(&self) -> CompressedRistretto {
        *self
    }
}

impl RangeProof {
    /// Create a rangeproof for a given pair of value `v` and
    /// blinding scalar `v_blinding`.
    /// This is a convenience wrapper around [`RangeProof::prove_multiple`].
    ///
    /// # Example
    /// ```
    /// extern crate rand;
    /// use rand::thread_rng;
    ///
    /// extern crate curve25519_dalek;
    /// use curve25519_dalek::scalar::Scalar;
    ///
    /// extern crate merlin;
    /// use merlin::Transcript;
    ///
    /// extern crate bulletproofs;
    /// use bulletproofs::{BulletproofGens, PedersenGens, RangeProof};
    ///
    /// # fn main() {
    /// // Generators for Pedersen commitments.  These can be selected
    /// // independently of the Bulletproofs generators.
    /// let pc_gens = PedersenGens::default();
    ///
    /// // Generators for Bulletproofs, valid for proofs up to bitsize 64
    /// // and aggregation size up to 1.
    /// let bp_gens = BulletproofGens::new(64, 1);
    ///
    /// // A secret value we want to prove lies in the range [0, 2^32)
    /// let secret_value = 1037578891u64;
    ///
    /// // The API takes a blinding factor for the commitment.
    /// let blinding = Scalar::random(&mut thread_rng());
    ///
    /// // The proof can be chained to an existing transcript.
    /// // Here we create a transcript with a doctest domain separator.
    /// let mut prover_transcript = Transcript::new(b"doctest example");
    ///
    /// // Create a 32-bit rangeproof.
    /// let (proof, committed_value) = RangeProof::prove_single(
    ///     &bp_gens,
    ///     &pc_gens,
    ///     &mut prover_transcript,
    ///     secret_value,
    ///     &blinding,
    ///     32,
    /// ).expect("A real program could handle errors");
    ///
    /// // Verification requires a transcript with identical initial state:
    /// let mut verifier_transcript = Transcript::new(b"doctest example");
    /// assert!(
    ///     proof
    ///         .verify_single(&bp_gens, &pc_gens, &mut verifier_transcript, &committed_value, 32)
    ///         .is_ok()
    /// );
    /// # }
    /// ```
    pub fn prove_single_with_rng<T: RngCore + CryptoRng>(
        bp_gens: &BulletproofGens,
        pc_gens: &PedersenGens,
        transcript: &mut Transcript,
        v: u64,
        v_blinding: &Scalar,
        n: usize,
        rng: &mut T,
    ) -> Result<(RangeProof, CompressedRistretto), ProofError> {
        let (p, Vs) = RangeProof::prove_multiple_with_rng(
            bp_gens,
            pc_gens,
            transcript,
            &[v],
            &[*v_blinding],
            n,
            rng,
        )?;
        Ok((p, Vs[0]))
    }

    /// Create a rangeproof for a given pair of value `v` and
    /// blinding scalar `v_blinding`.
    /// This is a convenience wrapper around [`RangeProof::prove_single_with_rng`],
    /// passing in a threadsafe RNG.
    #[cfg(feature = "std")]
    pub fn prove_single(
        bp_gens: &BulletproofGens,
        pc_gens: &PedersenGens,
        transcript: &mut Transcript,
        v: u64,
        v_blinding: &Scalar,
        n: usize,
    ) -> Result<(RangeProof, CompressedRistretto), ProofError> {
        RangeProof::prove_single_with_rng(
            bp_gens,
            pc_gens,
            transcript,
            v,
            v_blinding,
            n,
            &mut rand::rng(),
        )
    }

    /// Create a rangeproof for a set of values.
    ///
    /// # Example
    /// ```
    /// extern crate rand;
    /// use rand::thread_rng;
    ///
    /// extern crate curve25519_dalek;
    /// use curve25519_dalek::scalar::Scalar;
    ///
    /// extern crate merlin;
    /// use merlin::Transcript;
    ///
    /// extern crate bulletproofs;
    /// use bulletproofs::{BulletproofGens, PedersenGens, RangeProof};
    ///
    /// # fn main() {
    /// // Generators for Pedersen commitments.  These can be selected
    /// // independently of the Bulletproofs generators.
    /// let pc_gens = PedersenGens::default();
    ///
    /// // Generators for Bulletproofs, valid for proofs up to bitsize 64
    /// // and aggregation size up to 16.
    /// let bp_gens = BulletproofGens::new(64, 16);
    ///
    /// // Four secret values we want to prove lie in the range [0, 2^32)
    /// let secrets = [4242344947u64, 3718732727u64, 2255562556u64, 2526146994u64];
    ///
    /// // The API takes blinding factors for the commitments.
    /// let blindings: Vec<_> = (0..4).map(|_| Scalar::random(&mut thread_rng())).collect();
    ///
    /// // The proof can be chained to an existing transcript.
    /// // Here we create a transcript with a doctest domain separator.
    /// let mut prover_transcript = Transcript::new(b"doctest example");
    ///
    /// // Create an aggregated 32-bit rangeproof and corresponding commitments.
    /// let (proof, commitments) = RangeProof::prove_multiple(
    ///     &bp_gens,
    ///     &pc_gens,
    ///     &mut prover_transcript,
    ///     &secrets,
    ///     &blindings,
    ///     32,
    /// ).expect("A real program could handle errors");
    ///
    /// // Verification requires a transcript with identical initial state:
    /// let mut verifier_transcript = Transcript::new(b"doctest example");
    /// assert!(
    ///     proof
    ///         .verify_multiple(&bp_gens, &pc_gens, &mut verifier_transcript, &commitments, 32)
    ///         .is_ok()
    /// );
    /// # }
    /// ```
    pub fn prove_multiple_with_rng<T: RngCore + CryptoRng>(
        bp_gens: &BulletproofGens,
        pc_gens: &PedersenGens,
        transcript: &mut Transcript,
        values: &[u64],
        blindings: &[Scalar],
        n: usize,
        rng: &mut T,
    ) -> Result<(RangeProof, Vec<CompressedRistretto>), ProofError> {
        use self::dealer::*;
        use self::party::*;

        if values.len() != blindings.len() {
            return Err(ProofError::WrongNumBlindingFactors);
        }

        let dealer = Dealer::new(bp_gens, pc_gens, transcript, n, values.len())?;

        let parties: Vec<_> = values
            .iter()
            .zip(blindings.iter())
            .map(|(&v, &v_blinding)| Party::new(bp_gens, pc_gens, v, v_blinding, n))
            // Collect the iterator of Results into a Result<Vec>, then unwrap it
            .collect::<Result<Vec<_>, _>>()?;

        let (parties, bit_commitments): (Vec<_>, Vec<_>) = parties
            .into_iter()
            .enumerate()
            .map(|(j, p)| {
                p.assign_position_with_rng(j, rng)
                    .expect("We already checked the parameters, so this should never happen")
            })
            .unzip();

        let value_commitments: Vec<_> = bit_commitments.iter().map(|c| c.V_j).collect();

        let (dealer, bit_challenge) = dealer.receive_bit_commitments(bit_commitments)?;

        let (parties, poly_commitments): (Vec<_>, Vec<_>) = parties
            .into_iter()
            .map(|p| p.apply_challenge_with_rng(&bit_challenge, rng))
            .unzip();

        let (dealer, poly_challenge) = dealer.receive_poly_commitments(poly_commitments)?;

        let proof_shares: Vec<_> = parties
            .into_iter()
            .map(|p| p.apply_challenge(&poly_challenge))
            // Collect the iterator of Results into a Result<Vec>, then unwrap it
            .collect::<Result<Vec<_>, _>>()?;

        let proof = dealer.receive_trusted_shares(&proof_shares)?;

        Ok((proof, value_commitments))
    }

    /// Create a rangeproof for a set of values.
    /// This is a convenience wrapper around [`RangeProof::prove_multiple_with_rng`],
    /// passing in a threadsafe RNG.
    #[cfg(feature = "std")]
    pub fn prove_multiple(
        bp_gens: &BulletproofGens,
        pc_gens: &PedersenGens,
        transcript: &mut Transcript,
        values: &[u64],
        blindings: &[Scalar],
        n: usize,
    ) -> Result<(RangeProof, Vec<CompressedRistretto>), ProofError> {
        RangeProof::prove_multiple_with_rng(
            bp_gens,
            pc_gens,
            transcript,
            values,
            blindings,
            n,
            &mut rand::rng(),
        )
    }

    /// Verifies a rangeproof for a given value commitment \\(V\\).
    ///
    /// This is a convenience wrapper around `verify_multiple` for the `m=1` case.
    pub fn verify_single_with_rng<T: RngCore + CryptoRng>(
        &self,
        bp_gens: &BulletproofGens,
        pc_gens: &PedersenGens,
        transcript: &mut Transcript,
        V: &impl ValueCommitment,
        n: usize,
        rng: &mut T,
    ) -> Result<(), ProofError> {
        self.verify_multiple_with_rng(bp_gens, pc_gens, transcript, &[*V], n, rng)
    }

    /// Verifies a rangeproof for a given value commitment \\(V\\).
    ///
    /// This is a convenience wrapper around [`RangeProof::verify_single_with_rng`],
    /// passing in a threadsafe RNG.
    #[cfg(feature = "std")]
    pub fn verify_single(
        &self,
        bp_gens: &BulletproofGens,
        pc_gens: &PedersenGens,
        transcript: &mut Transcript,
        V: &impl ValueCommitment,
        n: usize,
    ) -> Result<(), ProofError> {
        self.verify_single_with_rng(bp_gens, pc_gens, transcript, V, n, &mut rand::rng())
    }

    /// Verifies an aggregated rangeproof for the given value commitments.
    pub fn verify_multiple_with_rng<T: RngCore + CryptoRng>(
        &self,
        bp_gens: &BulletproofGens,
        pc_gens: &PedersenGens,
        transcript: &mut Transcript,
        value_commitments: &[impl ValueCommitment],
        n: usize,
        rng: &mut T,
    ) -> Result<(), ProofError> {
        Self::verify_batch_with_rng(
            iter::once(self.verification_view(transcript, value_commitments, n)),
            bp_gens,
            pc_gens,
            rng,
        )
    }

    /// Verifies an aggregated rangeproof for the given value commitments.
    /// This is a convenience wrapper around [`RangeProof::verify_multiple_with_rng`],
    /// passing in a threadsafe RNG.
    #[cfg(feature = "std")]
    pub fn verify_multiple(
        &self,
        bp_gens: &BulletproofGens,
        pc_gens: &PedersenGens,
        transcript: &mut Transcript,
        value_commitments: &[impl ValueCommitment],
        n: usize,
    ) -> Result<(), ProofError> {
        self.verify_multiple_with_rng(
            bp_gens,
            pc_gens,
            transcript,
            value_commitments,
            n,
            &mut rand::rng(),
        )
    }

    /// Create a view to this range proof for batch verification.
    pub fn verification_view<'a, V: ValueCommitment>(
        &'a self,
        transcript: &'a mut Transcript,
        value_commitments: &'a [V],
        n: usize,
    ) -> RangeProofView<'a, V> {
        RangeProofView {
            proof: self,
            transcript,
            value_commitments,
            n,
        }
    }

    pub fn verify_batch<'a, V: ValueCommitment + 'a>(
        batch: impl IntoIterator<Item = RangeProofView<'a, V>>,
        bp_gens: &BulletproofGens,
        pc_gens: &PedersenGens,
    ) -> Result<(), ProofError> {
        Self::verify_batch_with_rng(batch, bp_gens, pc_gens, &mut rand::rng())
    }

    pub fn verify_batch_with_rng<'a, T: RngCore + CryptoRng, V: ValueCommitment + 'a>(
        batch: impl IntoIterator<Item = RangeProofView<'a, V>>,
        bp_gens: &BulletproofGens,
        pc_gens: &PedersenGens,
        rng: &mut T,
    ) -> Result<(), ProofError> {
        let mut collector = BatchCollector::new(bp_gens, pc_gens);
        for el in batch {
            collector.add_proof(el, rng)?
        }

        collector.verify()
    }

    /// Serializes the proof into a byte array of \\(2 \lg n + 9\\)
    /// 32-byte elements, where \\(n\\) is the number of secret bits.
    ///
    /// # Layout
    ///
    /// The layout of the range proof encoding is:
    ///
    /// * four compressed Ristretto points \\(A,S,T_1,T_2\\),
    /// * three scalars \\(t_x, \tilde{t}_x, \tilde{e}\\),
    /// * \\(n\\) pairs of compressed Ristretto points \\(L_0,R_0\dots,L_{n-1},R_{n-1}\\),
    /// * two scalars \\(a, b\\).
    pub fn to_bytes(&self) -> Vec<u8> {
        // 7 elements: points A, S, T1, T2, scalars tx, tx_bl, e_bl.
        let mut buf = Vec::with_capacity(7 * 32 + self.ipp_proof.serialized_size());
        buf.extend_from_slice(self.A.as_bytes());
        buf.extend_from_slice(self.S.as_bytes());
        buf.extend_from_slice(self.T_1.as_bytes());
        buf.extend_from_slice(self.T_2.as_bytes());
        buf.extend_from_slice(self.t_x.as_bytes());
        buf.extend_from_slice(self.t_x_blinding.as_bytes());
        buf.extend_from_slice(self.e_blinding.as_bytes());
        buf.extend(self.ipp_proof.to_bytes_iter());
        buf
    }

    /// Deserializes the proof from a byte slice.
    ///
    /// Returns an error if the byte slice cannot be parsed into a `RangeProof`.
    pub fn from_bytes(slice: &[u8]) -> Result<RangeProof, ProofError> {
        if slice.len() % 32 != 0 {
            return Err(ProofError::FormatError);
        }
        if slice.len() < 7 * 32 {
            return Err(ProofError::FormatError);
        }

        use crate::util::read32;

        let A = CompressedRistretto(read32(&slice[0 * 32..]));
        let S = CompressedRistretto(read32(&slice[1 * 32..]));
        let T_1 = CompressedRistretto(read32(&slice[2 * 32..]));
        let T_2 = CompressedRistretto(read32(&slice[3 * 32..]));

        let t_x = Option::from(Scalar::from_canonical_bytes(read32(&slice[4 * 32..])))
            .ok_or(ProofError::FormatError)?;
        let t_x_blinding = Option::from(Scalar::from_canonical_bytes(read32(&slice[5 * 32..])))
            .ok_or(ProofError::FormatError)?;
        let e_blinding = Option::from(Scalar::from_canonical_bytes(read32(&slice[6 * 32..])))
            .ok_or(ProofError::FormatError)?;

        let ipp_proof = InnerProductProof::from_bytes(&slice[7 * 32..])?;

        Ok(RangeProof {
            A,
            S,
            T_1,
            T_2,
            t_x,
            t_x_blinding,
            e_blinding,
            ipp_proof,
        })
    }
}

impl Serialize for RangeProof {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_bytes(&self.to_bytes())
    }
}

impl<'de> Deserialize<'de> for RangeProof {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let bytes = Vec::deserialize(deserializer)?;
        // Using Error::custom requires T: Display, which our error
        // type only implements when it implements std::error::Error.
        #[cfg(feature = "std")]
        return RangeProof::from_bytes(&bytes).map_err(serde::de::Error::custom);
        // In no-std contexts, drop the error message.
        #[cfg(not(feature = "std"))]
        return RangeProof::from_bytes(&bytes)
            .map_err(|_| serde::de::Error::custom("deserialization error"));
    }
}

// TODO(merge): naming
pub struct RangeProofView<'a, V: ValueCommitment> {
    proof: &'a RangeProof,
    transcript: &'a mut Transcript,
    value_commitments: &'a [V],
    n: usize,
}

// Internal type which constructs the multiscalar mul for a batch.
// TODO(merge): g_scalars and h_scalars should probably be laid flat in memory as they are matrices
struct BatchCollector<'a> {
    dynamic_scalars: Vec<Scalar>,
    dynamic_points: Vec<Option<RistrettoPoint>>,
    pedersen_B_scalar: Scalar,
    pedersen_B_blinding_scalar: Scalar,
    g_scalars: Vec<Vec<Scalar>>,
    h_scalars: Vec<Vec<Scalar>>,
    party_capacity: usize,
    gens_capacity: usize,
    bp_gens: &'a BulletproofGens,
    pc_gens: &'a PedersenGens,
}

impl<'a> BatchCollector<'a> {
    fn new(bp_gens: &'a BulletproofGens, pc_gens: &'a PedersenGens) -> Self {
        Self {
            dynamic_scalars: vec![],
            dynamic_points: vec![],
            pedersen_B_scalar: Scalar::ZERO,
            pedersen_B_blinding_scalar: Scalar::ZERO,
            g_scalars: vec![],
            h_scalars: vec![],
            party_capacity: 0,
            gens_capacity: 0,
            bp_gens,
            pc_gens,
        }
    }

    fn add_proof<T: RngCore + CryptoRng, V: ValueCommitment>(
        &mut self,
        view: RangeProofView<V>,
        rng: &mut T,
    ) -> Result<(), ProofError> {
        let m = view.value_commitments.len();

        // First, replay the "interactive" protocol using the proof
        // data to recompute all challenges.
        if !(view.n == 8 || view.n == 16 || view.n == 32 || view.n == 64) {
            return Err(ProofError::InvalidBitsize);
        }
        if self.bp_gens.gens_capacity < view.n {
            return Err(ProofError::InvalidGeneratorsLength);
        }
        if self.bp_gens.party_capacity < m {
            return Err(ProofError::InvalidGeneratorsLength);
        }

        view.transcript
            .rangeproof_domain_sep(view.n as u64, m as u64);

        for V in view.value_commitments.iter() {
            // Allow the commitments to be zero (0 value, 0 blinding)
            // See https://github.com/dalek-cryptography/bulletproofs/pull/248#discussion_r255167177
            view.transcript.append_point(b"V", &V.compress());
        }

        view.transcript
            .validate_and_append_point(b"A", &view.proof.A)?;
        view.transcript
            .validate_and_append_point(b"S", &view.proof.S)?;

        let y = view.transcript.challenge_scalar(b"y");
        let z = view.transcript.challenge_scalar(b"z");
        let zz = z * z;
        let minus_z = -z;

        view.transcript
            .validate_and_append_point(b"T_1", &view.proof.T_1)?;
        view.transcript
            .validate_and_append_point(b"T_2", &view.proof.T_2)?;

        let x = view.transcript.challenge_scalar(b"x");

        view.transcript.append_scalar(b"t_x", &view.proof.t_x);
        view.transcript
            .append_scalar(b"t_x_blinding", &view.proof.t_x_blinding);
        view.transcript
            .append_scalar(b"e_blinding", &view.proof.e_blinding);

        let w = view.transcript.challenge_scalar(b"w");

        let (x_sq, x_inv_sq, s) = view
            .proof
            .ipp_proof
            .verification_scalars(view.n * m, view.transcript)?;
        let s_inv = s.iter().rev();

        let a = view.proof.ipp_proof.a;
        let b = view.proof.ipp_proof.b;

        view.transcript.append_scalar(b"ipp_a", &a);
        view.transcript.append_scalar(b"ipp_b", &b);

        // Challenge value for batching statements to be verified
        let c = view.transcript.challenge_scalar(b"c");

        // Construct concat_z_and_2, an iterator of the values of
        // z^0 * \vec(2)^n || z^1 * \vec(2)^n || ... || z^(m-1) * \vec(2)^n
        let powers_of_2: Vec<Scalar> = util::exp_iter(Scalar::from(2u64)).take(view.n).collect();
        let concat_z_and_2: Vec<Scalar> = util::exp_iter(z)
            .take(m)
            .flat_map(|exp_z| powers_of_2.iter().map(move |exp_2| exp_2 * exp_z))
            .collect();

        let mut g = s.iter().map(|s_i| minus_z - a * s_i);
        let mut h = s_inv
            .zip(util::exp_iter(y.invert()))
            .zip(concat_z_and_2.iter())
            .map(|((s_i_inv, exp_y_inv), z_and_2)| z + exp_y_inv * (zz * z_and_2 - b * s_i_inv));

        let value_commitment_scalars = util::exp_iter(z).take(m).map(|z_exp| c * zz * z_exp);
        let basepoint_scalar =
            w * (view.proof.t_x - a * b) + c * (delta(view.n, m, &y, &z) - view.proof.t_x);

        // Collect for batched multiscalar mul.

        // Batch challenge - not in transcript as each proof has its own transcript.
        let batch_factor = Scalar::random(rng);

        self.dynamic_scalars.extend(
            iter::once(Scalar::ONE)
                .chain(iter::once(x))
                .chain(iter::once(c * x))
                .chain(iter::once(c * x * x))
                .chain(x_sq.iter().cloned())
                .chain(x_inv_sq.iter().cloned())
                .chain(value_commitment_scalars)
                .map(|s| s * batch_factor),
        );

        self.dynamic_points.extend(
            iter::once(view.proof.A.decompress())
                .chain(iter::once(view.proof.S.decompress()))
                .chain(iter::once(view.proof.T_1.decompress()))
                .chain(iter::once(view.proof.T_2.decompress()))
                .chain(view.proof.ipp_proof.L_vec.iter().map(|L| L.decompress()))
                .chain(view.proof.ipp_proof.R_vec.iter().map(|R| R.decompress()))
                .chain(view.value_commitments.iter().map(|V| V.decompress())),
        );

        self.pedersen_B_blinding_scalar +=
            (-view.proof.e_blinding - c * view.proof.t_x_blinding) * batch_factor;
        self.pedersen_B_scalar += basepoint_scalar * batch_factor;

        // Support (m,n) that are less than the bp_gens capacity.

        self.party_capacity = self.party_capacity.max(m);
        self.gens_capacity = self.gens_capacity.max(view.n);

        self.g_scalars.resize_with(self.party_capacity, || vec![]);
        for v in &mut self.g_scalars {
            v.resize(self.gens_capacity, Scalar::ZERO);
        }
        self.h_scalars.resize_with(self.party_capacity, || vec![]);
        for v in &mut self.h_scalars {
            v.resize(self.gens_capacity, Scalar::ZERO);
        }

        for cur_m in 0..m {
            for cur_n in 0..view.n {
                self.g_scalars[cur_m][cur_n] += g.next().unwrap() * batch_factor;
                self.h_scalars[cur_m][cur_n] += h.next().unwrap() * batch_factor;
            }
        }

        Ok(())
    }

    fn verify(self) -> Result<(), ProofError> {
        let mega_check = RistrettoPoint::optional_multiscalar_mul(
            self.dynamic_scalars
                .into_iter()
                .chain(util::AssertSizeHint::new(
                    self.g_scalars.into_iter().flatten(),
                    self.gens_capacity * self.party_capacity,
                ))
                .chain(util::AssertSizeHint::new(
                    self.h_scalars.into_iter().flatten(),
                    self.gens_capacity * self.party_capacity,
                ))
                .chain(iter::once(self.pedersen_B_blinding_scalar))
                .chain(iter::once(self.pedersen_B_scalar)),
            self.dynamic_points
                .into_iter()
                .chain(
                    self.bp_gens
                        .G(self.gens_capacity, self.party_capacity)
                        .copied()
                        .map(Some),
                )
                .chain(
                    self.bp_gens
                        .H(self.gens_capacity, self.party_capacity)
                        .copied()
                        .map(Some),
                )
                .chain(iter::once(Some(self.pc_gens.B_blinding)))
                .chain(iter::once(Some(self.pc_gens.B))),
        )
        .ok_or_else(|| ProofError::VerificationError)?;

        if mega_check.is_identity().into() {
            Ok(())
        } else {
            Err(ProofError::VerificationError)
        }
    }
}

/// Compute
/// \\[
/// \delta(y,z) = (z - z^{2}) \langle \mathbf{1}, {\mathbf{y}}^{n \cdot m} \rangle - \sum_{j=0}^{m-1} z^{j+3} \cdot \langle \mathbf{1}, {\mathbf{2}}^{n \cdot m} \rangle
/// \\]
fn delta(n: usize, m: usize, y: &Scalar, z: &Scalar) -> Scalar {
    let sum_y = util::sum_of_powers(y, n * m);
    let sum_2 = util::sum_of_powers(&Scalar::from(2u64), n);
    let sum_z = util::sum_of_powers(z, m);

    (z - z * z) * sum_y - z * z * z * sum_2 * sum_z
}

#[cfg(test)]
mod tests {
    use rand::Rng;

    use super::*;

    use crate::generators::PedersenGens;

    #[test]
    fn test_delta() {
        let mut rng = rand::rng();
        let y = Scalar::random(&mut rng);
        let z = Scalar::random(&mut rng);

        // Choose n = 256 to ensure we overflow the group order during
        // the computation, to check that that's done correctly
        let n = 256;

        // code copied from previous implementation
        let z2 = z * z;
        let z3 = z2 * z;
        let mut power_g = Scalar::ZERO;
        let mut exp_y = Scalar::ONE; // start at y^0 = 1
        let mut exp_2 = Scalar::ONE; // start at 2^0 = 1
        for _ in 0..n {
            power_g += (z - z2) * exp_y - z3 * exp_2;

            exp_y = exp_y * y; // y^i -> y^(i+1)
            exp_2 = exp_2 + exp_2; // 2^i -> 2^(i+1)
        }

        assert_eq!(power_g, delta(n, 1, &y, &z),);
    }

    /// Given a bitsize `n`, test the following:
    ///
    /// 1. Generate `m` random values and create a proof they are all in range;
    /// 2. Serialize to wire format;
    /// 3. Deserialize from wire format;
    /// 4. Verify the proof.
    fn singleparty_create_and_verify_helper(n: usize, m: usize) {
        // Split the test into two scopes, so that it's explicit what
        // data is shared between the prover and the verifier.

        // Use bincode for serialization
        //use bincode; // already present in lib.rs

        // Both prover and verifier have access to the generators and the proof
        let max_bitsize = 64;
        let max_parties = 8;
        let pc_gens = PedersenGens::default();
        let bp_gens = BulletproofGens::new(max_bitsize, max_parties);

        // Prover's scope
        let (proof_bytes, value_commitments) = {
            let mut rng = rand::rng();

            // 0. Create witness data
            let (min, max) = (0u64, ((1u128 << n) - 1) as u64);
            let values: Vec<u64> = (0..m).map(|_| rng.random_range(min..max)).collect();
            let blindings: Vec<Scalar> = (0..m).map(|_| Scalar::random(&mut rng)).collect();

            // 1. Create the proof
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

            // 2. Return serialized proof and value commitments
            (bincode::serialize(&proof).unwrap(), value_commitments)
        };

        // Verifier's scope
        {
            // 3. Deserialize
            let proof: RangeProof = bincode::deserialize(&proof_bytes).unwrap();

            // 4. Verify with the same customization label as above
            let mut transcript = Transcript::new(b"AggregatedRangeProofTest");

            assert!(proof
                .verify_multiple(&bp_gens, &pc_gens, &mut transcript, &value_commitments, n)
                .is_ok());
        }
    }

    #[test]
    fn create_and_verify_n_32_m_1() {
        singleparty_create_and_verify_helper(32, 1);
    }

    #[test]
    fn create_and_verify_n_32_m_2() {
        singleparty_create_and_verify_helper(32, 2);
    }

    #[test]
    fn create_and_verify_n_32_m_4() {
        singleparty_create_and_verify_helper(32, 4);
    }

    #[test]
    fn create_and_verify_n_32_m_8() {
        singleparty_create_and_verify_helper(32, 8);
    }

    #[test]
    fn create_and_verify_n_64_m_1() {
        singleparty_create_and_verify_helper(64, 1);
    }

    #[test]
    fn create_and_verify_n_64_m_2() {
        singleparty_create_and_verify_helper(64, 2);
    }

    #[test]
    fn create_and_verify_n_64_m_4() {
        singleparty_create_and_verify_helper(64, 4);
    }

    #[test]
    fn create_and_verify_n_64_m_8() {
        singleparty_create_and_verify_helper(64, 8);
    }

    fn singleparty_create_and_verify_batch_helper(nm: &[(usize, usize)]) {
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

                (bincode::serialize(&proof).unwrap(), value_commitments, n)
            })
            .collect();

        // Verifier
        {
            let mut rng = rand::rng();

            let proofs: Vec<(RangeProof, _, _)> = proofs
                .into_iter()
                .map(|(proof_bytes, commitments, n)| {
                    (bincode::deserialize(&proof_bytes).unwrap(), commitments, n)
                })
                .collect();

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
        }
    }

    #[test]
    fn create_and_verify_batch_64_2() {
        singleparty_create_and_verify_batch_helper(&[(64, 2)]);
    }

    #[test]
    fn create_and_verify_batch_64_2x2() {
        singleparty_create_and_verify_batch_helper(&[(64, 2), (64, 2)]);
    }

    #[test]
    fn create_and_verify_batch_32_1_64_4_64_2_64_1() {
        singleparty_create_and_verify_batch_helper(&[(32, 1), (64, 4), (64, 2), (64, 1)]);
    }

    #[test]
    fn detect_dishonest_party_during_aggregation() {
        use self::dealer::*;
        use self::party::*;

        use crate::errors::MPCError;

        // Simulate four parties, two of which will be dishonest and use a 64-bit value.
        let m = 4;
        let n = 32;

        let pc_gens = PedersenGens::default();
        let bp_gens = BulletproofGens::new(n, m);

        let mut rng = rand::rng();
        let mut transcript = Transcript::new(b"AggregatedRangeProofTest");

        // Parties 0, 2 are honest and use a 32-bit value
        let v0 = rng.random::<u32>() as u64;
        let v0_blinding = Scalar::random(&mut rng);
        let party0 = Party::new(&bp_gens, &pc_gens, v0, v0_blinding, n).unwrap();

        let v2 = rng.random::<u32>() as u64;
        let v2_blinding = Scalar::random(&mut rng);
        let party2 = Party::new(&bp_gens, &pc_gens, v2, v2_blinding, n).unwrap();

        // Parties 1, 3 are dishonest and use a 64-bit value
        let v1 = rng.random::<u64>();
        let v1_blinding = Scalar::random(&mut rng);
        let party1 = Party::new(&bp_gens, &pc_gens, v1, v1_blinding, n).unwrap();

        let v3 = rng.random::<u64>();
        let v3_blinding = Scalar::random(&mut rng);
        let party3 = Party::new(&bp_gens, &pc_gens, v3, v3_blinding, n).unwrap();

        let dealer = Dealer::new(&bp_gens, &pc_gens, &mut transcript, n, m).unwrap();

        let (party0, bit_com0) = party0.assign_position(0).unwrap();
        let (party1, bit_com1) = party1.assign_position(1).unwrap();
        let (party2, bit_com2) = party2.assign_position(2).unwrap();
        let (party3, bit_com3) = party3.assign_position(3).unwrap();

        let (dealer, bit_challenge) = dealer
            .receive_bit_commitments(vec![bit_com0, bit_com1, bit_com2, bit_com3])
            .unwrap();

        let (party0, poly_com0) = party0.apply_challenge(&bit_challenge);
        let (party1, poly_com1) = party1.apply_challenge(&bit_challenge);
        let (party2, poly_com2) = party2.apply_challenge(&bit_challenge);
        let (party3, poly_com3) = party3.apply_challenge(&bit_challenge);

        let (dealer, poly_challenge) = dealer
            .receive_poly_commitments(vec![poly_com0, poly_com1, poly_com2, poly_com3])
            .unwrap();

        let share0 = party0.apply_challenge(&poly_challenge).unwrap();
        let share1 = party1.apply_challenge(&poly_challenge).unwrap();
        let share2 = party2.apply_challenge(&poly_challenge).unwrap();
        let share3 = party3.apply_challenge(&poly_challenge).unwrap();

        match dealer.receive_shares(&[share0, share1, share2, share3]) {
            Err(MPCError::MalformedProofShares { bad_shares }) => {
                assert_eq!(bad_shares, vec![1, 3]);
            }
            Err(_) => {
                panic!("Got wrong error type from malformed shares");
            }
            Ok(_) => {
                panic!("The proof was malformed, but it was not detected");
            }
        }
    }

    #[test]
    fn detect_dishonest_dealer_during_aggregation() {
        use self::dealer::*;
        use self::party::*;
        use crate::errors::MPCError;

        // Simulate one party
        let m = 1;
        let n = 32;

        let pc_gens = PedersenGens::default();
        let bp_gens = BulletproofGens::new(n, m);

        let mut rng = rand::rng();
        let mut transcript = Transcript::new(b"AggregatedRangeProofTest");

        let v0 = rng.random::<u32>() as u64;
        let v0_blinding = Scalar::random(&mut rng);
        let party0 = Party::new(&bp_gens, &pc_gens, v0, v0_blinding, n).unwrap();

        let dealer = Dealer::new(&bp_gens, &pc_gens, &mut transcript, n, m).unwrap();

        // Now do the protocol flow as normal....

        let (party0, bit_com0) = party0.assign_position(0).unwrap();

        let (dealer, bit_challenge) = dealer.receive_bit_commitments(vec![bit_com0]).unwrap();

        let (party0, poly_com0) = party0.apply_challenge(&bit_challenge);

        let (_dealer, mut poly_challenge) =
            dealer.receive_poly_commitments(vec![poly_com0]).unwrap();

        // But now simulate a malicious dealer choosing x = 0
        poly_challenge.x = Scalar::ZERO;

        let maybe_share0 = party0.apply_challenge(&poly_challenge);

        assert!(maybe_share0.unwrap_err() == MPCError::MaliciousDealer);
    }
}
