//! Regenerates `catalogue/models-v1.json` from the compiled registry.
//!
//! ```text
//! cargo run --release -p fluent-core --example gen_catalogue \
//!   > catalogue/models-v1.json
//! ```
//!
//! The output is deterministic. A unit test parses the committed file and
//! checks it against the registry, so a stale catalogue fails CI.

use fluent_core::{language_list, ModelSpec, MODELS};
use serde_json::json;

fn license_url(spec: &ModelSpec) -> String {
    match spec.architecture.catalogue_id() {
        "t5" => "https://www.apache.org/licenses/LICENSE-2.0".to_string(),
        "small100" => "https://opensource.org/licenses/MIT".to_string(),
        _ => format!("https://huggingface.co/{}", spec.repo),
    }
}

fn main() {
    let models: Vec<_> = MODELS
        .iter()
        .map(|spec| {
            let files: Vec<_> = spec
                .files
                .iter()
                .map(|file| {
                    json!({
                        "name": file.name,
                        "size": file.size,
                        "sha256": file.sha256,
                        "url": format!(
                            "https://huggingface.co/{}/resolve/{}/{}",
                            spec.repo, spec.revision, file.name
                        ),
                    })
                })
                .collect();
            let languages: Vec<_> = language_list(spec.mode)
                .iter()
                .map(|language| language.code)
                .collect();
            json!({
                "id": spec.id,
                "display_name": spec.name,
                "description": spec.description,
                "backend": spec.architecture.catalogue_id(),
                "revision": spec.revision,
                "license": spec.license,
                "license_url": license_url(spec),
                "model_page": format!("https://huggingface.co/{}", spec.repo),
                "deprecated": false,
                "total_size": spec.download_bytes(),
                "languages": languages,
                "files": files,
            })
        })
        .collect();

    let catalogue = json!({
        "schema_version": 1,
        "generated_at": "2026-11-15T00:00:00Z",
        "minimum_app_version": "0.2.0",
        "models": models,
    });
    println!("{}", serde_json::to_string_pretty(&catalogue).unwrap());
}
