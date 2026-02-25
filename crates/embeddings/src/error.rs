#[derive(Debug, thiserror::Error)]
pub enum EmbeddingError {
    #[error("ONNX Runtime error: {0}")]
    OnnxRuntime(#[from] ort::Error),

    #[error("Tokenizer error: {0}")]
    Tokenizer(String),

    #[error("Model not found at: {0}")]
    ModelNotFound(String),

    #[error("Invalid input: {0}")]
    InvalidInput(String),
}

pub type Result<T> = std::result::Result<T, EmbeddingError>;
