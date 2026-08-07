use crate::error::AppError;
use crate::models::{
    AppDefinition, AppModel, AppPage, AppReference, DefinitionType,
};
use flowable_app_converter::{app_definition_to_json, parse_app_definition};
use flowable_app_model::{
    AppDefinition as CanonicalAppDefinition, AppPage as CanonicalAppPage,
    AppPageType as CanonicalAppPageType, AppReferenceType as CanonicalAppReferenceType,
    AppResourceReference as CanonicalAppResourceReference,
};

/// Convert a canonical app-model definition into the engine public model.
pub fn canonical_definition_to_engine(
    definition: CanonicalAppDefinition,
) -> AppDefinition {
    let definition_key = definition.key;
    let definition_name = definition
        .name
        .clone()
        .unwrap_or_else(|| definition_key.clone());
    let definition_id = definition
        .id
        .clone()
        .unwrap_or_else(|| definition_key.clone());

    let mut engine_definition =
        AppDefinition::new(definition_id, definition_key, definition_name);
    if let Some(description) = definition.description {
        engine_definition = engine_definition.with_description(description);
    }
    if let Some(category) = definition.category {
        engine_definition = engine_definition.with_category(category);
    }
    if let Some(theme) = definition.theme {
        engine_definition = engine_definition.with_theme(theme);
    }
    if let Some(icon) = definition.icon {
        engine_definition = engine_definition.with_icon(icon);
    }
    if let Some(users_access) = definition.users_access {
        engine_definition = engine_definition.with_users_access(users_access);
    }
    if let Some(groups_access) = definition.groups_access {
        engine_definition = engine_definition.with_groups_access(groups_access);
    }
    if let Some(landing_page) = definition.landing_page {
        engine_definition = engine_definition.with_landing_page(landing_page);
    }

    for (page_index, page) in definition.pages.into_iter().enumerate() {
        engine_definition = engine_definition.with_page(canonical_page_to_engine(page_index, page));
    }
    if !definition.references.is_empty() {
        let mut reference_page = AppPage::new("app-references", "Application references");
        for reference in definition.references {
            reference_page = reference_page.with_reference(canonical_reference_to_engine(reference));
        }
        engine_definition = engine_definition.with_page(reference_page);
    }

    engine_definition
}

/// Convert an engine app definition into the canonical app-model shape.
pub fn engine_definition_to_canonical(definition: &AppDefinition) -> CanonicalAppDefinition {
    let mut pages = Vec::new();
    let mut references = Vec::new();

    for page in &definition.pages {
        if page.id == "app-references" {
            for reference in &page.references {
                references.push(engine_reference_to_canonical(reference));
            }
            continue;
        }

        if page.references.len() == 1 {
            let reference = &page.references[0];
            // A pinned definitionId/tenantId cannot be expressed on a canonical
            // page; fall through to a top-level reference instead of silently
            // dropping those fields.
            if reference.definition_id.is_none() && reference.tenant_id.is_none() {
                pages.push(CanonicalAppPage {
                    id: page.id.clone(),
                    name: Some(page.name.clone()),
                    description: page.description.clone(),
                    page_type: definition_type_to_page_type(reference.definition_type),
                    definition_key: reference.definition_key.clone(),
                    icon: page.icon.clone(),
                    order: page.order,
                });
                continue;
            }
        }

        // Multi-reference pages flatten into top-level references, keeping page metadata
        // on the first reference entry for round-trip fidelity of keys.
        for reference in &page.references {
            references.push(CanonicalAppResourceReference {
                id: Some(reference.id.clone()),
                name: reference.name.clone().or_else(|| Some(page.name.clone())),
                description: reference.description.clone().or_else(|| page.description.clone()),
                reference_type: definition_type_to_reference_type(reference.definition_type),
                definition_key: reference.definition_key.clone(),
                definition_id: reference.definition_id.clone(),
                tenant_id: reference.tenant_id.clone(),
            });
        }
    }

    CanonicalAppDefinition {
        id: Some(definition.id.clone()),
        key: definition.key.clone(),
        name: Some(definition.name.clone()),
        description: definition.description.clone(),
        category: definition.category.clone(),
        theme: definition.theme.clone(),
        icon: definition.icon.clone(),
        users_access: definition.users_access.clone(),
        groups_access: definition.groups_access.clone(),
        landing_page: definition.landing_page.clone(),
        pages,
        references,
    }
}

pub fn engine_model_to_canonical(model: &AppModel) -> Result<CanonicalAppDefinition, AppError> {
    match model.app_definitions.as_slice() {
        [definition] => Ok(engine_definition_to_canonical(definition)),
        [] => Err(AppError::validation(
            "App model must contain at least one app definition",
        )),
        _ => Err(AppError::validation(
            "Canonical app-model bytes support a single app definition per resource",
        )),
    }
}

