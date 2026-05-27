# ADR-0010: Default Detection Model

## Status

Accepted 2026-05-27.

## Context

M4 needs one default object detector for pixel-path game perception and the
`minecraft.java` profile. The earlier PRD default was YOLOv10n, but OQ-025
forbids bundling Ultralytics-trained YOLO weights because those weights are not
compatible with the repository's MIT/Apache packaging posture.

The default must be an actual model artifact with a stable URL, SHA-256, class
map, and license record. Operators may still import license-compliant custom or
fine-tuned models later, but the bundled/default path cannot depend on an
unclear YOLO checkpoint.

## Decision

Synapse uses **RT-DETRv2-S COCO ONNX** as the default detection model:

```text
model_id:      rtdetr_v2_s_coco_onnx
source model:  PekingU/rtdetr_v2_r18vd
ONNX artifact: onnx-community/rtdetr_v2_r18vd-ONNX onnx/model.onnx
download URL:  https://huggingface.co/onnx-community/rtdetr_v2_r18vd-ONNX/resolve/main/onnx/model.onnx
sha256:        583a236ac21c95a7fd94f284fc21485e42355bfef82c27011ba78fbc09ee87e2
license:       Apache-2.0
input shape:   [1, 3, 640, 640]
class map:     COCO 80 labels from the Hugging Face config
```

The canonical registry row lives in
`crates/synapse-models/src/registry.rs`. The `minecraft.java` profile now pins
`model_id = "rtdetr_v2_s_coco_onnx"` so profile state and the model registry do
not drift.

YOLOv10n remains an operator-import/local-override option only when the
operator supplies a license-compliant checkpoint and matching SHA. Synapse must
not bundle Ultralytics-trained YOLO weights.

## Rationale

RT-DETRv2-S is materially larger and slower than YOLOv10n, but the available
artifact is Apache-2.0, already exported to ONNX, has a published SHA-256 in
the Hugging Face file metadata, and carries a stable COCO class map. That makes
it safer as the first default model than an unresolved community YOLO
checkpoint.

The model is a general COCO detector, not a Minecraft-specific entity model.
Minecraft-specific labels such as `creeper` and `zombie` still need a future
license-clean fine-tuned model or a profile-specific detector. The default here
solves the license-safe baseline and model registry contract; it does not claim
that COCO labels are sufficient for all Minecraft gameplay semantics.

## Alternatives Considered

- **YOLOv10n / YOLOv8n Ultralytics weights** - rejected as the default because
  OQ-025 forbids bundling those weights.
- **Community-trained YOLOv10n** - deferred because no specific permissively
  licensed artifact with a stable SHA and class map was selected for M4.
- **Florence-2-base detection mode** - deferred to M5/VLM work because it is
  too large and slow for the fast pixel loop.

## Consequences

- Positive: the default model can be represented in code with an exact URL,
  SHA-256, license, filename, input shape, and class map.
- Positive: the Minecraft profile no longer pins a model id that lacks a
  license-safe default artifact.
- Positive: model acquisition can fail closed by SHA before ONNX Runtime loads
  anything.
- Negative: the default is heavier than YOLOv10n and may require asynchronous
  or lower-rate detection on weaker GPUs.
- Negative: Minecraft entity-specific detection remains future work.

## Verification Plan

Manual FSV for this decision reads the physical file bytes for the ADR,
registry source, Minecraft profile, curated package fixture, and perception
docs before and after the edit. Runtime model loading still requires acquiring
the ONNX file on the configured host and reading the model file hash directly
before `ModelLoader` creates a session.

Supporting checks can compile/test the `synapse-models` and `synapse-profiles`
surfaces, but those checks are not FSV.

## Supersedes

- OQ-003 default of YOLOv10n in `docs/computergames/16_open_questions.md`.
- PRD text that said the game profile pins YOLOv10n at 60 Hz.

## References

- Issue: #415
- OQ-003 and OQ-025 in `docs/computergames/16_open_questions.md`
- Hugging Face ONNX artifact: https://huggingface.co/onnx-community/rtdetr_v2_r18vd-ONNX
- Hugging Face source model: https://huggingface.co/PekingU/rtdetr_v2_r18vd
- Official RT-DETR repository: https://github.com/lyuwenyu/RT-DETR
