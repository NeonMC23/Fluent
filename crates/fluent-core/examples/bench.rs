//! Reproducible CPU inference benchmark for Fluent's NLLB engine.
//!
//! The harness deliberately separates model loading from generation, uses
//! deterministic fixtures and records a machine-readable JSON document plus a
//! short human-readable table. It calls the same `Nllb` / `NllbTokenizer` API
//! as the product, so it measures the real inference path rather than a
//! synthetic approximation.
//!
//! ```text
//! cargo run --release -p fluent-core --example bench -- \
//!     '/path/to/nllb-200-1.3b' --reps 3 --json /tmp/fluent-bench.json
//! ```
//!
//! The reference matrix runs the short, product-like cases with the default
//! heuristic early stopping, then forces the full 32/64/128/200-token budgets
//! on a long paragraph with `EarlyStopping::Never`. Forced budgets measure raw
//! throughput; they are not what a user sees on a sentence that ends early.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use candle_core::{DType, Device, Tensor};
use fluent_core::model::nllb::{EarlyStopping, GenerateOptions, GenerationStats, Nllb};
use fluent_core::NllbTokenizer;
use serde_json::{json, Map, Value};

// ---------------------------------------------------------------------------
// Fixed corpus
// ---------------------------------------------------------------------------
//
// Kept in the source so the benchmark is reproducible without external files.
// The texts are original and redistributable; they exercise punctuation,
// accents, numbers, proper nouns and paragraph boundaries.

struct Fixture {
    id: &'static str,
    category: &'static str,
    source_lang: &'static str,
    target_lang: &'static str,
    text: &'static str,
}

