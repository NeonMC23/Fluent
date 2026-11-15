//! Chat model catalogue.
//!
//! Chat models are intentionally separate from translation models. They are
//! quantized causal instruct models with their own directories, manifests and
//! lifecycle.

/// One required chat model file. Chat files may come from different upstream
/// repositories (the GGUF and the tokenizer are usually separate).
#[derive(Debug, Clone, Copy)]
pub struct ChatModelFile {
    pub name: &'static str,
    pub size: u64,
    pub sha256: &'static str,
    pub repo: &'static str,
    pub revision: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct ChatModelSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: &'static str,
    pub quantization: &'static str,
    pub storage: &'static str,
    pub license: &'static str,
    pub default_dir: &'static str,
    /// Context budget Fluent uses for prompt plus generated tokens.
    pub context_tokens: usize,
    pub files: &'static [ChatModelFile],
}

impl ChatModelSpec {
    pub fn download_bytes(self) -> u64 {
        self.files.iter().map(|file| file.size).sum()
    }
}

pub const CHAT_MODELS: &[ChatModelSpec] = &[
    ChatModelSpec {
        id: "qwen2.5-1.5b-instruct",
        name: "Qwen2.5 1.5B Instruct",
        description: "Better-quality local chat",
        parameters: "1.5B",
        quantization: "Q4_K_M",
        storage: "1.1 GB Q4_K_M GGUF",
        license: "Apache-2.0",
        default_dir: "qwen2.5-1.5b-instruct",
        context_tokens: 4096,
        files: &[
            ChatModelFile {
                name: "qwen2.5-1.5b-instruct-q4_k_m.gguf",
                size: 1_117_320_736,
                sha256: "6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e",
                repo: "Qwen/Qwen2.5-1.5B-Instruct-GGUF",
                revision: "91cad51170dc346986eccefdc2dd33a9da36ead9",
            },
            ChatModelFile {
                name: "tokenizer.json",
                size: 7_031_645,
                sha256: "c0382117ea329cdf097041132f6d735924b697924d6f6fc3945713e96ce87539",
                repo: "Qwen/Qwen2.5-1.5B-Instruct",
                revision: "989aa7980e4cf806f80c7fef2b1adb7bc71aa306",
            },
            ChatModelFile {
                name: "tokenizer_config.json",
                size: 7_305,
                sha256: "5b5d4f65d0acd3b2d56a35b56d374a36cbc1c8fa5cf3b3febbbfabf22f359583",
                repo: "Qwen/Qwen2.5-1.5B-Instruct",
                revision: "989aa7980e4cf806f80c7fef2b1adb7bc71aa306",
            },
        ],
    },
    ChatModelSpec {
        id: "qwen2.5-0.5b-instruct",
        name: "Qwen2.5 0.5B Instruct",
        description: "Fastest local chat",
        parameters: "0.5B",
        quantization: "Q4_0",
        storage: "0.4 GB Q4_0 GGUF",
        license: "Apache-2.0",
        default_dir: "qwen2.5-0.5b-instruct",
        context_tokens: 4096,
        files: &[
            ChatModelFile {
                name: "qwen2.5-0.5b-instruct-q4_0.gguf",
                size: 428_730_208,
                sha256: "7671c0c304e6ce5a7fc577bcb12aba01e2c155cc2efd29b2213c95b18edaf6ed",
                repo: "Qwen/Qwen2.5-0.5B-Instruct-GGUF",
                revision: "9217f5db79a29953eb74d5343926648285ec7e67",
            },
            ChatModelFile {
                name: "tokenizer.json",
                size: 7_031_645,
                sha256: "c0382117ea329cdf097041132f6d735924b697924d6f6fc3945713e96ce87539",
                repo: "Qwen/Qwen2.5-0.5B-Instruct",
                revision: "7ae557604adf67be50417f59c2c2f167def9a775",
            },
            ChatModelFile {
                name: "tokenizer_config.json",
                size: 7_305,
                sha256: "5b5d4f65d0acd3b2d56a35b56d374a36cbc1c8fa5cf3b3febbbfabf22f359583",
                repo: "Qwen/Qwen2.5-0.5B-Instruct",
                revision: "7ae557604adf67be50417f59c2c2f167def9a775",
            },
        ],
    },
];

pub fn chat_model_spec(id: &str) -> Option<&'static ChatModelSpec> {
    CHAT_MODELS.iter().find(|spec| spec.id == id)
}

pub fn default_chat_model() -> &'static ChatModelSpec {
    &CHAT_MODELS[0]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn chat_models_are_unique_and_sized() {
        let mut ids = HashSet::new();
        let mut dirs = HashSet::new();
        for spec in CHAT_MODELS {
            assert!(ids.insert(spec.id), "duplicate {}", spec.id);
            assert!(
                dirs.insert(spec.default_dir),
                "duplicate dir {}",
                spec.default_dir
            );
            assert!(!spec.files.is_empty());
            assert!(spec.download_bytes() > 0);
            assert!(spec.context_tokens >= 512);
            let mut names = HashSet::new();
            for file in spec.files {
                assert!(names.insert(file.name), "duplicate file {}", file.name);
                assert_eq!(file.sha256.len(), 64);
                assert!(file.size > 0);
                assert!(file.revision.len() == 40);
            }
        }
        assert!(chat_model_spec("missing").is_none());
        assert_eq!(default_chat_model().id, "qwen2.5-1.5b-instruct");
    }
}
