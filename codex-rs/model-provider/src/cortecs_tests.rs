use super::*;
use pretty_assertions::assert_eq;

const LIST: &str = r#"{"object":"list","data":[
 {"id":"zeta-chat","object":"model","created":1,"owned_by":"x","pricing":{"currency":"EUR","input_token":0.1,"output_token":0.2},"providers":["a"],"context_size":32000,"tags":["Instruct"],"input_modalities":["text"],"supported_features":["json_mode"]},
 {"id":"glm-5.3-flash","object":"model","created":1,"owned_by":"Z","description":"Fast GLM.","pricing":{"currency":"EUR","input_token":0.09,"output_token":0.314},"providers":["a"],"context_size":1048576,"tags":["Instruct","Tools","Reasoning","Vision"],"input_modalities":["text","image"],"supported_features":["json_mode","reasoning","tools"]},
 {"id":"alpha-coder","object":"model","created":1,"owned_by":"y","pricing":{"currency":"EUR","input_token":1,"output_token":2},"providers":["a"],"context_size":256000,"input_modalities":["text"],"supported_features":["tools"]}
]}"#;

#[test]
fn maps_cortecs_model_list() {
    let models = models_from_openai_list(LIST.as_bytes()).expect("parse");
    let slugs: Vec<&str> = models.iter().map(|model| model.slug.as_str()).collect();
    assert_eq!(slugs, vec!["glm-5.3-flash", "alpha-coder", "zeta-chat"]);
    assert_eq!(
        models
            .iter()
            .map(|model| model.priority)
            .collect::<Vec<_>>(),
        vec![0, 1, 2]
    );

    let glm = &models[0];
    assert_eq!(glm.visibility, ModelVisibility::List);
    assert_eq!(glm.context_window, Some(1_048_576));
    assert_eq!(
        glm.input_modalities,
        vec![InputModality::Text, InputModality::Image]
    );
    assert_eq!(glm.default_reasoning_level, Some(ReasoningEffort::Medium));
    assert_eq!(glm.supported_reasoning_levels.len(), 3);
    assert_eq!(
        glm.apply_patch_tool_type,
        Some(ApplyPatchToolType::Freeform)
    );
    assert_eq!(
        glm.description.as_deref(),
        Some("€0.09/€0.314 per 1M in/out · 1M ctx · reasoning · vision")
    );

    let coder = &models[1];
    assert_eq!(coder.input_modalities, vec![InputModality::Text]);
    assert!(coder.supported_reasoning_levels.is_empty());
    assert!(!coder.supports_reasoning_summary_parameter);

    let chat = &models[2];
    assert_eq!(chat.visibility, ModelVisibility::Hide);
    assert_eq!(chat.apply_patch_tool_type, None);
}

#[test]
fn accepts_codex_catalog_shape_as_fallback() {
    assert_eq!(
        models_from_openai_list(br#"{"models":[]}"#).expect("parse"),
        Vec::new()
    );
    assert!(models_from_openai_list(br#"{"unexpected":[]}"#).is_err());
}

#[test]
fn strips_client_version_query() {
    assert_eq!(
        standard_models_url("https://api.cortecs.ai/v1/models?client_version=1.2.3"),
        "https://api.cortecs.ai/v1/models"
    );
    assert_eq!(
        standard_models_url("https://api.cortecs.ai/v1/models?eu_native=true&client_version=1"),
        "https://api.cortecs.ai/v1/models?eu_native=true"
    );
}