const FIXTURES: &[Fixture] = &[
    Fixture {
        id: "short-fr-en",
        category: "short",
        source_lang: "fra_Latn",
        target_lang: "eng_Latn",
        text: "Bonjour, comment allez-vous aujourd'hui ?",
    },
    Fixture {
        id: "short-en-fr",
        category: "short",
        source_lang: "eng_Latn",
        target_lang: "fra_Latn",
        text: "The weather is beautiful this morning, but it may rain later.",
    },
    Fixture {
        id: "short-en-de",
        category: "short",
        source_lang: "eng_Latn",
        target_lang: "deu_Latn",
        text: "Please send the signed contract to our office in Berlin by Friday.",
    },
    Fixture {
        id: "short-es-en",
        category: "short",
        source_lang: "spa_Latn",
        target_lang: "eng_Latn",
        text: "La reunión comienza a las nueve y media en la sala principal.",
    },
    Fixture {
        id: "short-it-en",
        category: "short",
        source_lang: "ita_Latn",
        target_lang: "eng_Latn",
        text: "Il museo apre alle nove e trenta, ma la mostra principale comincia solo nel pomeriggio.",
    },
    Fixture {
        id: "medium-en-fr",
        category: "medium",
        source_lang: "eng_Latn",
        target_lang: "fra_Latn",
        text: "The research team spent three years studying how coastal cities adapt to rising sea levels. \
               Their report, published in March 2024, combines satellite measurements with interviews \
               from more than four hundred residents in Lisbon, Rotterdam and New Orleans. Although the \
               authors disagree about the pace of change, they agree that local planning decisions have \
               become more important than international targets.",
    },
    Fixture {
        id: "long-en-fr",
        category: "long",
        source_lang: "eng_Latn",
        target_lang: "fra_Latn",
        text: "When the first trains arrived in the valley, few people believed that the small farming \
               community would become an industrial centre within a single generation. The river had \
               always provided water for the orchards, and the surrounding hills were covered with \
               forests that supplied timber for the new factories. Engineers from the capital surveyed \
               the land in 1857 and proposed a network of tracks that would connect the mines to the \
               harbour. Construction began the following spring, employing hundreds of workers who had \
               recently arrived from the countryside. They lived in temporary wooden houses near the \
               station, and many of them stayed after the line opened, buying plots of land on the \
               eastern slope. By 1890 the population had grown from twelve hundred to almost nine \
               thousand, and the town council had approved the building of a library, a hospital and a \
               secondary school. The old market square, once used for cattle and grain, was paved and \
               renamed after the engineer who had designed the bridge across the river. Today the \
               factories have mostly closed, but the terraces, the station and several of the original \
               warehouses remain, and the museum in the former power station documents the lives of \
               the families who built the railway.",
    },
    Fixture {
        id: "long-de-en",
        category: "long",
        source_lang: "deu_Latn",
        target_lang: "eng_Latn",
        text: "Die Stadtbibliothek wurde im Jahr 1902 eröffnet und war damals eines der modernsten \
               Gebäude der Region. Der Architekt entwarf einen großen Lesesaal mit hohen Fenstern, \
               dessen Dach von gusseisernen Säulen getragen wird. Während des Krieges wurden Teile der \
               Sammlung in ein Kloster auf dem Land gebracht, wo sie mehrere Jahre verborgen blieben. \
               Nach dem Wiederaufbau in den fünfziger Jahren erhielt die Bibliothek eine neue \
               Heizungsanlage und ein Magazin für rund zweihunderttausend Bände. Seit 2015 wird der \
               Bestand digitalisiert; bisher sind etwa vierzig Prozent der historischen Zeitungen \
               verfügbar. Besucher können die digitalisierten Seiten kostenlos einsehen, und die \
               Stadt plant, bis 2028 auch die Handschriften und Karten zugänglich zu machen.",
    },
    Fixture {
        id: "accent-fr-en",
        category: "short",
        source_lang: "fra_Latn",
        target_lang: "eng_Latn",
        text: "L'été dernier, à Montréal, nous avons visité une exposition consacrée à l'œuvre d'une artiste québécoise très célèbre.",
    },
    Fixture {
        id: "numbers-en-fr",
        category: "short",
        source_lang: "eng_Latn",
        target_lang: "fra_Latn",
        text: "The invoice for 1,250.75 euros was issued on 3 March 2024 and must be paid within 30 days.",
    },
    Fixture {
        id: "names-en-de",
        category: "short",
        source_lang: "eng_Latn",
        target_lang: "deu_Latn",
        text: "Dr. Anna Kowalski met Professor Müller and Ms. Chen at the International Institute in Vienna on Monday.",
    },
    Fixture {
        id: "repeat-es-en",
        category: "short",
        source_lang: "spa_Latn",
        target_lang: "eng_Latn",
        text: "El comité revisó el presupuesto, aprobó el presupuesto y publicó el presupuesto revisado en el sitio web del comité.",
    },
    Fixture {
        id: "paragraph-en-fr",
        category: "medium",
        source_lang: "eng_Latn",
        target_lang: "fra_Latn",
        text: "The library opens at eight. Visitors can borrow up to ten books for three weeks.\n\nThe reading room closes at seven on weekdays, but it stays open until nine on Thursdays.",
    },
    Fixture {
        id: "quotes-en-fr",
        category: "short",
        source_lang: "eng_Latn",
        target_lang: "fra_Latn",
        text: "The report (published by the \u{201c}Institut national de la statistique\u{201d} in 2024) states that approx. 12,000 residents \u{2014} i.e., about 8% \u{2014} support the plan; Dr. Martin disagreed, saying \u{201c}the numbers are misleading.\u{201d}",
    },
    Fixture {
        id: "measure-en-de",
        category: "short",
        source_lang: "eng_Latn",
        target_lang: "deu_Latn",
        text: "The parcel weighs 2.5 kg, measures 30 \u{00d7} 20 \u{00d7} 10 cm, and costs \u{20ac}49.99 (including VAT); delivery within the EU usually takes 3\u{2013}5 business days.",
    },
    Fixture {
        id: "boundary-en-fr",
        category: "long",
        source_lang: "eng_Latn",
        target_lang: "fra_Latn",
        text: "The regional planning office published its annual review on Thursday. The document describes how eleven coastal towns have changed since the harbour was modernised. It combines satellite maps, household surveys and interviews with local officials.\n\nAccording to the review, the population grew by twelve percent over the last decade, while the number of permanent jobs increased more slowly. Younger residents often commute to the capital, and several families have converted seasonal cottages into year-round homes.\n\nTransport remains the most contested topic. The new bus line reduced travel time to the hospital, but the evening service is still limited. Cyclists have asked for protected lanes along the main road, and shopkeepers worry that construction will reduce parking.\n\nThe report also examines the fishing fleet. Catches have declined, although prices rose enough to keep several boats in operation. The authors recommend a shared cold-storage facility and training programmes for crews who want to move into maintenance or tourism.\n\nWater management has become a second priority. The reservoir reached its lowest level in forty years last summer, and restrictions on garden watering lasted for six weeks. Engineers are studying a pipeline that would connect the region to the northern network, but the estimated cost is high.\n\nSchools report a shortage of teachers in science and mathematics. The district has offered housing support to new staff, and two retired teachers returned part-time. Parents have welcomed the measures but say class sizes remain too large in the eastern villages.\n\nFinally, the review considers the old school building. The council has not decided whether to renovate it as a library or sell the site to a developer. Public comments will be accepted until the end of March, and a final vote is expected in May.\n\nThe office will repeat the survey next year and publish an updated map of land use. Residents can download the full document, including the technical annexes, from the regional website or consult a printed copy at the library.",
    },
    Fixture {
        id: "truncated-en-fr",
        category: "long",
        source_lang: "eng_Latn",
        target_lang: "fra_Latn",
        text: "When the first trains arrived in the valley, few people believed that the small farming community would become an industrial centre within a single generation. The river had always provided water for the orchards, and the surrounding hills were covered with forests that supplied timber for the new factories. Engineers from the capital surveyed the land in 1857 and proposed a network of tracks that would connect the mines to the harbour.\n\nThe harbour authority approved a new warehouse near the eastern quay last month. The building will store grain before it is loaded onto ships, and it is expected to reduce waiting times during the harvest. Fishermen have asked for a separate cold room, arguing that the current facility is too small for the summer catch.\n\nA university consortium will open a research centre in the former textile mill next spring. The project includes laboratories for water analysis, a small auditorium and offices for visiting scientists. Local companies will be able to rent equipment, and the organisers hope that students will stay in the region after graduation.\n\nThe hospital announced that its emergency department will be renovated in two phases. During the first phase, patients will be redirected to a temporary unit beside the car park. The second phase will add two operating rooms and a dedicated imaging suite; officials say the work should finish before the winter.\n\nFarmers in the southern plain have started planting cover crops between the rows of vines. The practice reduces erosion and improves the soil, although it requires additional seed and careful timing. Several cooperatives are sharing machinery to lower the cost, and the regional council offers a small grant for the first three years.\n\nThe energy cooperative installed solar panels on the roofs of eleven public buildings. On sunny days the panels cover most of the demand, and the surplus is sold to the grid. The cooperative is now studying a battery system that would store electricity for the evening, when consumption rises sharply.\n\nThe city museum has digitised its collection of nineteenth-century photographs. Volunteers cleaned and scanned more than four thousand glass plates, many of which had never been displayed. The images can be browsed online, and a travelling exhibition will visit schools in the surrounding villages over the next year.\n\nA public meeting about the new flood defences will take place on the ninth of October in the town hall. Engineers will present three designs and explain how each affects the riverside paths. Residents who cannot attend may submit written comments until the end of the month; the final choice is expected in December.",
    },
];

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