pub fn serialize_engine_model_as_durable_bytes(model: &AppModel) -> Result<Vec<u8>, AppError> {
    if model.app_definitions.len() == 1 {
        let canonical = engine_model_to_canonical(model)?;
        let json = app_definition_to_json(&canonical).map_err(|error| {
            AppError::validation(format!(
                "Failed to serialize canonical app definition: {error}"
            ))
        })?;
        return Ok(json.into_bytes());
    }

    // Multi-definition resources are an engine packaging convenience; persist the
    // engine JSON shape so round-trips remain exact.
    serde_json::to_vec(model).map_err(|error| {
        AppError::validation(format!("Failed to serialize engine app model: {error}"))
    })
}

/// Parse durable resource bytes into an engine `AppModel`.
///
/// Prefers the canonical app-model/app-converter shape; falls back to the engine
/// compatibility JSON shape used by older builders/tests.
pub fn parse_resource_bytes_to_engine_model(bytes: &[u8]) -> Result<AppModel, AppError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| AppError::validation(format!("App resource is not valid UTF-8: {error}")))?;

    if let Ok(canonical) = parse_app_definition(text) {
        return Ok(AppModel::new().with_app_definition(canonical_definition_to_engine(canonical)));
    }

    if let Ok(engine_model) = serde_json::from_str::<AppModel>(text) {
        if !engine_model.app_definitions.is_empty() {
            return Ok(engine_model);
        }
    }

    // Single engine AppDefinition object.
    if let Ok(engine_definition) = serde_json::from_str::<AppDefinition>(text) {
        return Ok(AppModel::new().with_app_definition(engine_definition));
    }

    Err(AppError::validation(
        "App resource bytes are not a valid app definition (canonical or engine shape)",
    ))
}

/// Compare two engine models for deployment consistency.
///
/// Each definition pair is normalized through the canonical app-model AST and
/// compared structurally, so page metadata (id/name/description/icon/order),
/// reference ownership, order, and the canonical reference fields
/// (`definitionId`/`tenantId`) all participate — not just definition-level
/// fields plus a sorted `(type, key)` set.
pub fn models_semantically_equal(left: &AppModel, right: &AppModel) -> bool {
    if left.app_definitions.len() != right.app_definitions.len() {
        return false;
    }
    left.app_definitions
        .iter()
        .zip(right.app_definitions.iter())
        .all(|(a, b)| {
            let mut canonical_left = engine_definition_to_canonical(a);
            let mut canonical_right = engine_definition_to_canonical(b);
            // The engine definition id is deployment-generated and defaulted
            // from the key when canonical bytes omit it; it is identity, not
            // model semantics.
            canonical_left.id = None;
            canonical_right.id = None;
            canonical_left == canonical_right
        })
}

fn canonical_page_to_engine(page_index: usize, page: CanonicalAppPage) -> AppPage {
    let page_name = page.name.unwrap_or_else(|| page.id.clone());
    let reference_id = format!("{}-ref-{}", page.id, page_index + 1);
    let reference_name = page_name.clone();
    let mut engine_page = AppPage::new(page.id, page_name);
    if let Some(description) = page.description {
        engine_page = engine_page.with_description(description);
    }
    if let Some(icon) = page.icon {
        engine_page = engine_page.with_icon(icon);
    }
    if let Some(order) = page.order {
        engine_page = engine_page.with_order(order);
    }
    engine_page.with_reference(
        AppReference::new(reference_id, page_type_to_definition_type(page.page_type))
            .with_name(reference_name)
            .with_definition_key(page.definition_key),
    )
}

fn canonical_reference_to_engine(reference: CanonicalAppResourceReference) -> AppReference {
    let reference_id = reference
        .id
        .unwrap_or_else(|| format!("{}-ref", reference.definition_key));
    let mut engine_reference = AppReference::new(
        reference_id,
        reference_type_to_definition_type(reference.reference_type),
    )
    .with_definition_key(reference.definition_key);
    if let Some(name) = reference.name {
        engine_reference = engine_reference.with_name(name);
    }
    if let Some(description) = reference.description {
        engine_reference = engine_reference.with_description(description);
    }
    if let Some(definition_id) = reference.definition_id {
        engine_reference = engine_reference.with_definition_id(definition_id);
    }
    if let Some(tenant_id) = reference.tenant_id {
        engine_reference = engine_reference.with_tenant_id(tenant_id);
    }
    engine_reference
}

