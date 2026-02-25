use ort::session::Session;
use ort::value::Tensor;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;
use tokenizers::Tokenizer;

use crate::error::{EmbeddingError, Result};

/// BGE-m3-ko ONNX embedding engine
///
/// Loads an INT8 quantized ONNX model and tokenizer for Korean text embedding.
/// Produces 1024-dimensional dense vectors.
pub struct EmbeddingEngine {
    /// Session requires &mut self for run(), so wrap in Mutex for thread safety
    session: Mutex<Session>,
    tokenizer: Tokenizer,
    dimensions: usize,
    max_length: usize,
}

impl EmbeddingEngine {
    /// Initialize the embedding engine with model and tokenizer paths
    pub fn new(
        model_path: impl AsRef<Path>,
        tokenizer_path: impl AsRef<Path>,
        dimensions: usize,
        max_length: usize,
        num_threads: usize,
    ) -> Result<Self> {
        let model_path = model_path.as_ref();
        let tokenizer_path = tokenizer_path.as_ref();

        if !model_path.exists() {
            return Err(EmbeddingError::ModelNotFound(
                model_path.display().to_string(),
            ));
        }

        if !tokenizer_path.exists() {
            return Err(EmbeddingError::ModelNotFound(
                tokenizer_path.display().to_string(),
            ));
        }

        tracing::info!(
            model = %model_path.display(),
            threads = num_threads,
            "Loading ONNX embedding model"
        );

        // ort 2.0 API: builder -> options -> commit_from_file
        let session = Session::builder()?
            .with_intra_threads(num_threads)?
            .commit_from_file(model_path)?;

        let tokenizer = Tokenizer::from_file(tokenizer_path)
            .map_err(|e| EmbeddingError::Tokenizer(e.to_string()))?;

        tracing::info!(dimensions, max_length, "Embedding engine ready");

        Ok(Self {
            session: Mutex::new(session),
            tokenizer,
            dimensions,
            max_length,
        })
    }

    /// Embed a single text string
    pub fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let embeddings = self.embed_batch(&[text])?;
        Ok(embeddings.into_iter().next().unwrap())
    }

    /// Embed a batch of text strings
    pub fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        // Tokenize
        let encodings = self
            .tokenizer
            .encode_batch(texts.to_vec(), true)
            .map_err(|e| EmbeddingError::Tokenizer(e.to_string()))?;

        let batch_size = encodings.len();
        let max_len = encodings
            .iter()
            .map(|e| e.get_ids().len().min(self.max_length))
            .max()
            .unwrap_or(0);

        // Prepare input data as flat vectors
        let mut input_ids_data = vec![0i64; batch_size * max_len];
        let mut attention_mask_data = vec![0i64; batch_size * max_len];
        let token_type_ids_data = vec![0i64; batch_size * max_len];

        for (i, encoding) in encodings.iter().enumerate() {
            let ids = encoding.get_ids();
            let mask = encoding.get_attention_mask();
            let len = ids.len().min(max_len);

            for j in 0..len {
                input_ids_data[i * max_len + j] = ids[j] as i64;
                attention_mask_data[i * max_len + j] = mask[j] as i64;
            }
        }

        // Create ort Tensor values using from_array((shape, data))
        let shape = vec![batch_size as i64, max_len as i64];

        let input_ids = Tensor::from_array((shape.clone(), input_ids_data.into_boxed_slice()))
            .map_err(EmbeddingError::OnnxRuntime)?;

        let attention_mask =
            Tensor::from_array((shape.clone(), attention_mask_data.into_boxed_slice()))
                .map_err(EmbeddingError::OnnxRuntime)?;

        let token_type_ids = Tensor::from_array((shape, token_type_ids_data.into_boxed_slice()))
            .map_err(EmbeddingError::OnnxRuntime)?;

        // Run inference — session.run requires &mut self
        // We extract data into owned arrays within the session scope
        let (output_shape, output_data) = {
            let mut session = self
                .session
                .lock()
                .map_err(|e| EmbeddingError::Tokenizer(format!("Session lock poisoned: {e}")))?;

            let has_token_type_ids = session
                .inputs()
                .iter()
                .any(|input| input.name() == "token_type_ids");

            let outputs = if has_token_type_ids {
                session.run(ort::inputs![
                    "input_ids" => input_ids,
                    "attention_mask" => attention_mask,
                    "token_type_ids" => token_type_ids,
                ])?
            } else {
                session.run(ort::inputs![
                    "input_ids" => input_ids,
                    "attention_mask" => attention_mask,
                ])?
            };

            // Extract output tensor and copy to owned data
            let output_value = &outputs[0];
            let output_tensor = output_value
                .try_extract_array::<f32>()
                .map_err(|e| EmbeddingError::Tokenizer(format!("Failed to extract output: {e}")))?;

            let shape = output_tensor.shape().to_vec();
            let data = output_tensor
                .as_slice()
                .map(|s| s.to_vec())
                .unwrap_or_else(|| output_tensor.iter().copied().collect());
            (shape, data)
            // session + outputs dropped here
        };

        let hidden_dim = if output_shape.len() == 3 {
            output_shape[2]
        } else {
            self.dimensions
        };
        let seq_len_total = if output_shape.len() == 3 {
            output_shape[1]
        } else {
            max_len
        };
        let mut results = Vec::with_capacity(batch_size);

        for (i, encoding) in encodings.iter().enumerate().take(batch_size) {
            let actual_seq_len = encoding.get_ids().len().min(max_len);
            let mut embedding = vec![0f32; hidden_dim];

            // Mean pooling over non-padding tokens using flat output_data
            let batch_offset = i * seq_len_total * hidden_dim;
            for j in 0..actual_seq_len {
                let token_offset = batch_offset + j * hidden_dim;
                for k in 0..hidden_dim {
                    embedding[k] += output_data[token_offset + k];
                }
            }
            if actual_seq_len > 0 {
                for val in &mut embedding {
                    *val /= actual_seq_len as f32;
                }
            }

            // L2 normalize
            let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
            if norm > 0.0 {
                for val in &mut embedding {
                    *val /= norm;
                }
            }

            results.push(embedding);
        }

        Ok(results)
    }

    /// Vector dimensions
    pub fn dimensions(&self) -> usize {
        self.dimensions
    }
}

/// Create a shared embedding engine from config
pub fn create_engine(config: &oc_core::config::EmbeddingConfig) -> Result<Arc<EmbeddingEngine>> {
    let model_path = shellexpand(&config.model_path);
    let tokenizer_path = shellexpand(&config.tokenizer_path);

    let engine = EmbeddingEngine::new(
        &model_path,
        &tokenizer_path,
        config.dimensions,
        config.max_length,
        config.num_threads,
    )?;

    Ok(Arc::new(engine))
}

fn shellexpand(path: &str) -> String {
    if path.starts_with("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return path.replacen("~", &home.to_string_lossy(), 1);
    }
    path.to_string()
}