struct Args {
    model_dir: PathBuf,
    reps: usize,
    budgets: Vec<usize>,
    beams: usize,
    json_path: Option<PathBuf>,
    dtype: DType,
    matmul_bench: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut model_dir = None;
    let mut reps = 3usize;
    let mut budgets = vec![32usize, 64, 128, 200];
    let mut beams = 4usize;
    let mut json_path = None;
    let mut dtype = DType::F32;
    let mut matmul_bench = false;

    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--reps" => {
                reps = iter
                    .next()
                    .ok_or("--reps needs a value")?
                    .parse()
                    .map_err(|_| "--reps must be an integer")?;
            }
            "--budgets" => {
                let value = iter.next().ok_or("--budgets needs a value")?;
                budgets = value
                    .split(',')
                    .filter(|part| !part.trim().is_empty())
                    .map(|part| part.trim().parse::<usize>())
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|_| "--budgets must be comma-separated integers")?;
            }
            "--beams" => {
                beams = iter
                    .next()
                    .ok_or("--beams needs a value")?
                    .parse()
                    .map_err(|_| "--beams must be an integer")?;
            }
            "--json" => {
                json_path = Some(PathBuf::from(iter.next().ok_or("--json needs a path")?));
            }
            "--f16" => dtype = DType::F16,
            "--matmul-bench" => matmul_bench = true,
            "--help" | "-h" => return Err("help".to_string()),
            other if other.starts_with("--") => {
                return Err(format!("unknown option: {other}"));
            }
            other => model_dir = Some(PathBuf::from(other)),
        }
    }

    Ok(Args {
        model_dir: model_dir.unwrap_or_else(|| PathBuf::from(".")),
        reps: reps.max(1),
        budgets: budgets.into_iter().filter(|value| *value > 0).collect(),
        beams: beams.max(1),
        json_path,
        dtype,
        matmul_bench,
    })
}