fn engine_reference_to_canonical(reference: &AppReference) -> CanonicalAppResourceReference {
    CanonicalAppResourceReference {
        id: Some(reference.id.clone()),
        name: reference.name.clone(),
        description: reference.description.clone(),
        reference_type: definition_type_to_reference_type(reference.definition_type),
        definition_key: reference.definition_key.clone(),
        definition_id: reference.definition_id.clone(),
        tenant_id: reference.tenant_id.clone(),
    }
}

fn page_type_to_definition_type(page_type: CanonicalAppPageType) -> DefinitionType {
    match page_type {
        CanonicalAppPageType::Process => DefinitionType::BpmnProcess,
        CanonicalAppPageType::Decision => DefinitionType::DmnDecision,
        CanonicalAppPageType::Case => DefinitionType::CmmnCase,
        CanonicalAppPageType::Event => DefinitionType::EventRegistry,
    }
}

fn reference_type_to_definition_type(reference_type: CanonicalAppReferenceType) -> DefinitionType {
    match reference_type {
        CanonicalAppReferenceType::Bpmn => DefinitionType::BpmnProcess,
        CanonicalAppReferenceType::Dmn => DefinitionType::DmnDecision,
        CanonicalAppReferenceType::Cmmn => DefinitionType::CmmnCase,
        CanonicalAppReferenceType::EventRegistry => DefinitionType::EventRegistry,
    }
}

fn definition_type_to_page_type(definition_type: DefinitionType) -> CanonicalAppPageType {
    match definition_type {
        DefinitionType::BpmnProcess => CanonicalAppPageType::Process,
        DefinitionType::DmnDecision => CanonicalAppPageType::Decision,
        DefinitionType::CmmnCase => CanonicalAppPageType::Case,
        DefinitionType::EventRegistry => CanonicalAppPageType::Event,
    }
}

