#!/usr/bin/env python3
"""
Download and convert BGE-m3-ko to ONNX INT8 for oc-memory.

Usage:
    pip install optimum[exporters] onnxruntime transformers tokenizers
    python scripts/download_model.py

Output:
    ~/.local/share/oc-memory/models/bge-m3-ko-int8.onnx
    ~/.local/share/oc-memory/models/tokenizer.json
"""

import os
import sys
import shutil
from pathlib import Path

MODEL_ID = "dragonkue/BGE-m3-ko"
OUTPUT_DIR = Path.home() / ".local" / "share" / "oc-memory" / "models"
TEMP_DIR = Path("/tmp/oc-memory-model-export")


def check_dependencies():
    missing = []
    try:
        import optimum
    except ImportError:
        missing.append("optimum[exporters]")
    try:
        import onnxruntime
    except ImportError:
        missing.append("onnxruntime")
    try:
        import transformers
    except ImportError:
        missing.append("transformers")
    try:
        import tokenizers
    except ImportError:
        missing.append("tokenizers")

    if missing:
        print(f"Missing dependencies: {', '.join(missing)}")
        print(f"Install with: pip install {' '.join(missing)}")
        sys.exit(1)


def main():
    check_dependencies()

    from optimum.onnxruntime import ORTModelForFeatureExtraction
    from optimum.onnxruntime.configuration import AutoQuantizationConfig
    from optimum.onnxruntime import ORTQuantizer
    from transformers import AutoTokenizer

    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    TEMP_DIR.mkdir(parents=True, exist_ok=True)

    onnx_export_dir = TEMP_DIR / "onnx-fp32"
    onnx_int8_dir = TEMP_DIR / "onnx-int8"

    # Step 1: Export to ONNX (FP32)
    print(f"\n[1/4] Exporting {MODEL_ID} to ONNX (FP32)...")
    model = ORTModelForFeatureExtraction.from_pretrained(
        MODEL_ID, export=True
    )
    model.save_pretrained(str(onnx_export_dir))

    tokenizer = AutoTokenizer.from_pretrained(MODEL_ID)
    tokenizer.save_pretrained(str(onnx_export_dir))
    print(f"  -> Saved FP32 ONNX to {onnx_export_dir}")

    # Step 2: Quantize to INT8 (dynamic quantization)
    print("\n[2/4] Quantizing to INT8 (dynamic quantization)...")
    quantizer = ORTQuantizer.from_pretrained(str(onnx_export_dir))
    qconfig = AutoQuantizationConfig.avx2(is_static=False, per_channel=True)

    onnx_int8_dir.mkdir(parents=True, exist_ok=True)
    quantizer.quantize(save_dir=str(onnx_int8_dir), quantization_config=qconfig)
    print(f"  -> Saved INT8 ONNX to {onnx_int8_dir}")

    # Step 3: Copy final files
    print(f"\n[3/4] Copying model files to {OUTPUT_DIR}...")

    # Find the quantized model file
    int8_model = onnx_int8_dir / "model_quantized.onnx"
    if not int8_model.exists():
        # Try alternative name
        int8_model = onnx_int8_dir / "model.onnx"
    if not int8_model.exists():
        # List what's there
        files = list(onnx_int8_dir.glob("*.onnx"))
        if files:
            int8_model = files[0]
        else:
            print("ERROR: No ONNX file found after quantization!")
            sys.exit(1)

    dst_model = OUTPUT_DIR / "bge-m3-ko-int8.onnx"
    shutil.copy2(str(int8_model), str(dst_model))
    print(f"  -> Model: {dst_model}")

    # Copy tokenizer.json
    src_tokenizer = onnx_export_dir / "tokenizer.json"
    if not src_tokenizer.exists():
        # tokenizers library might have saved it differently
        src_tokenizer = onnx_export_dir / "tokenizer.json"
    dst_tokenizer = OUTPUT_DIR / "tokenizer.json"
    shutil.copy2(str(src_tokenizer), str(dst_tokenizer))
    print(f"  -> Tokenizer: {dst_tokenizer}")

    # Step 4: Verify & report sizes
    print("\n[4/4] Verification:")
    model_size_mb = dst_model.stat().st_size / (1024 * 1024)
    tokenizer_size_kb = dst_tokenizer.stat().st_size / 1024
    print(f"  Model size: {model_size_mb:.1f} MB")
    print(f"  Tokenizer size: {tokenizer_size_kb:.1f} KB")

    # Quick validation: load model with onnxruntime
    import onnxruntime as ort
    session = ort.InferenceSession(str(dst_model))
    inputs = session.get_inputs()
    outputs = session.get_outputs()
    print(f"  Inputs: {[i.name for i in inputs]}")
    print(f"  Outputs: {[o.name for o in outputs]}")
    output_shape = outputs[0].shape
    if len(output_shape) >= 3:
        print(f"  Output dim: {output_shape[-1]}")

    # Cleanup temp
    print(f"\nCleaning up temp dir: {TEMP_DIR}")
    shutil.rmtree(str(TEMP_DIR), ignore_errors=True)

    print(f"\n✅ Done! Model ready at: {OUTPUT_DIR}")
    print(f"\nTo use:")
    print(f"  oc-memory-mcp    # MCP server (stdio)")
    print(f"  oc-memory-server  # REST server (port 6342)")


if __name__ == "__main__":
    main()
