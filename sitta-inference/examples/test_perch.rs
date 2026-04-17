use std::path::Path;

use sitta_inference::birdnet::BirdNet;
use sitta_inference::model::Classifier;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (model_path, labels_path) = match args.len() {
        3 => (args[1].as_str(), args[2].as_str()),
        _ => {
            eprintln!("Usage: test_perch <model.onnx> <labels.csv>");
            std::process::exit(1);
        }
    };

    println!("Loading Perch model via birdnet-onnx...");

    let model = BirdNet::load(Path::new(model_path), Path::new(labels_path), 0.1, 10)
        .expect("failed to load model");

    let sample_count = model.window_samples();
    println!(
        "Model: {}, sample_rate: {} Hz, window: {} samples ({:.1}s)",
        model.name(),
        model.sample_rate(),
        sample_count,
        sample_count as f64 / model.sample_rate() as f64,
    );

    assert_eq!(
        model.sample_rate(),
        32_000,
        "Perch expects 32 kHz, got {}",
        model.sample_rate()
    );
    assert_eq!(
        sample_count, 160_000,
        "Perch expects 160k samples (5s @ 32kHz), got {}",
        sample_count
    );

    // Test 1: silence
    println!("\n--- Test 1: Silence ---");
    let silence = vec![0.0f32; sample_count];
    let (detections, embeddings) = model
        .classify_with_embeddings(&silence)
        .expect("inference failed");
    println!("{} detections above 0.1 threshold", detections.len());
    for d in detections.iter().take(3) {
        println!(
            "  {:.3}  {} ({})",
            d.confidence, d.species.common_name, d.species.scientific_name
        );
    }
    match &embeddings {
        Some(emb) => println!("Embeddings: {} dimensions", emb.len()),
        None => panic!("Expected embeddings from Perch model, got None"),
    }

    // Test 2: synthetic noise at 32 kHz
    println!("\n--- Test 2: Synthetic noise ---");
    let noise: Vec<f32> = (0..sample_count)
        .map(|i| (i as f32 * 0.1).sin() * 0.5 + (i as f32 * 0.037).cos() * 0.3)
        .collect();
    let (detections, embeddings) = model
        .classify_with_embeddings(&noise)
        .expect("inference failed");
    println!("{} detections above 0.1 threshold", detections.len());
    for d in detections.iter().take(5) {
        println!(
            "  {:.3}  {} ({})",
            d.confidence, d.species.common_name, d.species.scientific_name
        );
    }
    let emb = embeddings.expect("Expected embeddings from Perch model");
    println!("Embeddings: {} dimensions", emb.len());

    // Test 3: inference timing
    println!("\n--- Test 3: Inference timing ---");
    let start = std::time::Instant::now();
    let n = 5;
    for _ in 0..n {
        let _ = model
            .classify_with_embeddings(&silence)
            .expect("inference failed");
    }
    let elapsed = start.elapsed();
    println!(
        "{n} inferences in {:.2}s ({:.0}ms each)",
        elapsed.as_secs_f64(),
        elapsed.as_secs_f64() / n as f64 * 1000.0
    );
}