fn definition_type_to_reference_type(definition_type: DefinitionType) -> CanonicalAppReferenceType {
    match definition_type {
        DefinitionType::BpmnProcess => CanonicalAppReferenceType::Bpmn,
        DefinitionType::DmnDecision => CanonicalAppReferenceType::Dmn,
        DefinitionType::CmmnCase => CanonicalAppReferenceType::Cmmn,
        DefinitionType::EventRegistry => CanonicalAppReferenceType::EventRegistry,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::AppReference;

    #[test]
    fn round_trips_single_reference_pages_through_canonical_bytes() {
        let engine = AppDefinition::new("app-1", "portal", "Portal").with_page(
            AppPage::new("page-process", "Process").with_reference(
                AppReference::process("start")
                    .with_name("Start")
                    .with_definition_key("onboarding"),
            ),
        );
        let model = AppModel::new().with_app_definition(engine.clone());
        let bytes = serialize_engine_model_as_durable_bytes(&model).unwrap();
        let parsed = parse_resource_bytes_to_engine_model(&bytes).unwrap();
        assert!(models_semantically_equal(&model, &parsed));
        assert_eq!(parsed.app_definitions[0].key, "portal");
        assert_eq!(
            parsed.app_definitions[0].pages[0].references[0].definition_key,
            "onboarding"
        );
    }

    fn base_definition() -> AppDefinition {
        AppDefinition::new("app-1", "portal", "Portal")
            .with_page(
                AppPage::new("page-process", "Process")
                    .with_description("Process page")
                    .with_icon("icon.png")
                    .with_order(3)
                    .with_reference(
                        AppReference::process("start")
                            .with_name("Start")
                            .with_definition_key("onboarding"),
                    ),
            )
            .with_page(
                AppPage::new("page-mixed", "Mixed")
                    .with_reference(
                        AppReference::decision("benefits")
                            .with_name("Benefits")
                            .with_definition_key("benefits-check"),
                    )
                    .with_reference(
                        AppReference::case("equipment")
                            .with_definition_key("equipment-case")
                            .with_definition_id("equipment-case:3:77")
                            .with_tenant_id("tenant-a"),
                    ),
            )
    }

    fn model_of(definition: AppDefinition) -> AppModel {
        AppModel::new().with_app_definition(definition)
    }

    #[test]
    fn round_trips_reference_definition_id_and_tenant_id() {
        // P2-7: canonical definitionId/tenantId were previously written out as
        // None, silently unpinning references on every round-trip.
        let model = model_of(base_definition());
        let bytes = serialize_engine_model_as_durable_bytes(&model).unwrap();
        let parsed = parse_resource_bytes_to_engine_model(&bytes).unwrap();
        assert!(models_semantically_equal(&model, &parsed));

        let references: Vec<&AppReference> = parsed.app_definitions[0]
            .pages
            .iter()
            .flat_map(|page| page.references.iter())
            .collect();
        let pinned = references
            .iter()
            .find(|r| r.id == "equipment")
            .expect("pinned reference survives the round-trip");
        assert_eq!(pinned.definition_id.as_deref(), Some("equipment-case:3:77"));
        assert_eq!(pinned.tenant_id.as_deref(), Some("tenant-a"));
    }

    #[test]
    fn single_reference_page_with_pinned_definition_keeps_the_pin() {
        // A canonical page cannot express definitionId/tenantId; the converter
        // must emit a top-level reference rather than drop the pin.
        let model = model_of(AppDefinition::new("app-1", "portal", "Portal").with_page(
            AppPage::new("page-only", "Only").with_reference(
                AppReference::process("start")
                    .with_definition_key("onboarding")
                    .with_definition_id("onboarding:5:99"),
            ),
        ));
        let canonical = engine_model_to_canonical(&model).unwrap();
        assert!(canonical.pages.is_empty(), "the pin cannot live on a page");
        assert_eq!(canonical.references.len(), 1);
        assert_eq!(
            canonical.references[0].definition_id.as_deref(),
            Some("onboarding:5:99")
        );

        let bytes = serialize_engine_model_as_durable_bytes(&model).unwrap();
        let parsed = parse_resource_bytes_to_engine_model(&bytes).unwrap();
        assert!(models_semantically_equal(&model, &parsed));
    }

    #[test]
    fn page_metadata_differences_are_detected() {
        // Previously only definition-level fields and sorted (type, key) pairs
        // were compared; page id/name/description/icon/order changes passed.
        let base = model_of(base_definition());

        let mut renamed_page = base_definition();
        renamed_page.pages[0].name = "Renamed".to_string();
        assert!(!models_semantically_equal(&base, &model_of(renamed_page)));

        let mut different_id = base_definition();
        different_id.pages[0].id = "page-other".to_string();
        assert!(!models_semantically_equal(&base, &model_of(different_id)));

        let mut different_description = base_definition();
        different_description.pages[0].description = Some("Other".to_string());
        assert!(!models_semantically_equal(&base, &model_of(different_description)));

        let mut different_icon = base_definition();
        different_icon.pages[0].icon = Some("other.png".to_string());
        assert!(!models_semantically_equal(&base, &model_of(different_icon)));

        let mut different_order = base_definition();
        different_order.pages[0].order = Some(9);
        assert!(!models_semantically_equal(&base, &model_of(different_order)));
    }

    #[test]
    fn reference_metadata_and_pin_differences_are_detected() {
        let base = model_of(base_definition());

        let mut different_name = base_definition();
        different_name.pages[1].references[0].name = Some("Other".to_string());
        assert!(!models_semantically_equal(&base, &model_of(different_name)));

        let mut different_pin = base_definition();
        different_pin.pages[1].references[1].definition_id = Some("equipment-case:4:88".to_string());
        assert!(!models_semantically_equal(&base, &model_of(different_pin)));

        let mut different_tenant = base_definition();
        different_tenant.pages[1].references[1].tenant_id = Some("tenant-b".to_string());
        assert!(!models_semantically_equal(&base, &model_of(different_tenant)));

        let mut moved_reference = base_definition();
        let moved = moved_reference.pages[1].references.remove(1);
        moved_reference.pages[0].references.push(moved);
        assert!(
            !models_semantically_equal(&base, &model_of(moved_reference)),
            "the same (type, key) set on different pages is not the same model"
        );

        let mut swapped = base_definition();
        swapped.pages[1].references.swap(0, 1);
        assert!(
            !models_semantically_equal(&base, &model_of(swapped)),
            "reference order within a page is part of the model"
        );
    }

    #[test]
    fn regenerated_reference_ids_on_single_reference_pages_still_compare_equal() {
        // Canonical pages do not persist a reference id; the parse side
        // regenerates one. Normalizing both sides through the canonical AST
        // keeps that lossy detail out of the comparison.
        let model = model_of(AppDefinition::new("app-1", "portal", "Portal").with_page(
            AppPage::new("page-process", "Process").with_reference(
                AppReference::process("original-ref-id")
                    .with_name("Start")
                    .with_definition_key("onboarding"),
            ),
        ));
        let bytes = serialize_engine_model_as_durable_bytes(&model).unwrap();
        let parsed = parse_resource_bytes_to_engine_model(&bytes).unwrap();
        assert_ne!(
            parsed.app_definitions[0].pages[0].references[0].id,
            "original-ref-id"
        );
        assert!(models_semantically_equal(&model, &parsed));
    }
}