/// Times the quantized kernel alone, without the model. Useful to separate
/// kernel throughput from the surrounding inference pipeline.
fn run_matmul_bench() {
    use fluent_core::model::quant::QuantizedMatrix;

    println!("quantized matmul micro-benchmark");
    println!("simd: {}", fluent_core::model::quant::simd_backend());
    println!(
        "{:>5} {:>8} {:>7} {:>10} {:>11} {:>10}",
        "m", "rows", "cols", "ms/call", "GB/s(1pass)", "GFLOP/s"
    );

    // Prefill-sized matrices are included on purpose: the decoding kernel is
    // measured at m = 4, while the encoder runs at m = source length.
    let shapes: &[(usize, usize, &[usize])] = &[
        (1024, 1024, &[1, 4, 8, 16, 32, 64, 128, 267]),
        (8192, 1024, &[1, 4, 8, 16, 32, 64, 128, 267]),
        (256_206, 1024, &[1, 4, 8]),
    ];

    for &(rows, cols, ms) in shapes {
        let mut state = 0x1234_5678u32;
        let mut data = Vec::with_capacity(rows * cols);
        for _ in 0..rows * cols {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            data.push((state >> 8) as f32 / 8_388_608.0 - 1.0);
        }
        let weight = Tensor::from_vec(data, (rows, cols), &Device::Cpu).unwrap();
        let started = Instant::now();
        let matrix = QuantizedMatrix::from_tensor(&weight).unwrap();
        let quantized_in = started.elapsed();
        drop(weight);
        println!(
            "  {}x{}: quantize {:.1} ms for {} MB",
            rows,
            cols,
            quantized_in.as_secs_f64() * 1000.0,
            matrix.size_bytes() / (1024 * 1024)
        );

        for &m in ms {
            let input = Tensor::from_vec(
                (0..m * cols)
                    .map(|index| ((index * 7 % 31) as f32 - 15.0) / 15.0)
                    .collect::<Vec<_>>(),
                (m, cols),
                &Device::Cpu,
            )
            .unwrap();

            let work = m.saturating_mul(rows).saturating_mul(cols);
            let iterations = (200_000_000usize / work.max(1)).clamp(3, 200);
            // Warm up.
            let _ = matrix.matmul(&input).unwrap();
            let started = Instant::now();
            for _ in 0..iterations {
                let out = matrix.matmul(&input).unwrap();
                assert_eq!(out.dims(), &[m, rows]);
            }
            let per_call = started.elapsed().as_secs_f64() / iterations as f64;
            let weight_bytes = rows * cols * 9 / 8;
            let flops = 2.0 * m as f64 * rows as f64 * cols as f64;
            println!(
                "{:>5} {:>8} {:>7} {:>10.3} {:>11.1} {:>10.1}",
                m,
                rows,
                cols,
                per_call * 1000.0,
                weight_bytes as f64 / per_call / 1e9,
                flops / per_call / 1e9
            );
        }
    }

    println!("\nlogits post-processing ([4][256206] f32, log_softmax + top-k)");
    {
        use std::cmp::{Ordering, Reverse};
        use std::collections::BinaryHeap;

        #[derive(PartialEq)]
        struct Entry(f32, u32);
        impl Eq for Entry {}
        impl PartialOrd for Entry {
            fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
                Some(self.cmp(other))
            }
        }
        impl Ord for Entry {
            fn cmp(&self, other: &Self) -> Ordering {
                self.0.total_cmp(&other.0).then(self.1.cmp(&other.1))
            }
        }

        let beams = 4usize;
        let vocab = 256_206usize;
        let mut state = 0xdead_beefu32;
        let data: Vec<f32> = (0..beams * vocab)
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                (state >> 8) as f32 / 8_388_608.0 - 1.0
            })
            .collect();
        let logits = Tensor::from_vec(data, (beams, vocab), &Device::Cpu).unwrap();
        let iterations = 10;

        let started = Instant::now();
        for _ in 0..iterations {
            let lp = candle_nn::ops::log_softmax(&logits, 1).unwrap();
            std::hint::black_box(&lp);
        }
        let per = started.elapsed().as_secs_f64() / iterations as f64;
        println!("  candle log_softmax      : {:.2} ms", per * 1000.0);

        let lp = candle_nn::ops::log_softmax(&logits, 1).unwrap();
        let started = Instant::now();
        for _ in 0..iterations {
            let rows = lp.to_vec2::<f32>().unwrap();
            std::hint::black_box(&rows);
        }
        let per = started.elapsed().as_secs_f64() / iterations as f64;
        println!("  to_vec2 (nested Vec)    : {:.2} ms", per * 1000.0);

        let started = Instant::now();
        for _ in 0..iterations {
            let flat = lp.flatten_all().unwrap().to_vec1::<f32>().unwrap();
            std::hint::black_box(&flat);
        }
        let per = started.elapsed().as_secs_f64() / iterations as f64;
        println!("  to_vec1 (flat)          : {:.2} ms", per * 1000.0);

        let flat = logits.flatten_all().unwrap().to_vec1::<f32>().unwrap();
        let started = Instant::now();
        for _ in 0..iterations {
            for beam in 0..beams {
                let row = &flat[beam * vocab..(beam + 1) * vocab];
                let mut heap: BinaryHeap<Reverse<Entry>> = BinaryHeap::with_capacity(9);
                for (token, &value) in row.iter().enumerate() {
                    let entry = Reverse(Entry(value, token as u32));
                    if heap.len() < 8 {
                        heap.push(entry);
                    } else if value > heap.peek().unwrap().0 .0 {
                        heap.pop();
                        heap.push(entry);
                    }
                }
                std::hint::black_box(&heap);
            }
        }
        let per = started.elapsed().as_secs_f64() / iterations as f64;
        println!("  top-8 x 4 beams (heap)  : {:.2} ms", per * 1000.0);

        let started = Instant::now();
        for _ in 0..iterations {
            for beam in 0..beams {
                let row = &flat[beam * vocab..(beam + 1) * vocab];
                let mut max = f32::NEG_INFINITY;
                for &value in row {
                    max = max.max(value);
                }
                let mut sum = 0f32;
                for &value in row {
                    sum += (value - max).exp();
                }
                std::hint::black_box((max, sum));
            }
        }
        let per = started.elapsed().as_secs_f64() / iterations as f64;
        println!("  manual logsumexp        : {:.2} ms", per * 1000.0);
    }

    println!("\nencoder self-attention (batch 1, 16 heads, head_dim 64, candle path)");
    for seq in [32usize, 64, 128, 267, 512] {
        let q = Tensor::zeros((1, 16, seq, 64), DType::F32, &Device::Cpu).unwrap();
        let kt = Tensor::zeros((1, 16, 64, seq), DType::F32, &Device::Cpu).unwrap();
        let v = Tensor::zeros((1, 16, seq, 64), DType::F32, &Device::Cpu).unwrap();
        let iterations = if seq >= 267 { 20 } else { 50 };
        let started = Instant::now();
        for _ in 0..iterations {
            let scores = q.matmul(&kt).unwrap();
            let probs = candle_nn::ops::softmax_last_dim(&scores).unwrap();
            let out = probs.matmul(&v).unwrap();
            assert_eq!(out.dims(), &[1, 16, seq, 64]);
        }
        let per_call = started.elapsed().as_secs_f64() / iterations as f64;
        println!(
            "  seq {:>3}: {:.3} ms per layer (scores + softmax + values)",
            seq,
            per_call * 1000.0
        );
    }

    println!("\nattention building blocks (4 beams, 16 heads, head_dim 64)");
    for seq in [1usize, 32, 128, 267] {
        let q = Tensor::zeros((4, 16, 1, 64), DType::F32, &Device::Cpu).unwrap();
        let k = Tensor::zeros((1, 16, seq, 64), DType::F32, &Device::Cpu).unwrap();
        let v = Tensor::zeros((1, 16, seq, 64), DType::F32, &Device::Cpu).unwrap();
        let kt = k.transpose(2, 3).unwrap();
        let iterations = 200;
        let started = Instant::now();
        for _ in 0..iterations {
            let scores = q.broadcast_matmul(&kt).unwrap();
            let probs = candle_nn::ops::softmax_last_dim(&scores).unwrap();
            let out = probs.broadcast_matmul(&v).unwrap();
            assert_eq!(out.dims(), &[4, 16, 1, 64]);
        }
        let per_call = started.elapsed().as_secs_f64() / iterations as f64;
        println!(
            "  seq {:>3}: {:.3} ms per layer (scores + softmax + values)",
            seq,
            per_call * 1000.0
        );
    }
}

