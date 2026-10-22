//! Raw samples for the capsem-bench collector protocol; no statistics here.
//! Single-threaded, CPU-dispatched implementations. Cryptographic candidates
//! produce 256 bits; the optional non-security XXH3 comparison produces 128 bits.
use std::collections::BTreeMap;
use std::fs::File;
use std::hint::black_box;
use std::io::Read;
use std::time::Instant;

use sha2::Digest;

const ALGORITHMS: [&str; 5] = [
    "sha256-aws-lc",
    "sha256-sha2",
    "blake2b-256",
    "blake2s-256",
    "blake3-256",
];
const CHUNK: usize = 1024 * 1024;

enum Hash {
    Sha(capsem_crypto::Sha256),
    Reference(sha2::Sha256),
    B2b(blake2b_simd::State),
    B2s(blake2s_simd::State),
    B3(Box<blake3::Hasher>),
}
impl Hash {
    fn new(algorithm: &str) -> Self {
        match algorithm {
            "sha256-aws-lc" => Self::Sha(capsem_crypto::Sha256::new()),
            "sha256-sha2" => Self::Reference(sha2::Sha256::new()),
            "blake2b-256" => Self::B2b(blake2b_simd::Params::new().hash_length(32).to_state()),
            "blake2s-256" => Self::B2s(blake2s_simd::Params::new().hash_length(32).to_state()),
            "blake3-256" => Self::B3(Box::new(blake3::Hasher::new())),
            _ => unreachable!(),
        }
    }
    fn update(&mut self, bytes: &[u8]) {
        match self {
            Self::Sha(state) => state.update(bytes),
            Self::Reference(state) => state.update(bytes),
            Self::B2b(state) => {
                state.update(bytes);
            }
            Self::B2s(state) => {
                state.update(bytes);
            }
            Self::B3(state) => {
                state.update(bytes);
            }
        }
    }
    fn finish(self) -> [u8; 32] {
        match self {
            Self::Sha(state) => state.finalize().as_ref().try_into().unwrap(),
            Self::Reference(state) => state.finalize().into(),
            Self::B2b(state) => state.finalize().as_bytes().try_into().unwrap(),
            Self::B2s(state) => state.finalize().as_bytes().try_into().unwrap(),
            Self::B3(state) => *state.finalize().as_bytes(),
        }
    }
}

fn sample(metrics: &mut BTreeMap<String, Vec<f64>>, key: String, started: Instant) {
    let nanos = started.elapsed().as_secs_f64() * 1_000_000_000.0;
    eprintln!("{key}: {nanos:.0}ns");
    metrics.entry(key).or_default().push(nanos);
}

fn fingerprint(algorithm: &str, bytes: &[u8]) -> [u8; 32] {
    match algorithm {
        "blake3-256" => *capsem_crypto::blake3::hash(bytes).as_bytes(),
        "xxh3-128" => {
            // The return container is uniform, but the actual digest is 128 bits.
            // Padding is benchmark plumbing, never a 256-bit security claim.
            let mut result = [0; 32];
            result[..16].copy_from_slice(&capsem_crypto::xxh3::hash(bytes).to_le_bytes());
            result
        }
        _ => unreachable!(),
    }
}

