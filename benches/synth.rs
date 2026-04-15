// Criterion benches for Kokoro TTS warm-inference latency.
//
// Scope: measures warm inference only. The first call per (voice, duration)
// cell is a warm-up executed outside the measured closure to trigger
// ONNXRUNTIME_INIT and per-voice model load. Cold-start variance is
// explicitly excluded from this gate (see plan E-03).

use std::time::Duration;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use lingopilot_tts_kokoro::live_test_support::LiveTestAssets;
use lingopilot_tts_kokoro::synthesis::{resolve_model_assets, ExecutionProvider, SynthesisCache};

// Fixture word counts were picked so that Kokoro's generated PCM16 duration
// lands within ~10% of the target. Calibrate at implementation time.
const TEXT_EN_1S: &str = "The quick brown fox jumps over the lazy dog today.";
const TEXT_EN_5S: &str = "The quick brown fox jumps over the lazy dog today, then returns along the river bank, weaving through the tall grass and pausing briefly to listen for distant thunder before moving on.";
const TEXT_EN_20S: &str = "The quick brown fox jumps over the lazy dog today, then returns along the river bank, weaving through the tall grass and pausing briefly to listen for distant thunder. A flock of starlings rises from the oak tree in a spiral, scattering leaves across the worn footpath that leads toward the old stone bridge. Children laugh somewhere beyond the hedgerow while a single bell rings twice from the chapel on the hill. The fox watches the sky shift from gold to a deepening slate blue, then slips between the ferns and disappears into the hollow beneath the fallen cedar.";

const TEXT_PT_1S: &str = "A rápida raposa marrom pula sobre o cão preguiçoso hoje cedo.";
const TEXT_PT_5S: &str = "A rápida raposa marrom pula sobre o cão preguiçoso hoje cedo, depois retorna pela margem do rio, contornando a grama alta e parando por um instante para ouvir um trovão distante antes de seguir adiante.";
const TEXT_PT_20S: &str = "A rápida raposa marrom pula sobre o cão preguiçoso hoje cedo, depois retorna pela margem do rio, contornando a grama alta e parando por um instante para ouvir um trovão distante. Um bando de estorninhos se ergue do carvalho em espiral, espalhando folhas pelo caminho gasto que leva à velha ponte de pedra. Crianças riem em algum lugar além da cerca viva, enquanto um único sino toca duas vezes na capela da colina. A raposa observa o céu mudar do dourado para um azul ardósia profundo, então desliza entre as samambaias e desaparece na toca sob o cedro caído.";

const TEXT_ES_1S: &str = "El veloz zorro marrón salta sobre el perro perezoso esta mañana temprano.";
const TEXT_ES_5S: &str = "El veloz zorro marrón salta sobre el perro perezoso esta mañana temprano, luego regresa por la ribera del río, atravesando la hierba alta y deteniéndose un momento para escuchar un trueno lejano antes de seguir su camino.";
const TEXT_ES_20S: &str = "El veloz zorro marrón salta sobre el perro perezoso esta mañana temprano, luego regresa por la ribera del río, atravesando la hierba alta y deteniéndose un momento para escuchar un trueno lejano. Una bandada de estorninos se eleva del roble en espiral, esparciendo hojas por el sendero gastado que conduce al viejo puente de piedra. Los niños ríen en algún lugar más allá del seto, mientras una sola campana suena dos veces en la capilla de la colina. El zorro observa cómo el cielo pasa del dorado a un azul pizarra profundo, luego se desliza entre los helechos y desaparece en la hondonada bajo el cedro caído.";

struct Cell {
    voice: &'static str,
    duration: &'static str,
    text: &'static str,
}

const CELLS: &[Cell] = &[
    Cell { voice: "af_heart", duration: "1s",  text: TEXT_EN_1S },
    Cell { voice: "af_heart", duration: "5s",  text: TEXT_EN_5S },
    Cell { voice: "af_heart", duration: "20s", text: TEXT_EN_20S },
    Cell { voice: "pf_dora",  duration: "1s",  text: TEXT_PT_1S },
    Cell { voice: "pf_dora",  duration: "5s",  text: TEXT_PT_5S },
    Cell { voice: "pf_dora",  duration: "20s", text: TEXT_PT_20S },
    Cell { voice: "ef_alice", duration: "1s",  text: TEXT_ES_1S },
    Cell { voice: "ef_alice", duration: "5s",  text: TEXT_ES_5S },
    Cell { voice: "ef_alice", duration: "20s", text: TEXT_ES_20S },
];

fn bench_synth(c: &mut Criterion) {
    if !LiveTestAssets::is_configured() {
        eprintln!("live assets not configured; skipping bench");
        std::process::exit(0);
    }

    let assets = LiveTestAssets::from_env();
    assets.install_onnxruntime_env();

    let mut cache = SynthesisCache::new(
        assets.espeak_runtime_dir.join("espeak-ng-data"),
        ExecutionProvider::Cpu,
    );

    let mut group = c.benchmark_group("synth");
    group.sample_size(50);
    group.measurement_time(Duration::from_secs(30));
    group.warm_up_time(Duration::from_secs(3));

    for cell in CELLS {
        let resolved = resolve_model_assets(&assets.model_dir, cell.voice)
            .unwrap_or_else(|e| panic!("resolve_model_assets({}) failed: {}", cell.voice, e));

        // Warm-up outside the measured closure: triggers ORT init + model load.
        let _ = cache
            .synthesize(cell.text, &resolved, 1.0)
            .unwrap_or_else(|e| panic!("warm-up synthesize({}/{}) failed: {}", cell.voice, cell.duration, e));

        let id = BenchmarkId::new(cell.voice, cell.duration);
        group.bench_function(id, |b| {
            b.iter(|| {
                cache
                    .synthesize(cell.text, &resolved, 1.0)
                    .expect("synthesize should succeed")
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_synth);
criterion_main!(benches);