// ---------------------------------------------------------------------------
// Measurement helpers
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Sample {
    tokenize_ms: f64,
    encode_ms: f64,
    encode_attn_ms: f64,
    encode_ffn_ms: f64,
    cross_prepare_ms: f64,
    prefill_ms: f64,
    decoder_ms: f64,
    lm_head_ms: f64,
    search_ms: f64,
    search_topk_ms: f64,
    search_logits_ms: f64,
    search_extract_ms: f64,
    reorder_ms: f64,
    detokenize_ms: f64,
    generation_ms: f64,
    end_to_end_ms: f64,
    generated_tokens: usize,
    rss_peak_mb: Option<u64>,
    embedding_ms: f64,
    projections_ms: f64,
    kv_cache_ms: f64,
    self_attn_ms: f64,
    cross_attn_ms: f64,
    ffn_ms: f64,
}

struct CaseResult {
    id: String,
    category: String,
    source_lang: String,
    target_lang: String,
    source_tokens: usize,
    budget: usize,
    beams: usize,
    early_stopping: &'static str,
    truncated: bool,
    samples: Vec<Sample>,
    output: String,
    /// Token ids of the last repetition, for exact quality comparisons.
    tokens: Vec<u32>,
}

impl CaseResult {
    fn median(&self) -> Sample {
        let mut samples = self.samples.clone();
        median_of(&mut samples, |sample| sample.generation_ms)
    }

    fn min_generation(&self) -> f64 {
        self.samples
            .iter()
            .map(|sample| sample.generation_ms)
            .fold(f64::INFINITY, f64::min)
    }

    fn max_generation(&self) -> f64 {
        self.samples
            .iter()
            .map(|sample| sample.generation_ms)
            .fold(f64::NEG_INFINITY, f64::max)
    }

    fn tokens_per_second(&self) -> f64 {
        let sample = self.median();
        if sample.generation_ms <= 0.0 {
            return 0.0;
        }
        sample.generated_tokens as f64 * 1000.0 / sample.generation_ms
    }

    fn ms_per_token(&self) -> f64 {
        let sample = self.median();
        if sample.generated_tokens == 0 {
            return 0.0;
        }
        sample.generation_ms / sample.generated_tokens as f64
    }
}

fn median_of<F>(samples: &mut [Sample], key: F) -> Sample
where
    F: Fn(&Sample) -> f64,
{
    samples.sort_by(|a, b| key(a).total_cmp(&key(b)));
    samples[samples.len() / 2]
}

fn rss_mb() -> (Option<u64>, Option<u64>) {
    #[cfg(target_os = "linux")]
    {
        let Ok(raw) = std::fs::read_to_string("/proc/self/status") else {
            return (None, None);
        };
        let mut current = None;
        let mut peak = None;
        for line in raw.lines() {
            if let Some(rest) = line.strip_prefix("VmRSS:") {
                current = rest
                    .split_whitespace()
                    .next()
                    .and_then(|value| value.parse::<u64>().ok())
                    .map(|kb| kb / 1024);
            } else if let Some(rest) = line.strip_prefix("VmHWM:") {
                peak = rest
                    .split_whitespace()
                    .next()
                    .and_then(|value| value.parse::<u64>().ok())
                    .map(|kb| kb / 1024);
            }
        }
        (current, peak)
    }
    #[cfg(not(target_os = "linux"))]
    {
        (None, None)
    }
}

fn cpu_model() -> String {
    #[cfg(target_os = "linux")]
    {
        if let Ok(raw) = std::fs::read_to_string("/proc/cpuinfo") {
            for line in raw.lines() {
                if let Some(value) = line.strip_prefix("model name") {
                    if let Some((_, value)) = value.split_once(':') {
                        return value.trim().to_string();
                    }
                }
            }
        }
    }
    std::env::consts::ARCH.to_string()
}

fn cpu_flags() -> Vec<String> {
    let mut flags = Vec::new();
    if let Ok(raw) = std::fs::read_to_string("/proc/cpuinfo") {
        if let Some(line) = raw.lines().find(|line| line.starts_with("flags")) {
            if let Some((_, value)) = line.split_once(':') {
                for flag in value.split_whitespace() {
                    if matches!(
                        flag,
                        "avx" | "avx2" | "avx512f" | "fma" | "sse4_2" | "neon" | "dotprod"
                    ) {
                        flags.push(flag.to_string());
                    }
                }
            }
        }
    }
    flags
}

