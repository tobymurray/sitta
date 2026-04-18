//! eBird taxonomy lookup for resolving scientific names to common names and species codes.

use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
pub enum TaxonomyError {
    #[error("failed to open taxonomy file: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse taxonomy CSV: {0}")]
    Csv(#[from] csv::Error),
}

/// A single taxon entry from the eBird taxonomy.
#[derive(Debug, Clone)]
pub struct TaxonEntry {
    /// eBird species code (e.g., "barowl1").
    pub species_code: String,
    /// English common name (e.g., "Barn Owl").
    pub common_name: String,
    /// Scientific name with canonical casing (e.g., "Tyto alba").
    pub scientific_name: String,
}

/// eBird taxonomy indexed by normalized scientific name for fast lookup.
pub struct EbirdTaxonomy {
    /// Keyed by lowercase scientific name with spaces (underscores already normalized).
    entries: HashMap<String, TaxonEntry>,
}

#[derive(Deserialize)]
struct TaxonRecord {
    // eBird taxonomy API format (fmt=csv)
    #[serde(rename = "SPECIES_CODE", alias = "species_code")]
    species_code: String,
    // eBird API uses PRIMARY_COM_NAME; Clements integrated checklist uses "English name"
    #[serde(rename = "PRIMARY_COM_NAME", alias = "English name")]
    common_name: String,
    // eBird API uses SCI_NAME; Clements integrated checklist uses "scientific name"
    #[serde(rename = "SCI_NAME", alias = "scientific name")]
    sci_name: String,
}

impl EbirdTaxonomy {
    /// Load the eBird taxonomy from a CSV file.
    ///
    /// Expected columns: `TAXON_ORDER`, `CATEGORY`, `SPECIES_CODE`,
    /// `PRIMARY_COM_NAME`, `SCI_NAME`, `ORDER1`, `FAMILY`, `SPECIES_GROUP`, `REPORT_AS`.
    /// Download from: https://api.ebird.org/v2/ref/taxonomy/ebird?fmt=csv
    pub fn load(path: &Path) -> Result<Self, TaxonomyError> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let mut csv_reader = csv::Reader::from_reader(reader);

        let mut entries = HashMap::new();
        for result in csv_reader.deserialize::<TaxonRecord>() {
            let record = result?;
            let key = normalize(&record.sci_name);
            entries.insert(
                key,
                TaxonEntry {
                    species_code: record.species_code,
                    common_name: record.common_name,
                    scientific_name: record.sci_name,
                },
            );
        }

        tracing::info!(count = entries.len(), "Loaded eBird taxonomy");
        Ok(Self { entries })
    }

    /// Create a taxonomy from pre-built entries (for testing).
    #[cfg(any(test, feature = "test-util"))]
    pub fn from_entries(entries: Vec<TaxonEntry>) -> Self {
        let map = entries
            .into_iter()
            .map(|e| (normalize(&e.scientific_name), e))
            .collect();
        Self { entries: map }
    }

    /// Look up a taxon by scientific name.
    ///
    /// Accepts both space-separated (`"Tyto alba"`) and underscore-separated
    /// (`"Tyto_alba"`) forms; the lookup is case-insensitive.
    pub fn lookup(&self, scientific_name: &str) -> Option<&TaxonEntry> {
        self.entries.get(&normalize(scientific_name))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Normalize a scientific name for consistent map keys.
/// Replaces underscores with spaces and lowercases.
fn normalize(name: &str) -> String {
    name.replace('_', " ").to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_space_separated() {
        assert_eq!(normalize("Tyto alba"), "tyto alba");
    }

    #[test]
    fn normalize_underscore_separated() {
        assert_eq!(normalize("Tyto_alba"), "tyto alba");
    }

    #[test]
    fn normalize_mixed_case() {
        assert_eq!(normalize("TURDUS_Migratorius"), "turdus migratorius");
    }

    #[test]
    fn lookup_space_form() {
        let tax = EbirdTaxonomy::from_entries(vec![TaxonEntry {
            species_code: "barowl1".into(),
            common_name: "Barn Owl".into(),
            scientific_name: "Tyto alba".into(),
        }]);
        let entry = tax.lookup("Tyto alba").unwrap();
        assert_eq!(entry.species_code, "barowl1");
        assert_eq!(entry.common_name, "Barn Owl");
    }

    #[test]
    fn lookup_underscore_form() {
        let tax = EbirdTaxonomy::from_entries(vec![TaxonEntry {
            species_code: "barowl1".into(),
            common_name: "Barn Owl".into(),
            scientific_name: "Tyto alba".into(),
        }]);
        assert!(tax.lookup("Tyto_alba").is_some());
    }

    #[test]
    fn lookup_case_insensitive() {
        let tax = EbirdTaxonomy::from_entries(vec![TaxonEntry {
            species_code: "barowl1".into(),
            common_name: "Barn Owl".into(),
            scientific_name: "Tyto alba".into(),
        }]);
        assert!(tax.lookup("tyto alba").is_some());
        assert!(tax.lookup("TYTO ALBA").is_some());
    }

    #[test]
    fn lookup_missing() {
        let tax = EbirdTaxonomy::from_entries(vec![]);
        assert!(tax.lookup("Tyto alba").is_none());
    }

    #[test]
    fn len_and_is_empty() {
        let empty = EbirdTaxonomy::from_entries(vec![]);
        assert!(empty.is_empty());
        assert_eq!(empty.len(), 0);

        let one = EbirdTaxonomy::from_entries(vec![TaxonEntry {
            species_code: "x".into(),
            common_name: "X".into(),
            scientific_name: "Genus species".into(),
        }]);
        assert!(!one.is_empty());
        assert_eq!(one.len(), 1);
    }
}
