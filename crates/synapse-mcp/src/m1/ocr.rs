use rmcp::ErrorData;
use synapse_core::{OcrBackend, OcrResult, OcrWord, Rect, error_codes};
use synapse_perception::{TextRegion, read_text, read_text_with_provider};

use crate::m1::{M1State, ReadTextParams, current_input, mcp_error};

pub fn read_text_in_state(state: &M1State, params: ReadTextParams) -> Result<OcrResult, ErrorData> {
    let region = text_region(state, &params)?;
    if state.synthetic.is_some() {
        if region.w <= 0 || region.h <= 0 {
            return Err(mcp_error(error_codes::OCR_NO_TEXT, "OCR produced no text"));
        }
        let provider = SyntheticOcrProvider { region };
        let words = read_text_with_provider(&provider, region)
            .map_err(|err| mcp_error(err.code(), err.to_string()))?;
        return Ok(ocr_result_from_text_regions(
            words,
            params.backend,
            params.lang_hint,
        ));
    }
    let words = read_text(region).map_err(|err| mcp_error(err.code(), err.to_string()))?;
    Ok(ocr_result_from_text_regions(
        words,
        params.backend,
        params.lang_hint,
    ))
}

fn text_region(state: &M1State, params: &ReadTextParams) -> Result<Rect, ErrorData> {
    if let Some(region) = params.region {
        return Ok(region);
    }
    let Some(element_id) = &params.element_id else {
        return Err(mcp_error(
            error_codes::OCR_NO_TEXT,
            "read_text requires region or element_id",
        ));
    };
    let input = current_input(state, 2)?;
    input
        .elements
        .iter()
        .find(|node| &node.element_id == element_id)
        .map(|node| node.bbox)
        .ok_or_else(|| {
            mcp_error(
                error_codes::OCR_NO_TEXT,
                "element_id has no visible OCR region",
            )
        })
}

fn ocr_result_from_text_regions(
    regions: Vec<TextRegion>,
    backend: OcrBackend,
    language: Option<String>,
) -> OcrResult {
    let text = regions
        .iter()
        .map(|word| word.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    OcrResult {
        text,
        words: regions
            .into_iter()
            .map(|word| OcrWord {
                text: word.text,
                bbox: word.bbox,
                confidence: word.confidence,
            })
            .collect(),
        language,
        backend,
    }
}

struct SyntheticOcrProvider {
    region: Rect,
}

impl synapse_perception::OcrProvider for SyntheticOcrProvider {
    fn read_text(&self, _region: Rect) -> synapse_perception::PerceptionResult<Vec<TextRegion>> {
        Ok(vec![TextRegion {
            text: "Synapse".to_owned(),
            bbox: Rect {
                x: self.region.x.saturating_add(4),
                y: self.region.y.saturating_add(4),
                w: 72,
                h: 18,
            },
            confidence: 0.99,
        }])
    }
}