fn total_ram_mb() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let raw = std::fs::read_to_string("/proc/meminfo").ok()?;
        for line in raw.lines() {
            if let Some(rest) = line.strip_prefix("MemTotal:") {
                return rest
                    .split_whitespace()
                    .next()
                    .and_then(|value| value.parse::<u64>().ok())
                    .map(|kb| kb / 1024);
            }
        }
        None
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

fn os_label() -> String {
    #[cfg(target_os = "linux")]
    {
        if let Ok(raw) = std::fs::read_to_string("/etc/os-release") {
            for line in raw.lines() {
                if let Some(value) = line.strip_prefix("PRETTY_NAME=") {
                    return value.trim_matches('"').to_string();
                }
            }
        }
    }
    std::env::consts::OS.to_string()
}

fn rustc_version() -> String {
    Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_string())
        .unwrap_or_else(|| "unavailable".to_string())
}

fn dvfs_governor() -> Option<String> {
    std::fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor")
        .ok()
        .map(|value| value.trim().to_string())
}

// ---------------------------------------------------------------------------
// Benchmark execution
// ---------------------------------------------------------------------------

fn run_case(
    model: &Nllb,
    tokenizer: &NllbTokenizer,
    fixture: &Fixture,
    budget: usize,
    beams: usize,
    reps: usize,
    early_stopping: EarlyStopping,
) -> Result<CaseResult, Box<dyn std::error::Error>> {
    let mut samples = Vec::with_capacity(reps);
    let mut output = String::new();
    let mut tokens_out: Vec<u32> = Vec::new();
    let mut source_tokens = 0;
    let mut truncated = false;

    for _ in 0..reps {
        // End-to-end is measured around the same steps the product performs:
        // tokenize the source, generate, then detokenize the result. Model load
        // and GUI rendering stay outside the window.
        let end_started = Instant::now();
        let tokenize_started = Instant::now();
        let (source_ids, was_truncated) =
            tokenizer.encode_source(fixture.text, fixture.source_lang, 512)?;
        let tokenize = tokenize_started.elapsed();
        source_tokens = source_ids.len();
        truncated = was_truncated;

        let started = Instant::now();
        let mut noop = |_progress: fluent_core::model::nllb::GenerationProgress| {};
        let cancel = AtomicBool::new(false);
        let options = GenerateOptions {
            max_new_tokens: budget,
            num_beams: beams,
            early_stopping,
        };
        let (tokens, stats) = model.generate_with_stats(
            tokenizer,
            &source_ids,
            fixture.target_lang,
            options,
            &mut noop,
            &cancel,
        )?;
        let generation = started.elapsed();
        let decode_started = Instant::now();
        output = tokenizer.decode(&tokens)?;
        let detokenize = decode_started.elapsed();
        let end_to_end = end_started.elapsed();
        tokens_out = tokens;
        let mut sample = sample_from(stats, generation, end_to_end, tokens_out.len());
        sample.tokenize_ms = tokenize.as_secs_f64() * 1000.0;
        sample.detokenize_ms = detokenize.as_secs_f64() * 1000.0;
        samples.push(sample);
    }

    Ok(CaseResult {
        id: fixture.id.to_string(),
        category: fixture.category.to_string(),
        source_lang: fixture.source_lang.to_string(),
        target_lang: fixture.target_lang.to_string(),
        source_tokens,
        budget,
        beams,
        early_stopping: match early_stopping {
            EarlyStopping::Always => "always",
            EarlyStopping::Heuristic => "heuristic",
            EarlyStopping::Never => "never",
        },
        truncated,
        samples,
        output,
        tokens: tokens_out,
    })
}

