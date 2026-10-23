use std::collections::HashMap;
use std::path::{Path, PathBuf};

use candle_core::pickle::PthTensors;
use candle_core::{DType, Device, Shape, Tensor};
use candle_nn::var_builder::SimpleBackend;
use candle_nn::VarBuilder;
use serde::Deserialize;

use crate::{Error, Result};

/// NLLB weights, in PyTorch or safetensors format, optionally sharded.
pub enum Checkpoint {
    SafeTensors(Vec<PathBuf>),
    Pth(Vec<PathBuf>),
}

impl Checkpoint {
    pub fn discover(model_dir: &Path) -> Result<Self> {
        if !model_dir.is_dir() {
            return Err(Error::ModelNotFound(model_dir.to_path_buf()));
        }

        if model_dir.join("model.safetensors").is_file() {
            return Ok(Self::SafeTensors(vec![model_dir.join("model.safetensors")]));
        }
        if model_dir.join("model.safetensors.index.json").is_file() {
            let index = read_safetensors_index(&model_dir.join("model.safetensors.index.json"))?;
            return Ok(Self::SafeTensors(index));
        }
        if model_dir.join("pytorch_model.bin").is_file() {
            return Ok(Self::Pth(vec![model_dir.join("pytorch_model.bin")]));
        }
        if model_dir.join("pytorch_model.bin.index.json").is_file() {
            let path = model_dir.join("pytorch_model.bin.index.json");
            let index = read_pth_index(&path)?;
            let mut shards: Vec<PathBuf> = index.values().cloned().collect();
            shards.sort();
            shards.dedup();
            if shards.is_empty() {
                return Err(Error::MissingModelFile {
                    dir: model_dir.to_path_buf(),
                    hint: "empty weight index".to_string(),
                });
            }
            return Ok(Self::Pth(shards));
        }

        // Last resort: look for HF-style shard names.
        let mut shards = Vec::new();
        if let Ok(entries) = std::fs::read_dir(model_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.starts_with("pytorch_model-") && name.ends_with(".bin") {
                    shards.push(entry.path());
                }
            }
        }
        if !shards.is_empty() {
            shards.sort();
            return Ok(Self::Pth(shards));
        }

        Err(Error::MissingModelFile {
            dir: model_dir.to_path_buf(),
            hint: "expected pytorch_model.bin, pytorch_model.bin.index.json or model.safetensors"
                .to_string(),
        })
    }

    pub fn var_builder(&self, dtype: DType, device: &Device) -> Result<VarBuilder<'static>> {
        match self {
            Self::SafeTensors(paths) => {
                // Safe: the files are not modified while loading.
                let vb = unsafe {
                    VarBuilder::from_mmaped_safetensors(paths.as_slice(), dtype, device)?
                };
                Ok(vb)
            }
            Self::Pth(paths) => {
                let index = if paths.len() > 1 {
                    let index_path = paths[0]
                        .parent()
                        .map(|dir| dir.join("pytorch_model.bin.index.json"))
                        .filter(|path| path.is_file());
                    index_path.as_deref().map(read_pth_index).transpose()?
                } else {
                    None
                };
                let backend = ShardedPth::new(paths, index)?;
                Ok(VarBuilder::from_backend(
                    Box::new(backend),
                    dtype,
                    device.clone(),
                ))
            }
        }
    }

    pub fn on_disk_bytes(&self) -> u64 {
        let paths = match self {
            Self::SafeTensors(paths) | Self::Pth(paths) => paths,
        };
        paths
            .iter()
            .filter_map(|path| std::fs::metadata(path).ok())
            .map(|metadata| metadata.len())
            .sum()
    }
}

struct ShardedPth {
    shards: Vec<PthTensors>,
    route: HashMap<String, usize>,
}

impl ShardedPth {
    fn new(paths: &[PathBuf], index: Option<HashMap<String, PathBuf>>) -> Result<Self> {
        let mut shards = Vec::with_capacity(paths.len());
        for path in paths {
            let pth = PthTensors::new(path, None).map_err(|error| {
                Error::Other(format!(
                    "could not read checkpoint {}: {error}",
                    path.display()
                ))
            })?;
            shards.push(pth);
        }

        let mut route = HashMap::new();
        if let Some(index) = index {
            for (name, path) in index {
                if let Some(shard) = paths.iter().position(|candidate| candidate == &path) {
                    route.insert(name, shard);
                }
            }
        } else {
            for (shard_index, shard) in shards.iter().enumerate() {
                for name in shard.tensor_infos().keys() {
                    route.entry(name.clone()).or_insert(shard_index);
                }
            }
        }

        Ok(Self { shards, route })
    }

    fn load(&self, name: &str, dtype: DType, device: &Device) -> candle_core::Result<Tensor> {
        let tensor = if let Some(shard) = self.route.get(name) {
            self.shards[*shard].get(name)?
        } else {
            let mut found = None;
            for shard in &self.shards {
                if let Some(tensor) = shard.get(name)? {
                    found = Some(tensor);
                    break;
                }
            }
            found
        };

        let Some(tensor) = tensor else {
            return Err(candle_core::Error::CannotFindTensor {
                path: name.to_string(),
            });
        };
        tensor.to_dtype(dtype)?.to_device(device)
    }
}

impl SimpleBackend for ShardedPth {
    fn get(
        &self,
        shape: Shape,
        name: &str,
        _hints: candle_nn::Init,
        dtype: DType,
        device: &Device,
    ) -> candle_core::Result<Tensor> {
        let tensor = self.load(name, dtype, device)?;
        if tensor.shape() != &shape {
            return Err(candle_core::Error::UnexpectedShape {
                msg: format!("unexpected shape for {name}"),
                expected: shape,
                got: tensor.shape().clone(),
            });
        }
        Ok(tensor)
    }

    fn get_unchecked(
        &self,
        name: &str,
        dtype: DType,
        device: &Device,
    ) -> candle_core::Result<Tensor> {
        self.load(name, dtype, device)
    }

    fn contains_tensor(&self, name: &str) -> bool {
        self.route.contains_key(name)
    }
}

fn read_pth_index(path: &Path) -> Result<HashMap<String, PathBuf>> {
    #[derive(Deserialize)]
    struct Index {
        weight_map: HashMap<String, String>,
    }
    let raw = std::fs::read_to_string(path)?;
    let index: Index = serde_json::from_str(&raw)?;
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    Ok(index
        .weight_map
        .into_iter()
        .map(|(name, file)| (name, base.join(file)))
        .collect())
}

fn read_safetensors_index(path: &Path) -> Result<Vec<PathBuf>> {
    let index = read_pth_index(path)?;
    let mut files: Vec<PathBuf> = index.into_values().collect();
    files.sort();
    files.dedup();
    Ok(files)
}
