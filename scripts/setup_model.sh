#!/bin/bash
# Setup BGE-m3-ko ONNX INT8 model for oc-memory
# Usage: bash scripts/setup_model.sh

set -euo pipefail

MODEL_DIR="$HOME/.local/share/oc-memory/models"

echo "=== oc-memory Model Setup ==="
echo ""

# Check if model already exists
if [ -f "$MODEL_DIR/bge-m3-ko-int8.onnx" ] && [ -f "$MODEL_DIR/tokenizer.json" ]; then
    echo "Model already exists at $MODEL_DIR"
    echo "  Model: $(du -h "$MODEL_DIR/bge-m3-ko-int8.onnx" | cut -f1)"
    echo "  Tokenizer: $(du -h "$MODEL_DIR/tokenizer.json" | cut -f1)"
    echo ""
    read -p "Re-download? (y/N) " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        echo "Skipping. Model is ready."
        exit 0
    fi
fi

# Check Python
if ! command -v python3 &>/dev/null; then
    echo "ERROR: python3 not found. Install Python 3.9+ first."
    exit 1
fi

echo "Installing Python dependencies..."
pip3 install --quiet "optimum[exporters]" onnxruntime transformers tokenizers

echo ""
echo "Downloading and converting model..."
python3 "$(dirname "$0")/download_model.py"
