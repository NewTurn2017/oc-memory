#!/usr/bin/env python3
"""
Prepare ONNX model assets for oc-memory.

Default mode downloads prebuilt INT8 ONNX files (fast, stable).
Optional mode performs local export/quantization via --convert.

Usage:
    python scripts/download_model.py
    python scripts/download_model.py --convert

Output:
    ~/.local/share/oc-memory/models/bge-m3-ko-int8.onnx
    ~/.local/share/oc-memory/models/tokenizer.json
"""

from __future__ import annotations

import argparse
import shutil
import sys
from pathlib import Path
from urllib.request import urlretrieve

MODEL_ID_CONVERT = "dragonkue/BGE-m3-ko"
PREBUILT_MODEL_URL = "https://huggingface.co/Xenova/bge-m3/resolve/main/onnx/model_int8.onnx"
PREBUILT_TOKENIZER_URL = "https://huggingface.co/Xenova/bge-m3/resolve/main/tokenizer.json"

OUTPUT_DIR = Path.home() / ".local" / "share" / "oc-memory" / "models"
TEMP_DIR = Path("/tmp/oc-memory-model-export")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Prepare model files for oc-memory")
    parser.add_argument(
        "--convert",
        action="store_true",
        help="Export and quantize from Hugging Face model locally instead of downloading prebuilt files",
    )
    return parser.parse_args()


def require_imports(modules: list[str]) -> None:
    missing: list[str] = []
    for module in modules:
        try:
            __import__(module)
        except ImportError:
            missing.append(module)

    if missing:
        print(f"Missing Python modules: {', '.join(missing)}")
        print("Install dependencies with scripts/setup_model.sh")
        sys.exit(1)


def download_prebuilt() -> tuple[Path, Path]:
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    dst_model = OUTPUT_DIR / "bge-m3-ko-int8.onnx"
    dst_tokenizer = OUTPUT_DIR / "tokenizer.json"

    print("[1/3] Downloading prebuilt INT8 model...")
    urlretrieve(PREBUILT_MODEL_URL, dst_model)
    print(f"  -> {dst_model}")

    print("[2/3] Downloading tokenizer...")
    urlretrieve(PREBUILT_TOKENIZER_URL, dst_tokenizer)
    print(f"  -> {dst_tokenizer}")

    return dst_model, dst_tokenizer


def convert_locally() -> tuple[Path, Path]:
    require_imports(["optimum", "onnxruntime", "transformers", "tokenizers"])
    from optimum.onnxruntime import ORTModelForFeatureExtraction
    from optimum.onnxruntime import ORTQuantizer
    from optimum.onnxruntime.configuration import AutoQuantizationConfig
    from transformers import AutoTokenizer

    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    TEMP_DIR.mkdir(parents=True, exist_ok=True)

    onnx_export_dir = TEMP_DIR / "onnx-fp32"
    onnx_int8_dir = TEMP_DIR / "onnx-int8"

    print(f"[1/4] Exporting {MODEL_ID_CONVERT} to ONNX (FP32)...")
    model = ORTModelForFeatureExtraction.from_pretrained(MODEL_ID_CONVERT, export=True)
    model.save_pretrained(str(onnx_export_dir))
    tokenizer = AutoTokenizer.from_pretrained(MODEL_ID_CONVERT)
    tokenizer.save_pretrained(str(onnx_export_dir))

    print("[2/4] Quantizing to INT8...")
    quantizer = ORTQuantizer.from_pretrained(str(onnx_export_dir))
    qconfig = AutoQuantizationConfig.avx2(is_static=False, per_channel=True)
    onnx_int8_dir.mkdir(parents=True, exist_ok=True)
    quantizer.quantize(save_dir=str(onnx_int8_dir), quantization_config=qconfig)

    print("[3/4] Copying final model files...")
    int8_model = onnx_int8_dir / "model_quantized.onnx"
    if not int8_model.exists():
        alt_model = onnx_int8_dir / "model.onnx"
        int8_model = alt_model if alt_model.exists() else int8_model
    if not int8_model.exists():
        model_candidates = list(onnx_int8_dir.glob("*.onnx"))
        if not model_candidates:
            print("ERROR: No ONNX file found after quantization")
            sys.exit(1)
        int8_model = model_candidates[0]

    src_tokenizer = onnx_export_dir / "tokenizer.json"
    if not src_tokenizer.exists():
        print("ERROR: tokenizer.json not found after export")
        sys.exit(1)

    dst_model = OUTPUT_DIR / "bge-m3-ko-int8.onnx"
    dst_tokenizer = OUTPUT_DIR / "tokenizer.json"
    shutil.copy2(str(int8_model), str(dst_model))
    shutil.copy2(str(src_tokenizer), str(dst_tokenizer))

    print("[4/4] Cleaning temporary files...")
    shutil.rmtree(str(TEMP_DIR), ignore_errors=True)

    return dst_model, dst_tokenizer


def verify_model(model_path: Path, tokenizer_path: Path) -> None:
    require_imports(["onnxruntime"])
    import onnxruntime as ort

    model_size_mb = model_path.stat().st_size / (1024 * 1024)
    tokenizer_size_kb = tokenizer_path.stat().st_size / 1024
    print("Verification:")
    print(f"  Model size: {model_size_mb:.1f} MB")
    print(f"  Tokenizer size: {tokenizer_size_kb:.1f} KB")

    session = ort.InferenceSession(str(model_path))
    input_names = [i.name for i in session.get_inputs()]
    output_names = [o.name for o in session.get_outputs()]
    print(f"  Inputs: {input_names}")
    print(f"  Outputs: {output_names}")


def main() -> None:
    args = parse_args()

    if args.convert:
        print("Using local conversion mode (--convert)")
        model_path, tokenizer_path = convert_locally()
    else:
        print("Using prebuilt download mode (default)")
        model_path, tokenizer_path = download_prebuilt()

    verify_model(model_path, tokenizer_path)
    print(f"\nDone. Model assets are ready in: {OUTPUT_DIR}")


if __name__ == "__main__":
    main()