fn fingerprint_samples(metrics: &mut BTreeMap<String, Vec<f64>>, bytes: &[u8]) {
    // Paired one-shot calls avoid per-iteration heap allocation or input copies.
    // Small-input samples amortize the clock; the benchmark owner computes stats.
    for round in 0..8 {
        let mut algorithms = ["blake3-256", "xxh3-128"];
        if round % 2 == 1 {
            algorithms.reverse();
        }
        for size in [32, 128, 1024, 64 * 1024, CHUNK, 10 * CHUNK] {
            let input = &bytes[..size];
            let iterations = (8 * CHUNK / size).max(1);
            for algorithm in algorithms {
                let expected = fingerprint(algorithm, input);
                let started = Instant::now();
                for _ in 0..iterations {
                    assert_eq!(black_box(fingerprint(algorithm, black_box(input))), expected);
                }
                sample(
                    metrics,
                    format!("oneshot.{size}.iterations-{iterations}.{algorithm}.wall_ns"),
                    started,
                );
            }
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut metrics = BTreeMap::<String, Vec<f64>>::new();
    let bytes: Vec<u8> = (0..256 * CHUNK).map(|index| (index % 251) as u8).collect();
    let files: Vec<_> = std::env::args().skip(1).collect();
    let mut identities = BTreeMap::new();
    let mut file_sizes = BTreeMap::new();
    let mut buffer = vec![0u8; CHUNK];
    // Warm every file before measuring; explicitly a cached verification workload.
    for path in &files {
        let mut file = File::open(path)?;
        while file.read(&mut buffer)? != 0 {}
    }
    for round in 0..2 {
        let mut algorithms = ALGORITHMS;
        if round == 1 {
            algorithms.reverse();
        }
        for size in [32, 1024, CHUNK, bytes.len()] {
            let iterations = (8 * CHUNK / size).max(1);
            for algorithm in algorithms {
                // Initialize dispatch before its timed samples.
                let mut warm = Hash::new(algorithm);
                warm.update(&bytes[..size]);
                let expected = warm.finish();
                let started = Instant::now();
                for _ in 0..iterations {
                    let mut state = Hash::new(algorithm);
                    for chunk in black_box(&bytes[..size]).chunks(CHUNK) {
                        state.update(chunk);
                    }
                    assert_eq!(black_box(state.finish()), expected);
                }
                sample(
                    &mut metrics,
                    format!("memory.{size}.iterations-{iterations}.{algorithm}.wall_ns"),
                    started,
                );
            }
        }
        for (index, path) in files.iter().enumerate() {
            for algorithm in std::iter::once("read-only").chain(algorithms) {
                let mut file = File::open(path)?;
                let mut state = (algorithm != "read-only").then(|| Hash::new(algorithm));
                let started = Instant::now();
                let mut total = 0;
                loop {
                    let count = file.read(&mut buffer)?;
                    if count == 0 {
                        break;
                    }
                    total += count;
                    if let Some(state) = &mut state {
                        state.update(black_box(&buffer[..count]));
                    } else {
                        black_box(&buffer[..count]);
                    }
                }
                let digest = state.map(Hash::finish);
                sample(&mut metrics, format!("erofs-{index}.{algorithm}.wall_ns"), started);
                let key = format!("erofs-{index}.{algorithm}");
                if let Some(previous) = identities.insert(key, digest) {
                    assert_eq!(digest, previous);
                }
                file_sizes.insert(index, total);
            }
        }
    }
    fingerprint_samples(&mut metrics, &bytes);
    let sidecar = serde_json::json!({"files": files, "file_sizes": file_sizes,
        "digests": identities, "chunk_bytes": CHUNK, "threads": 1,
        "fingerprint_output_bits": {"blake3-256":256, "xxh3-128":128},
        "fingerprint_input_copies":0, "fingerprint_heap_allocations_per_call":0,
        "architecture":std::env::consts::ARCH,
        "xxh3_feature":"xxh3", "xxh3_crate_version":"0.8.19",
        "neon":cfg!(target_feature="neon"), "avx2":cfg!(target_feature="avx2"),
        "cpu_flags": std::fs::read_to_string("/proc/cpuinfo").ok()
            .and_then(|text| text.lines().find(|line| line.starts_with("flags")).map(str::to_owned))});
    let metrics = collector_metrics(metrics);
    println!(
        "{}",
        serde_json::json!({"metrics":metrics, "sidecar":sidecar.to_string()})
    );
    Ok(())
}

fn collector_metrics(metrics: BTreeMap<String, Vec<f64>>) -> BTreeMap<String, serde_json::Value> {
    // The benchmark owner rounds to hundredths of the declared unit. Seconds
    // silently erased sub-5-ms samples; nanoseconds retain microbenchmark data.
    metrics
        .into_iter()
        .map(|(key, samples)| (key, serde_json::json!({"unit":"nanoseconds", "samples":samples})))
        .collect()
}

#[cfg(test)]
#[path = "compare_hashes/tests.rs"]
mod tests;
