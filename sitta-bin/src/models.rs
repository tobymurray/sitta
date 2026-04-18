use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use sitta_inference::birdnet::BirdNet;
use sitta_inference::model::Classifier;
use sitta_inference::rangefilter::RangeFilter;
use sitta_taxonomy::EbirdTaxonomy;

use crate::config::Config;

pub fn load_taxonomy(config: &Config) -> Result<Option<Arc<EbirdTaxonomy>>> {
    let Some(tax_config) = &config.taxonomy else {
        return Ok(None);
    };
    let taxonomy = EbirdTaxonomy::load(Path::new(&tax_config.ebird_path))
        .with_context(|| format!("failed to load eBird taxonomy: {}", tax_config.ebird_path))?;
    Ok(Some(Arc::new(taxonomy)))
}

pub type BirdnetLoadResult = (Option<Arc<dyn Classifier>>, Option<RangeFilter>);

pub fn load_birdnet(
    config: &Config,
    taxonomy: Option<Arc<EbirdTaxonomy>>,
) -> Result<BirdnetLoadResult> {
    let Some(birdnet_config) = &config.inference.birdnet else {
        return Ok((None, None));
    };

    let model = BirdNet::load_with_taxonomy(
        Path::new(&birdnet_config.model_path),
        Path::new(&birdnet_config.labels_path),
        birdnet_config.min_confidence,
        birdnet_config.top_k,
        taxonomy,
    )
    .context("failed to load BirdNET model")?;

    let range_filter = match (
        &birdnet_config.meta_model_path,
        config.station.latitude,
        config.station.longitude,
    ) {
        (Some(meta_path), Some(lat), Some(lon)) => {
            let force_allow = birdnet_config.force_allow.iter().cloned().collect();
            if !birdnet_config.force_allow.is_empty() && config.taxonomy.is_none() {
                tracing::warn!(
                    codes = ?birdnet_config.force_allow,
                    "force_allow requires [taxonomy] to resolve species codes — \
                     force_allow entries will have no effect without it"
                );
            }
            let filter = RangeFilter::load(
                Path::new(meta_path),
                model.labels(),
                lat,
                lon,
                birdnet_config.meta_threshold,
                force_allow,
            )
            .context("failed to load BirdNET range filter")?;
            Some(filter)
        }
        (Some(_), _, _) => {
            tracing::warn!(
                "meta_model_path is set but [station] latitude/longitude are missing — \
                 range filter disabled"
            );
            None
        }
        _ => None,
    };

    Ok((Some(Arc::new(model)), range_filter))
}

pub fn load_perch(
    config: &Config,
    taxonomy: Option<Arc<EbirdTaxonomy>>,
) -> Result<Option<Arc<dyn Classifier>>> {
    let Some(perch_config) = &config.inference.perch else {
        return Ok(None);
    };
    let model = BirdNet::load_with_taxonomy(
        Path::new(&perch_config.model_path),
        Path::new(&perch_config.labels_path),
        perch_config.min_confidence,
        perch_config.top_k,
        taxonomy,
    )
    .context("failed to load Perch model")?;
    Ok(Some(Arc::new(model)))
}
