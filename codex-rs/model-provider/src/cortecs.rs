//! cortecs.ai model catalog support.
//!
//! cortecs.ai serves the standard OpenAI `GET /v1/models` list (`{"object":"list","data":[…]}`)
//! extended with capability metadata (`context_size`, `supported_features`,
//! `input_modalities`, `pricing`, …). This module maps that list onto Codex model metadata so
//! the model picker, context-window accounting, reasoning controls and image support reflect
//! what cortecs actually serves.

use codex_model_provider_info::CORTECS_DEFAULT_MODEL;
use codex_models_manager::model_info::generic_model_info;
use codex_protocol::openai_models::ApplyPatchToolType;
use codex_protocol::openai_models::ConfigShellToolType;
use codex_protocol::openai_models::InputModality;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ModelVisibility;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::openai_models::ReasoningEffortPreset;
use codex_protocol::openai_models::TruncationPolicyConfig;
use serde::Deserialize;

/// Fallback context window when the catalog does not report one.
const DEFAULT_CONTEXT_WINDOW: i64 = 128_000;

#[derive(Debug, Deserialize)]
struct OpenAiModelList {
    data: Vec<OpenAiModel>,
}

#[derive(Debug, Deserialize)]
struct OpenAiModel {
    id: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    context_size: Option<i64>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    #[serde(default)]
    input_modalities: Option<Vec<String>>,
    #[serde(default)]
    supported_features: Option<Vec<String>>,
    #[serde(default)]
    pricing: Option<Pricing>,
}

#[derive(Debug, Deserialize)]
struct Pricing {
    #[serde(default)]
    input_token: Option<f64>,
    #[serde(default)]
    output_token: Option<f64>,
    #[serde(default)]
    currency: Option<String>,
}

impl OpenAiModel {
    fn has_feature(&self, feature: &str) -> bool {
        self.supported_features
            .iter()
            .flatten()
            .any(|value| value.eq_ignore_ascii_case(feature))
            || self
                .tags
                .iter()
                .flatten()
                .any(|value| value.eq_ignore_ascii_case(feature))
    }

    fn accepts_images(&self) -> bool {
        match &self.input_modalities {
            Some(modalities) => modalities
                .iter()
                .any(|value| value.eq_ignore_ascii_case("image")),
            None => self.has_feature("vision"),
        }
    }
}

/// cortecs answers `/models?client_version=…` with a generic Codex-format catalog that omits
/// reasoning, vision and pricing details. Request the richer standard list instead.
pub(crate) fn standard_models_url(request_url: &str) -> String {
    let Ok(mut url) = url::Url::parse(request_url) else {
        return request_url.to_string();
    };
    let retained: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(name, _)| name != "client_version")
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect();
    if retained.is_empty() {
        url.set_query(None);
    } else {
        url.query_pairs_mut().clear().extend_pairs(retained);
    }
    url.into()
}

/// Parses a cortecs `/models` response into Codex model metadata.
///
/// Accepts the standard OpenAI list (preferred) and, as a fallback, a Codex-format catalog.
pub(crate) fn models_from_openai_list(body: &[u8]) -> Result<Vec<ModelInfo>, serde_json::Error> {
    let list: OpenAiModelList = match serde_json::from_slice(body) {
        Ok(list) => list,
        Err(error) => {
            return serde_json::from_slice::<codex_protocol::openai_models::ModelsResponse>(body)
                .map(|catalog| catalog.models)
                .map_err(|_| error);
        }
    };
    let mut models: Vec<ModelInfo> = list.data.iter().map(model_info).collect();
    // Default model first, then alphabetical; the picker orders by priority.
    models.sort_by(|a, b| {
        (a.slug != CORTECS_DEFAULT_MODEL, &a.slug).cmp(&(b.slug != CORTECS_DEFAULT_MODEL, &b.slug))
    });
    for (index, model) in models.iter_mut().enumerate() {
        model.priority = i32::try_from(index).unwrap_or(i32::MAX);
    }
    Ok(models)
}

fn model_info(model: &OpenAiModel) -> ModelInfo {
    let mut info = generic_model_info(&model.id);
    let supports_tools = model.has_feature("tools");
    let supports_reasoning = model.has_feature("reasoning");

    info.description = describe(model);
    // Agentic coding needs tool calling; keep other models selectable by name but out of the
    // picker.
    info.visibility = if supports_tools {
        ModelVisibility::List
    } else {
        ModelVisibility::Hide
    };
    info.shell_type = ConfigShellToolType::UnifiedExec;
    info.apply_patch_tool_type = supports_tools.then_some(ApplyPatchToolType::Freeform);
    info.truncation_policy = TruncationPolicyConfig::tokens(/*limit*/ 10_000);
    let context_window = model
        .context_size
        .filter(|size| *size > 0)
        .unwrap_or(DEFAULT_CONTEXT_WINDOW);
    info.context_window = Some(context_window);
    info.max_context_window = Some(context_window);
    info.input_modalities = if model.accepts_images() {
        vec![InputModality::Text, InputModality::Image]
    } else {
        vec![InputModality::Text]
    };
    if supports_reasoning {
        info.supported_reasoning_levels = reasoning_presets();
        info.default_reasoning_level = Some(ReasoningEffort::Medium);
        info.supports_reasoning_summary_parameter = true;
    } else {
        info.supported_reasoning_levels = Vec::new();
        info.default_reasoning_level = None;
        info.supports_reasoning_summary_parameter = false;
    }
    info
}

fn reasoning_presets() -> Vec<ReasoningEffortPreset> {
    // cortecs maps each requested level to the closest level the routed model supports.
    [
        (
            ReasoningEffort::Low,
            "Fast responses with lighter reasoning",
        ),
        (
            ReasoningEffort::Medium,
            "Balances speed and reasoning depth for everyday tasks",
        ),
        (
            ReasoningEffort::High,
            "Greater reasoning depth for complex problems",
        ),
    ]
    .into_iter()
    .map(|(effort, description)| ReasoningEffortPreset {
        effort,
        description: description.to_string(),
    })
    .collect()
}

fn describe(model: &OpenAiModel) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(pricing) = &model.pricing
        && let (Some(input), Some(output)) = (pricing.input_token, pricing.output_token)
    {
        let currency = match pricing.currency.as_deref() {
            None | Some("EUR") => "€".to_string(),
            Some("USD") => "$".to_string(),
            Some(other) => format!("{other} "),
        };
        parts.push(format!(
            "{currency}{input}/{currency}{output} per 1M in/out"
        ));
    }
    if let Some(context) = model.context_size.filter(|size| *size > 0) {
        parts.push(format!("{} ctx", format_tokens(context)));
    }
    if let Some(description) = model
        .description
        .as_deref()
        .map(str::trim)
        .filter(|description| !description.is_empty())
    {
        parts.push(description.to_string());
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

fn format_tokens(tokens: i64) -> String {
    if tokens >= 1_000_000 {
        let millions = tokens as f64 / 1_048_576.0;
        if (millions - millions.round()).abs() < 0.05 {
            format!("{}M", millions.round())
        } else {
            format!("{:.1}M", tokens as f64 / 1_000_000.0)
        }
    } else {
        format!("{}k", tokens / 1000)
    }
}

#[cfg(test)]
#[path = "cortecs_tests.rs"]
mod tests;
