//! LLM-MT-01 benchmark: measure Qwen2.5-0.5B / Phi-3-mini GGUF latency and
//! throughput for CPU real-time ja→vi machine translation.
//!
//! # Purpose
//!
//! Produces the empirical evidence artifact required by:
//! - `docs/adr/llm-mt-01-llm-vs-opus-mt-quality-tradeoff.md`
//! - `docs/adr/llm-mt-03-cpu-inference-crate-selection.md`
//!
//! # Usage (requires `--features local-llm-mt`)
//!
//! ```sh
//! cargo build --release --features local-llm-mt --bin llm_mt_bench
//!
//! # Load from local GGUF file (tokenizer.json + config.json must be in the same dir)
//! ./target/release/llm_mt_bench \
//!   --quantized-model-id path/to/qwen2.5-0.5b-instruct-q4_k_m.gguf \
//!   --tok-model-id Qwen/Qwen2.5-0.5B-Instruct \
//!   --model-label "Qwen2.5-0.5B Q4_K_M" \
//!   --output docs/evidence/llm-mt-01-bench.json
//!
//! # Load from HuggingFace cache (`huggingface-cli download` first)
//! ./target/release/llm_mt_bench \
//!   --hf-repo bartowski/Qwen2.5-0.5B-Instruct-GGUF \
//!   --quantized-filename Qwen2.5-0.5B-Instruct-Q4_K_M.gguf \
//!   --tok-model-id Qwen/Qwen2.5-0.5B-Instruct \
//!   --output docs/evidence/llm-mt-01-bench.json
//! ```
//!
//! # Pass/fail thresholds (from qa-leader council in LLM-MT-01 issue #696)
//!
//! | Metric                              | Threshold |
//! |-------------------------------------|-----------|
//! | P95 wall-clock latency (50-char JA) | ≤ 3 000 ms |
//! | Peak RSS delta vs baseline          | ≤ 1 536 MB |
//! | Cold start (load → first token)     | ≤ 5 000 ms |
//! | Sustained tokens/sec                | ≥ 15 tok/s |
//! | Glossary placeholder survival rate  | ≥ 0.98 |

// ── Feature guard ─────────────────────────────────────────────────────────────
// When `local-llm-mt` feature is absent the binary still compiles but exits
// immediately with an informative error.  This keeps `cargo clippy --all-targets`
// green without requiring the full mistralrs dependency tree.

#[cfg(feature = "local-llm-mt")]
mod bench {
    use anyhow::{bail, Context, Result};
    use candle_core::Device;
    use clap::Parser;
    use mistralrs_core::{
        AdapterPaths, DefaultSchedulerMethod, DeviceMapSetting, GGUFLoaderBuilder,
        GGUFSpecificConfig, LocalModelPaths, MistralRsBuilder, ModelDType, NormalRequest, Request,
        RequestMessage, Response, SamplingParams, SchedulerConfig, TokenSource,
    };
    use serde::{Deserialize, Serialize};
    use std::{
        fs,
        num::NonZeroUsize,
        path::PathBuf,
        sync::Arc,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };
    use tokio::sync::mpsc::channel;

    // ── CLI args ───────────────────────────────────────────────────────────────

    #[derive(Parser, Debug)]
    #[command(about = "LLM-MT-01 benchmark: measure GGUF LLM latency/throughput for ja→vi MT")]
    pub struct Args {
        /// Local path to the GGUF file.  The parent directory must also contain
        /// `tokenizer.json` and `config.json` from the base model's HF page.
        /// Mutually exclusive with `--hf-repo`.
        #[arg(long)]
        pub quantized_model_id: Option<String>,

        /// HuggingFace repo containing the GGUF file.
        /// Used when `--quantized-model-id` is not set.
        #[arg(long)]
        pub hf_repo: Option<String>,

        /// GGUF filename inside the HF repo (ignored with `--quantized-model-id`).
        #[arg(long, default_value = "Qwen2.5-0.5B-Instruct-Q4_K_M.gguf")]
        pub quantized_filename: String,

        /// HF model ID for tokenizer / chat template.
        #[arg(long, default_value = "Qwen/Qwen2.5-0.5B-Instruct")]
        pub tok_model_id: String,

        /// Human-readable label included in the output JSON.
        #[arg(long, default_value = "LLM-MT-01 model")]
        pub model_label: String,

        /// Warmup rounds (excluded from statistics).
        #[arg(long, default_value = "3")]
        pub warmup_rounds: usize,

        /// Benchmark rounds.
        #[arg(long, default_value = "20")]
        pub bench_rounds: usize,