fn sample_from(
    stats: GenerationStats,
    generation: std::time::Duration,
    end_to_end: std::time::Duration,
    generated_tokens: usize,
) -> Sample {
    let (_, peak) = rss_mb();
    Sample {
        tokenize_ms: 0.0,
        encode_ms: stats.encode.as_secs_f64() * 1000.0,
        encode_attn_ms: stats.encode_attn.as_secs_f64() * 1000.0,
        encode_ffn_ms: stats.encode_ffn.as_secs_f64() * 1000.0,
        cross_prepare_ms: stats.cross_prepare.as_secs_f64() * 1000.0,
        prefill_ms: stats.prefill.as_secs_f64() * 1000.0,
        decoder_ms: stats.decoder.as_secs_f64() * 1000.0,
        lm_head_ms: stats.lm_head.as_secs_f64() * 1000.0,
        search_ms: stats.search.as_secs_f64() * 1000.0,
        search_topk_ms: stats.search_topk.as_secs_f64() * 1000.0,
        search_logits_ms: stats.search_logits.as_secs_f64() * 1000.0,
        search_extract_ms: stats.search_extract.as_secs_f64() * 1000.0,
        reorder_ms: stats.reorder.as_secs_f64() * 1000.0,
        detokenize_ms: 0.0,
        generation_ms: generation.as_secs_f64() * 1000.0,
        end_to_end_ms: end_to_end.as_secs_f64() * 1000.0,
        generated_tokens,
        rss_peak_mb: peak,
        embedding_ms: stats.embedding.as_secs_f64() * 1000.0,
        projections_ms: stats.projections.as_secs_f64() * 1000.0,
        kv_cache_ms: stats.kv_cache.as_secs_f64() * 1000.0,
        self_attn_ms: stats.self_attn.as_secs_f64() * 1000.0,
        cross_attn_ms: stats.cross_attn.as_secs_f64() * 1000.0,
        ffn_ms: stats.ffn.as_secs_f64() * 1000.0,
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = match parse_args() {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}");
            eprintln!(
                "usage: bench <model_dir> [--reps N] [--budgets 32,64,128,200] \
                 [--beams N] [--json PATH] [--f16]"
            );
            std::process::exit(2);
        }
    };

    if args.matmul_bench {
        run_matmul_bench();
        return Ok(());
    }

    if !args.model_dir.is_dir() {
        return Err(format!("model directory not found: {}", args.model_dir.display()).into());
    }

    let device = Device::Cpu;
    println!("Fluent CPU benchmark");
    println!("model      : {}", args.model_dir.display());
    println!("dtype      : {:?}", args.dtype);
    println!("beams      : {}", args.beams);
    println!("reps       : {}", args.reps);
    println!("budgets    : {:?}", args.budgets);
    println!("simd       : {}", fluent_core::model::quant::simd_backend());
    println!();
    println!("loading model…");

    let load_started = Instant::now();
    let tokenizer = NllbTokenizer::from_dir(&args.model_dir)?;
    let model = Nllb::load(&args.model_dir, args.dtype, &device)?;
    let load = load_started.elapsed();
    let (rss_after_load, rss_peak_after_load) = rss_mb();
    println!(
        "loaded in {:.2} s (RSS {} MB, peak {} MB), weights quantized: {}",
        load.as_secs_f64(),
        rss_after_load.unwrap_or(0),
        rss_peak_after_load.unwrap_or(0),
        model.is_quantized()
    );
    println!();

    let mut cases: Vec<CaseResult> = Vec::new();

    // Product-like cases: heuristic early stopping, the default beam count, and
    // a budget large enough that the model stops on EOS.
    for fixture in FIXTURES {
        let budget = 64;
        cases.push(run_case(
            &model,
            &tokenizer,
            fixture,
            budget,
            args.beams,
            args.reps,
            EarlyStopping::Heuristic,
        )?);
    }

    // Forced-budget throughput matrix on the long English fixture. `Never`
    // guarantees that exactly the requested number of steps is executed.
    let long = FIXTURES
        .iter()
        .find(|fixture| fixture.id == "long-en-fr")
        .expect("long-en-fr fixture");
    for budget in &args.budgets {
        cases.push(run_case(
            &model,
            &tokenizer,
            long,
            *budget,
            args.beams,
            args.reps,
            EarlyStopping::Never,
        )?);
    }

    print_table(&cases);

    let (rss_end, rss_peak_end) = rss_mb();
    let report = build_report(
        &args,
        load,
        rss_after_load,
        rss_peak_after_load,
        (rss_end, rss_peak_end),
        &cases,
    );

    if let Some(path) = &args.json_path {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        std::fs::write(path, serde_json::to_string_pretty(&report)?)?;
        println!("\nJSON report written to {}", path.display());
    }

    print_breakdown(&cases);
    Ok(())
}

fn print_table(cases: &[CaseResult]) {
    println!(
        "{:<16} {:>5} {:>5} {:>5} {:>8} {:>8} {:>8} {:>8} {:>7} {:>8}",
        "case", "src", "budget", "gen", "gen s", "e2e s", "min s", "max s", "tok/s", "ms/tok"
    );
    println!("{}", "-".repeat(91));
    for case in cases {
        let median = case.median();
        println!(
            "{:<16} {:>5} {:>5} {:>5} {:>8.2} {:>8.2} {:>8.2} {:>8.2} {:>7.2} {:>8.1}",
            case.id,
            case.source_tokens,
            case.budget,
            median.generated_tokens,
            median.generation_ms / 1000.0,
            median.end_to_end_ms / 1000.0,
            case.min_generation() / 1000.0,
            case.max_generation() / 1000.0,
            case.tokens_per_second(),
            case.ms_per_token()
        );
    }
}

