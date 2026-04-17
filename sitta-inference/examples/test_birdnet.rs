use std::path::Path;

use sitta_inference::birdnet::BirdNet;
use sitta_inference::model::Classifier;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (model_path, labels_path) = match args.len() {
        3 => (args[1].as_str(), args[2].as_str()),
        _ => {
            eprintln!("Usage: test_birdnet <model.onnx> <labels.txt>");
            std::process::exit(1);
        }
    };

    println!("Loading BirdNET model via birdnet-onnx...");

    let model =
        BirdNet::load(Path::new(model_path), Path::new(labels_path), 0.1, 10)
            .expect("failed to load model");

    let sample_count = model.window_samples();
    println!(
        "Model: {}, sample_rate: {} Hz, window: {} samples",
        model.name(),
        model.sample_rate(),
        sample_count
    );

    // Test 1: silence (should produce no/few detections)
    println!("\n--- Test 1: Silence ---");
    let silence = vec![0.0f32; sample_count];
    let detections = model.classify(&silence).expect("inference failed");
    println!("{} detections above 0.1 threshold", detections.len());
    for d in detections.iter().take(3) {
        println!(
            "  {:.3}  {} ({})",
            d.confidence, d.species.common_name, d.species.scientific_name
        );
    }

    // Test 2: synthetic noise
    println!("\n--- Test 2: Synthetic noise ---");
    let noise: Vec<f32> = (0..sample_count)
        .map(|i| (i as f32 * 0.1).sin() * 0.5 + (i as f32 * 0.037).cos() * 0.3)
        .collect();
    let detections = model.classify(&noise).expect("inference failed");
    println!("{} detections above 0.1 threshold", detections.len());
    for d in detections.iter().take(5) {
        println!(
            "  {:.3}  {} ({})",
            d.confidence, d.species.common_name, d.species.scientific_name
        );
    }

    // Test 3: timing
    println!("\n--- Test 3: Inference timing ---");
    let start = std::time::Instant::now();
    let n = 10;
    for _ in 0..n {
        let _ = model.classify(&silence).expect("inference failed");
    }
    let elapsed = start.elapsed();
    println!(
        "{n} inferences in {:.2}s ({:.0}ms each)",
        elapsed.as_secs_f64(),
        elapsed.as_secs_f64() / n as f64 * 1000.0
    );

    // Test 4: embeddings (if supported by model)
    println!("\n--- Test 4: Embeddings ---");
    let (dets, embeddings) = model
        .classify_with_embeddings(&silence)
        .expect("inference failed");
    match embeddings {
        Some(emb) => println!("Embedding dimension: {} (from {} detections)", emb.len(), dets.len()),
        None => println!("No embeddings (expected for BirdNET v2.4)"),
    }
}
