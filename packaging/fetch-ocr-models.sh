#!/usr/bin/env bash
# Downloads OCR model sets and checks them against pinned SHA-256 sums (spec 03, spec 10).
#   packaging/fetch-ocr-models.sh [default|paddle-server] [dest]
# "default" is what releases ship (CI runs this). "paddle-server" is an optional larger set
# (about 200 MB, much slower, more accurate on digits): run it yourself, it is never bundled and
# the app never downloads anything. Then set `ui.ocr_model = "paddle-server"` in config.toml.
# Files land in dest (default "models"): default -> dest/, paddle-server -> dest/paddle-server/.
set -euo pipefail
set_name="${1:-default}"
dest="${2:-models}"

get() { # url sha256 file
  curl -fsSL --retry 3 -o "$3" "$1"
  echo "$2  $3" | sha256sum -c -
}

case "$set_name" in
  default)
    # https://github.com/robertknight/ocrs-models, CC BY-SA 4.0 (THIRD_PARTY_NOTICES.md)
    mkdir -p "$dest"; cd "$dest"
    base=https://ocrs-models.s3-accelerate.amazonaws.com
    get "$base/text-detection.onnx" a917b23dbd9524b465df7e922641b3ff2981623df4ded5a0234004ef2fee7cfe text-detection.onnx
    get "$base/text-recognition.onnx" 86c145c2edb96c8caed5b1ebb8f44d706408922211451c309c625157dd6061c5 text-recognition.onnx
    ;;
  paddle-server)
    # PP-OCRv4 server models via RapidOCR's ONNX export (Apache-2.0), dictionary from PaddleOCR (Apache-2.0).
    mkdir -p "$dest/paddle-server"; cd "$dest/paddle-server"
    hf=https://huggingface.co/SWHL/RapidOCR/resolve/main/PP-OCRv4
    get "$hf/ch_PP-OCRv4_det_server_infer.onnx" cfa39a3f298f6d3fc71789834d15da36d11a6c59b489fc16ea4733728012f786 text-detection.onnx
    get "$hf/ch_PP-OCRv4_rec_server_infer.onnx" 6a2676219be9907c7fc9cf61ebaa843bf2898777def567925b78886fcd90c07a text-recognition.onnx
    get https://raw.githubusercontent.com/PaddlePaddle/PaddleOCR/main/ppocr/utils/ppocr_keys_v1.txt a1c84d9bdb9ab29043c58896224d32941783eb821629618416dcb08f12886492 dict.txt
    ;;
  *) echo "unknown set '$set_name' (default|paddle-server)" >&2; exit 2 ;;
esac
