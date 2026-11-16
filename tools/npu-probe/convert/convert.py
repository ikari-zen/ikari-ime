"""zenz-v3.2-small の GGUF を、形を固定した OpenVINO IR にする（ADR 0075 §4.6。スパイク）。

使い方:
  python -I convert.py <zenz-v3.2-small-Q5_K_M.gguf> <出力のディレクトリ> [--length 128]

出すもの（出力のディレクトリ。リポジトリに入れない）:
  zenz-v3.2-small-L<L>-fp16.xml/.bin  重みを FP16 に圧縮
  zenz-v3.2-small-L<L>-int8.xml/.bin  重みだけ INT8（NNCF の weight compression、INT8_ASYM）
  SHA256SUMS                          上の 4 つと元の GGUF の sha256
  versions.txt                        変換に使った版

GGUF の Q5_K / Q6_K は transformers の GGUF 読み込みで FP32 に戻る（量子化の誤差は元のまま残る）。
入出力の取り決めは tools/npu-probe/README.md の「契約」。
"""

import argparse
import hashlib
import shutil
import tempfile
from pathlib import Path

import nncf
import openvino as ov
import torch
import transformers
from transformers import GPT2LMHeadModel

MODEL_STEM = "zenz-v3.2-small"
INPUT_NAME = "input_ids"
OUTPUT_NAME = "logits"
DEFAULT_LENGTH = 128
PAD_TOKEN = 1


class LogitsOnly(torch.nn.Module):
    """input_ids [1, L] -> logits [1, L, vocab]。KV キャッシュも attention_mask も使わない"""

    def __init__(self, model):
        super().__init__()
        self.model = model

    def forward(self, input_ids):
        return self.model(input_ids=input_ids, use_cache=False).logits


def load_hf(gguf: Path) -> GPT2LMHeadModel:
    # transformers の GGUF 読み込みはディレクトリとファイル名を分けて渡す
    model = GPT2LMHeadModel.from_pretrained(
        str(gguf.parent), gguf_file=gguf.name, attn_implementation="eager", dtype=torch.float32
    )
    model.eval()
    return model


def to_ov(model: GPT2LMHeadModel, length: int) -> ov.Model:
    # TorchScript の trace は transformers 5 のマスクの組み立てで落ちるので、torch.export を通す
    example = torch.full((1, length), PAD_TOKEN, dtype=torch.long)
    with torch.no_grad():
        exported = torch.export.export(LogitsOnly(model), (example,))
        ov_model = ov.convert_model(exported)
    if len(ov_model.inputs) != 1 or len(ov_model.outputs) != 1:
        raise RuntimeError(f"unexpected IO: {ov_model.inputs} {ov_model.outputs}")
    ov_model.reshape({ov_model.inputs[0]: ov.PartialShape([1, length])})
    ov_model.inputs[0].get_tensor().set_names({INPUT_NAME})
    ov_model.outputs[0].get_tensor().set_names({OUTPUT_NAME})
    return ov_model


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("gguf", type=Path)
    ap.add_argument("out", type=Path)
    ap.add_argument("--length", type=int, default=DEFAULT_LENGTH)
    a = ap.parse_args()
    if a.length <= 0:
        ap.error("--length must be positive")
    a.out.mkdir(parents=True, exist_ok=True)

    # GGUF の読み込みは、元のファイルだけの一時ディレクトリから（隣のファイルを読まない）
    with tempfile.TemporaryDirectory() as tmp:
        src = Path(tmp) / a.gguf.name
        shutil.copyfile(a.gguf, src)
        model = load_hf(src)
    ov_model = to_ov(model, a.length)

    stem = f"{MODEL_STEM}-L{a.length}"
    fp16 = a.out / f"{stem}-fp16.xml"
    ov.save_model(ov_model, fp16, compress_to_fp16=True)

    int8_model = nncf.compress_weights(ov_model, mode=nncf.CompressWeightsMode.INT8_ASYM)
    int8 = a.out / f"{stem}-int8.xml"
    ov.save_model(int8_model, int8, compress_to_fp16=True)

    files = [a.gguf, fp16, fp16.with_suffix(".bin"), int8, int8.with_suffix(".bin")]
    (a.out / "SHA256SUMS").write_text(
        "".join(f"{sha256(p)}  {p.name}\n" for p in files), encoding="utf-8"
    )
    (a.out / "versions.txt").write_text(
        f"openvino {ov.get_version()}\nnncf {nncf.__version__}\n"
        f"torch {torch.__version__}\ntransformers {transformers.__version__}\n",
        encoding="utf-8",
    )
    for p in files[1:]:
        print(f"{p.name}\t{p.stat().st_size}")


if __name__ == "__main__":
    main()