fn print_breakdown(cases: &[CaseResult]) {
    println!("\nStage breakdown (median, ms):");
    println!(
        "{:<16} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9}",
        "case",
        "encode",
        "crossKV",
        "prefill",
        "decoder",
        "lm_head",
        "search",
        "topk",
        "logits",
        "extract",
        "reorder"
    );
    for case in cases {
        let median = case.median();
        println!(
            "{:<16} {:>9.1} {:>9.1} {:>9.1} {:>9.1} {:>9.1} {:>9.1} {:>9.1} {:>9.1} {:>9.1} {:>9.1}",
            case.id,
            median.encode_ms,
            median.cross_prepare_ms,
            median.prefill_ms,
            median.decoder_ms,
            median.lm_head_ms,
            median.search_ms,
            median.search_topk_ms,
            median.search_logits_ms,
            median.search_extract_ms,
            median.reorder_ms
        );
    }
    println!("\nEncoder sub-stages (median, ms):");
    println!(
        "{:<16} {:>12} {:>12} {:>12}",
        "case", "attention", "ffn", "other"
    );
    for case in cases {
        let median = case.median();
        let other = (median.encode_ms - median.encode_attn_ms - median.encode_ffn_ms).max(0.0);
        println!(
            "{:<16} {:>12.1} {:>12.1} {:>12.1}",
            case.id, median.encode_attn_ms, median.encode_ffn_ms, other
        );
    }

    println!("\nDecoder sub-stages (median, ms):");
    println!(
        "{:<16} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9}",
        "case", "embed", "project", "kvCat", "selfAttn", "crossAttn", "ffn"
    );
    for case in cases {
        let median = case.median();
        println!(
            "{:<16} {:>9.1} {:>9.1} {:>9.1} {:>9.1} {:>9.1} {:>9.1}",
            case.id,
            median.embedding_ms,
            median.projections_ms,
            median.kv_cache_ms,
            median.self_attn_ms,
            median.cross_attn_ms,
            median.ffn_ms
        );
    }
    println!("\nTranslations:");
    for case in cases {
        if case.category == "short" {
            println!(
                "- {} ({}→{}): {}",
                case.id, case.source_lang, case.target_lang, case.output
            );
        }
    }
}

fn sample_to_json(sample: &Sample) -> Value {
    json!({
        "tokenize_ms": round3(sample.tokenize_ms),
        "encode_ms": round3(sample.encode_ms),
        "encode_attn_ms": round3(sample.encode_attn_ms),
        "encode_ffn_ms": round3(sample.encode_ffn_ms),
        "cross_prepare_ms": round3(sample.cross_prepare_ms),
        "prefill_ms": round3(sample.prefill_ms),
        "decoder_ms": round3(sample.decoder_ms),
        "lm_head_ms": round3(sample.lm_head_ms),
        "search_ms": round3(sample.search_ms),
        "search_topk_ms": round3(sample.search_topk_ms),
        "search_logits_ms": round3(sample.search_logits_ms),
        "search_extract_ms": round3(sample.search_extract_ms),
        "reorder_ms": round3(sample.reorder_ms),
        "detokenize_ms": round3(sample.detokenize_ms),
        "embedding_ms": round3(sample.embedding_ms),
        "projections_ms": round3(sample.projections_ms),
        "kv_cache_ms": round3(sample.kv_cache_ms),
        "self_attn_ms": round3(sample.self_attn_ms),
        "cross_attn_ms": round3(sample.cross_attn_ms),
        "ffn_ms": round3(sample.ffn_ms),
        "generation_ms": round3(sample.generation_ms),
        "end_to_end_ms": round3(sample.end_to_end_ms),
        "generated_tokens": sample.generated_tokens,
        "rss_peak_mb": sample.rss_peak_mb,
    })
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

fn build_report(
    args: &Args,
    load: std::time::Duration,
    rss_after_load: Option<u64>,
    rss_peak_after_load: Option<u64>,
    rss_end: (Option<u64>, Option<u64>),
    cases: &[CaseResult],
) -> Value {
    let mut environment = Map::new();
    environment.insert("cpu".into(), json!(cpu_model()));
    environment.insert("cpu_flags".into(), json!(cpu_flags()));
    environment.insert(
        "logical_cpus".into(),
        json!(std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(0)),
    );
    environment.insert("total_ram_mb".into(), json!(total_ram_mb()));
    environment.insert("os".into(), json!(os_label()));
    environment.insert("arch".into(), json!(std::env::consts::ARCH));
    environment.insert("rustc".into(), json!(rustc_version()));
    environment.insert("candle".into(), json!("0.11.0"));
    environment.insert("dvfs_governor".into(), json!(dvfs_governor()));

    let case_values: Vec<Value> = cases
        .iter()
        .map(|case| {
            let median = case.median();
            json!({
                "id": case.id,
                "category": case.category,
                "source_lang": case.source_lang,
                "target_lang": case.target_lang,
                "source_tokens": case.source_tokens,
                "budget": case.budget,
                "beams": case.beams,
                "early_stopping": case.early_stopping,
                "truncated": case.truncated,
                "output": case.output,
                "tokens": case.tokens,
                "median": sample_to_json(&median),
                "min_generation_ms": round3(case.min_generation()),
                "max_generation_ms": round3(case.max_generation()),
                "ms_per_token": round3(case.ms_per_token()),
                "tokens_per_second": round3(case.tokens_per_second()),
                "samples": case.samples.iter().map(sample_to_json).collect::<Vec<_>>(),
            })
        })
        .collect();

    json!({
        "schema": "fluent-bench/1",
        "generated_at_unix": SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0),
        "environment": Value::Object(environment),
        "model": {
            "dir": args.model_dir.display().to_string(),
            "dtype": format!("{:?}", args.dtype),
            "load_ms": round3(load.as_secs_f64() * 1000.0),
            "rss_after_load_mb": rss_after_load,
            "rss_peak_after_load_mb": rss_peak_after_load,
        },
        "reps": args.reps,
        "cases": case_values,
        "rss_end_mb": rss_end.0,
        "rss_peak_end_mb": rss_end.1,
    })
}