        /// Max output tokens per translation.
        #[arg(long, default_value = "128")]
        pub max_tokens: usize,

        /// Path to write the JSON evidence artifact.
        #[arg(long, default_value = "docs/evidence/llm-mt-01-bench.json")]
        pub output: PathBuf,

        /// Suppress mistralrs progress output.
        #[arg(long, default_value = "true")]
        pub silent: bool,
    }

    // ── Evidence types ─────────────────────────────────────────────────────────

    /// One translated segment with timing.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct TranslationSample {
        /// Original Japanese input.
        pub input_ja: String,
        /// Vietnamese output produced by the model.
        pub output_vi: String,
        /// Wall-clock latency from send to first complete response (milliseconds).
        pub latency_ms: u64,
        /// Number of output tokens generated.
        pub output_tokens: usize,
        /// Output tokens per second for this segment.
        pub tokens_per_sec: f64,
    }

    /// Evidence artifact written to `docs/evidence/llm-mt-01-bench.json`.
    #[derive(Debug, Serialize, Deserialize)]
    pub struct BenchReport {
        /// Schema version for forward-compatible parsing.
        pub schema_version: &'static str,
        pub generated_at_unix_s: u64,
        pub model_label: String,
        pub tok_model_id: String,
        pub quantized_source: String,
        pub platform: String,
        pub cpu_cores: usize,

        // Latency (ms)
        pub latency_mean_ms: f64,
        pub latency_p50_ms: u64,
        pub latency_p95_ms: u64,
        pub latency_min_ms: u64,
        pub latency_max_ms: u64,

        // Throughput
        pub tokens_per_sec_mean: f64,
        /// 5th-percentile sustained throughput (worst-case in run).
        pub tokens_per_sec_p5: f64,

        // Memory
        pub cold_start_ms: u64,
        pub rss_baseline_bytes: u64,
        pub rss_peak_bytes: u64,
        pub rss_delta_bytes: i64,

        // Glossary sentinel survival (0.0–1.0)
        pub glossary_survival_rate: f64,

        // Verdict
        pub pass: bool,
        pub fail_reasons: Vec<String>,

        // Raw samples (warmup excluded)
        pub samples: Vec<TranslationSample>,
    }

    // ── Benchmark corpus ───────────────────────────────────────────────────────

    /// Zoom meeting JA segments mixing short phrases and medium sentences.
    const JA_SEGMENTS: &[&str] = &[
        "今日のスプリントレビューを始めます。",
        "このAPIのパフォーマンスを改善する必要があります。",
        "バックログの優先順位を再確認してください。",
        "リリース候補バージョン1.2.0の品質確認が完了しました。",
        "次のスプリントではユーザー認証機能を実装します。",
        "テストカバレッジを80%以上に維持してください。",
        "デプロイは金曜日の午後5時に予定されています。",
        "コードレビューのフィードバックを反映しました。",
        "データベースのマイグレーションスクリプトを更新しました。",
        "ステークホルダーへのデモは来週の月曜日です。",
    ];

    /// Sentinel tokens that must survive translation unchanged.
    const GLOSSARY_SENTINELS: &[&str] = &["__GTERM_0__", "__GTERM_1__", "__GTERM_2__"];

    fn glossary_test_input() -> String {
        "__GTERM_0__は今四半期のスプリント13です。\
         __GTERM_1__のAPIを使います。__GTERM_2__リリースは来月の予定です。"
            .to_string()
    }

    // ── RSS helper ─────────────────────────────────────────────────────────────

    /// Returns the resident-set size in bytes, or 0 when unavailable.
    ///
    /// On Linux: reads `/proc/self/status` VmRSS field.
    /// On macOS / Windows: returns 0 — use Activity Monitor or Process Explorer.
    fn rss_bytes() -> u64 {
        #[cfg(target_os = "linux")]
        {
            if let Ok(s) = fs::read_to_string("/proc/self/status") {
                for line in s.lines() {
                    if line.starts_with("VmRSS:") {
                        if let Some(kb) = line.split_whitespace().nth(1) {
                            if let Ok(n) = kb.parse::<u64>() {
                                return n * 1024;
                            }
                        }
                    }
                }
            }
            0
        }
        #[cfg(not(target_os = "linux"))]
        0
    }

    // ── Quantile helpers ───────────────────────────────────────────────────────

    fn percentile_u64(sorted: &[u64], p: f64) -> u64 {
        if sorted.is_empty() {
            return 0;
        }
        let idx = ((p / 100.0) * (sorted.len() - 1) as f64).round() as usize;
        sorted[idx.min(sorted.len() - 1)]
    }

    fn percentile_f64(sorted: &[f64], p: f64) -> f64 {
        if sorted.is_empty() {
            return 0.0;
        }
        let idx = ((p / 100.0) * (sorted.len() - 1) as f64).round() as usize;
        sorted[idx.min(sorted.len() - 1)]
    }

    // ── Single-segment translation ─────────────────────────────────────────────

    /// Translates `ja_text` to Vietnamese via the loaded model.
    ///
    /// Returns `(translated_text, output_token_count, wall_clock_duration)`.
    async fn translate_once(
        mistralrs: &Arc<mistralrs_core::MistralRs>,
        ja_text: &str,
        max_tokens: usize,
        request_id: usize,
    ) -> Result<(String, usize, Duration)> {
        let prompt = format!(
            "Translate the following Japanese text to Vietnamese. \
             Output only the Vietnamese translation, nothing else.\n\n\
             Japanese: {ja_text}\nVietnamese:"
        );

        let (tx, mut rx) = channel::<Response>(32);

        let sampling = SamplingParams {
            temperature: Some(0.1),
            top_p: Some(0.9),
            max_len: Some(max_tokens),
            ..SamplingParams::neutral()
        };

        let request = Request::Normal(Box::new(NormalRequest::new_simple(
            RequestMessage::Completion {
                text: prompt,
                echo_prompt: false,
                best_of: Some(1),
            },
            sampling,
            tx,
            request_id,
            None,
            None,
        )));

        let start = Instant::now();
        mistralrs
            .get_sender(None)
            .context("failed to get mistralrs sender")?
            .send(request)
            .await
            .context("failed to send request to mistralrs")?;

        let response = rx.recv().await.context("no response from mistralrs")?;
        let elapsed = start.elapsed();

        match response {
            Response::CompletionDone(cr) => {
                let tokens = cr.usage.completion_tokens;
                let text = cr
                    .choices
                    .into_iter()
                    .next()
                    .map(|c| c.text)
                    .unwrap_or_default();
                Ok((text, tokens, elapsed))
            }
            Response::InternalError(e) | Response::ValidationError(e) => {
                bail!("model returned error: {e}")
            }
            Response::CompletionModelError(msg, _) => {
                bail!("model completion error: {msg}")
            }
            _ => bail!("unexpected response variant from mistralrs"),
        }
    }

    // ── Main benchmark ─────────────────────────────────────────────────────────

    /// Run the full LLM-MT-01 benchmark and write the evidence JSON artifact.
    pub async fn run(args: Args) -> Result<()> {
        // ── Resolve model source ───────────────────────────────────────────────
        let (quantized_model_id, quantized_filename) = match &args.quantized_model_id {
            Some(p) => {
                let name = PathBuf::from(p)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| p.clone());
                (p.clone(), name)
            }
            None => match &args.hf_repo {
                Some(repo) => (repo.clone(), args.quantized_filename.clone()),
                None => bail!(
                    "provide either --quantized-model-id (local file) \
                     or --hf-repo (HuggingFace)"
                ),
            },
        };
        let is_local = args.quantized_model_id.is_some();

        // ── RSS baseline ───────────────────────────────────────────────────────
        let rss_baseline = rss_bytes();

        // ── Load model ─────────────────────────────────────────────────────────
        tracing::info!(model = %args.model_label, "loading GGUF model");
        let cold_start_begin = Instant::now();

        let loader = GGUFLoaderBuilder::new(
            None,
            Some(args.tok_model_id.clone()),
            quantized_model_id.clone(),
            vec![quantized_filename.clone()],
            GGUFSpecificConfig { topology: None },
            false,
            None,
        )
        .build();

        let device = Device::Cpu;
        let dtype = ModelDType::Auto;
        let device_map = DeviceMapSetting::dummy();

        let pipeline = if is_local {
            let gguf_path = PathBuf::from(&quantized_model_id);
            if !gguf_path.exists() {
                bail!("GGUF file not found: {}", gguf_path.display());
            }
            let dir = gguf_path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."));
            let paths: Box<dyn mistralrs_core::ModelPaths> = Box::new(LocalModelPaths::new(
                dir.join("tokenizer.json"),
                dir.join("config.json"),
                dir.join("tokenizer_config.json"),
                vec![gguf_path],
                AdapterPaths::None,
                None,
                None,
                None,
                None,
            ));
            loader.load_model_from_path(
                &paths,
                &dtype,
                &device,
                args.silent,
                device_map,
                None,
                None,
            )
        } else {
            loader.load_model_from_hf(
                None,
                TokenSource::CacheToken,
                &dtype,
                &device,
                args.silent,
                device_map,
                None,
                None,
            )
        }
        .context("failed to load GGUF model — check model path and tokenizer files")?;

        // 5 concurrent sequences is well within CPU memory for a 0.5B model.
        let concurrency = NonZeroUsize::new(5).expect("literal 5 is non-zero; this cannot fail");
        let mistralrs = MistralRsBuilder::new(
            pipeline,
            SchedulerConfig::DefaultScheduler {
                method: DefaultSchedulerMethod::Fixed(concurrency),
            },
            false,
            None,
        )
        .build()
        .await;

        let cold_start_ms = cold_start_begin.elapsed().as_millis() as u64;
        let rss_after_load = rss_bytes();
        tracing::info!(cold_start_ms, "model loaded");

        // ── Warmup rounds ──────────────────────────────────────────────────────
        tracing::info!(rounds = args.warmup_rounds, "running warmup");
        for i in 0..args.warmup_rounds {
            let seg = JA_SEGMENTS[i % JA_SEGMENTS.len()];
            translate_once(&mistralrs, seg, args.max_tokens, i).await?;
        }

        // ── Benchmark rounds ───────────────────────────────────────────────────
        tracing::info!(rounds = args.bench_rounds, "running benchmark rounds");
        let mut samples: Vec<TranslationSample> = Vec::with_capacity(args.bench_rounds);
        let mut rss_peak = rss_after_load;

        for i in 0..args.bench_rounds {
            let seg = JA_SEGMENTS[i % JA_SEGMENTS.len()];
            let (output, tokens, elapsed) =
                translate_once(&mistralrs, seg, args.max_tokens, args.warmup_rounds + i)
                    .await
                    .with_context(|| format!("benchmark round {i} failed"))?;

            let latency_ms = elapsed.as_millis() as u64;
            let tokens_per_sec = if elapsed.as_secs_f64() > 0.0 {
                tokens as f64 / elapsed.as_secs_f64()
            } else {
                0.0
            };
            rss_peak = rss_peak.max(rss_bytes());
            tracing::info!(round = i + 1, latency_ms, tokens, "sample done");
            samples.push(TranslationSample {
                input_ja: seg.to_string(),
                output_vi: output.trim().to_string(),
                latency_ms,
                output_tokens: tokens,
                tokens_per_sec,
            });
        }

        // ── Glossary sentinel survival ─────────────────────────────────────────
        let (gloss_out, _, _) =
            translate_once(&mistralrs, &glossary_test_input(), args.max_tokens, 9999)
                .await
                .context("glossary survival test failed")?;
        let survived = GLOSSARY_SENTINELS
            .iter()
            .filter(|&&s| gloss_out.contains(s))
            .count();
        let glossary_survival_rate = survived as f64 / GLOSSARY_SENTINELS.len() as f64;

        // ── Statistics ─────────────────────────────────────────────────────────
        let mut lat: Vec<u64> = samples.iter().map(|s| s.latency_ms).collect();
        lat.sort_unstable();
        let mut tps: Vec<f64> = samples.iter().map(|s| s.tokens_per_sec).collect();
        tps.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let latency_mean = lat.iter().sum::<u64>() as f64 / lat.len().max(1) as f64;
        let latency_p50 = percentile_u64(&lat, 50.0);
        let latency_p95 = percentile_u64(&lat, 95.0);
        let latency_min = lat.first().copied().unwrap_or(0);
        let latency_max = lat.last().copied().unwrap_or(0);
        let tps_mean = tps.iter().sum::<f64>() / tps.len().max(1) as f64;
        let tps_p5 = percentile_f64(&tps, 5.0);
        let rss_delta = rss_peak as i64 - rss_baseline as i64;

        // ── Pass/fail thresholds (from issue #696 qa-leader council) ───────────
        const P95_THRESHOLD_MS: u64 = 3_000;
        const RSS_THRESHOLD_BYTES: i64 = 1_536 * 1024 * 1024;
        const COLD_THRESHOLD_MS: u64 = 5_000;
        const TPS_THRESHOLD: f64 = 15.0;
        const GLOSSARY_THRESHOLD: f64 = 0.98;

        let mut fail_reasons: Vec<String> = Vec::new();
        if latency_p95 > P95_THRESHOLD_MS {
            fail_reasons.push(format!(
                "P95 latency {latency_p95}ms > {P95_THRESHOLD_MS}ms"
            ));
        }
        if rss_delta > RSS_THRESHOLD_BYTES {
            fail_reasons.push(format!(
                "RSS delta {}MB > {}MB",
                rss_delta / 1024 / 1024,
                RSS_THRESHOLD_BYTES / 1024 / 1024,
            ));
        }
        if cold_start_ms > COLD_THRESHOLD_MS {
            fail_reasons.push(format!(
                "cold start {cold_start_ms}ms > {COLD_THRESHOLD_MS}ms"
            ));
        }
        if tps_p5 < TPS_THRESHOLD {
            fail_reasons.push(format!(
                "P5 throughput {tps_p5:.1} tok/s < {TPS_THRESHOLD} tok/s"
            ));
        }
        if glossary_survival_rate < GLOSSARY_THRESHOLD {
            fail_reasons.push(format!(
                "glossary survival {glossary_survival_rate:.2} < {GLOSSARY_THRESHOLD}"
            ));
        }
        let pass = fail_reasons.is_empty();

        // ── Report ─────────────────────────────────────────────────────────────
        let report = BenchReport {
            schema_version: "llm-mt-01-v1",
            generated_at_unix_s: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            model_label: args.model_label.clone(),
            tok_model_id: args.tok_model_id.clone(),
            quantized_source: quantized_model_id.clone(),
            platform: format!(
                "{}/{}/{}",
                std::env::consts::OS,
                std::env::consts::ARCH,
                std::env::consts::FAMILY
            ),
            cpu_cores: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(0),
            latency_mean_ms: latency_mean,
            latency_p50_ms: latency_p50,
            latency_p95_ms: latency_p95,
            latency_min_ms: latency_min,
            latency_max_ms: latency_max,
            tokens_per_sec_mean: tps_mean,
            tokens_per_sec_p5: tps_p5,
            cold_start_ms,
            rss_baseline_bytes: rss_baseline,
            rss_peak_bytes: rss_peak,
            rss_delta_bytes: rss_delta,
            glossary_survival_rate,
            pass,
            fail_reasons: fail_reasons.clone(),
            samples,
        };

        println!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        println!("  LLM-MT-01 — {}", report.model_label);
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        println!("  Platform   : {}", report.platform);
        println!("  CPU cores  : {}", report.cpu_cores);
        println!(
            "  Cold start : {}ms  (≤ {}ms)",
            report.cold_start_ms, COLD_THRESHOLD_MS
        );
        println!("  Lat mean   : {:.0}ms", report.latency_mean_ms);
        println!("  Lat P50    : {}ms", report.latency_p50_ms);
        println!(
            "  Lat P95    : {}ms  (≤ {}ms)",
            report.latency_p95_ms, P95_THRESHOLD_MS
        );
        println!("  TPS mean   : {:.1} tok/s", report.tokens_per_sec_mean);
        println!(
            "  TPS P5     : {:.1} tok/s  (≥ {} tok/s)",
            report.tokens_per_sec_p5, TPS_THRESHOLD
        );
        println!(
            "  RSS delta  : {}MB  (≤ {}MB; 0 = platform not supported)",
            report.rss_delta_bytes / 1024 / 1024,
            RSS_THRESHOLD_BYTES / 1024 / 1024,
        );
        println!(
            "  Glossary   : {:.2}  (≥ {})",
            report.glossary_survival_rate, GLOSSARY_THRESHOLD
        );
        println!(
            "  Verdict    : {}",
            if report.pass { "✅ PASS" } else { "❌ FAIL" }
        );
        for r in &fail_reasons {
            println!("    ✗ {r}");
        }
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

        if let Some(parent) = args.output.parent() {
            fs::create_dir_all(parent).context("create evidence directory")?;
        }
        let json = serde_json::to_string_pretty(&report).context("serialize bench report")?;
        fs::write(&args.output, &json)
            .with_context(|| format!("write evidence to {}", args.output.display()))?;
        println!("\nEvidence written: {}", args.output.display());

        if !pass {
            bail!("benchmark FAILED — see fail_reasons in the evidence JSON");
        }
        Ok(())
    }
}

// ── Entry points ───────────────────────────────────────────────────────────────

#[cfg(feature = "local-llm-mt")]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use clap::Parser;
    tracing_subscriber::fmt::init();
    let args = bench::Args::parse();
    bench::run(args).await
}

#[cfg(not(feature = "local-llm-mt"))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!(
        "llm_mt_bench requires --features local-llm-mt\n\
         Build with: cargo build --features local-llm-mt --bin llm_mt_bench"
    )
}
