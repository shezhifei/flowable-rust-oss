use flowable_app_converter::{
    app_definition_to_json, app_page_to_json, app_resource_reference_to_json,
    page_type_to_reference_type, parse_app_definition, parse_app_page,
    parse_app_resource_reference,
};
use flowable_app_model::{AppPageType, AppReferenceType};
use serde_json::{Value, json};

fn parse_value(json_text: &str) -> Value {
    serde_json::from_str(json_text).expect("test json should be valid")
}

#[test]
fn parses_and_serializes_supported_app_definition_shape() {
    let app_json = json!({
        "id": "app-1",
        "key": "customerOperations",
        "name": "Customer Operations",
        "description": "Owned M17 app definition subset",
        "category": "operations",
        "pages": [
            {
                "id": "page-process",
                "name": "Order process",
                "pageType": "process",
                "definitionKey": "orderProcess"
            },
            {
                "id": "page-decision",
                "name": "Risk decision",
                "pageType": "decision",
                "definitionKey": "riskDecision"
            },
            {
                "id": "page-case",
                "name": "Support case",
                "pageType": "case",
                "definitionKey": "supportCase"
            },
            {
                "id": "page-event",
                "name": "Order received event",
                "pageType": "event",
                "definitionKey": "orderReceived"
            }
        ],
        "references": [
            {
                "id": "ref-bpmn",
                "name": "Order process",
                "referenceType": "bpmn",
                "definitionKey": "orderProcess"
            },
            {
                "id": "ref-dmn",
                "name": "Risk decision",
                "referenceType": "dmn",
                "definitionKey": "riskDecision"
            },
            {
                "id": "ref-cmmn",
                "name": "Support case",
                "referenceType": "cmmn",
                "definitionKey": "supportCase"
            },
            {
                "id": "ref-event",
                "name": "Order received event",
                "referenceType": "eventRegistry",
                "definitionKey": "orderReceived"
            }
        ]
    })
    .to_string();

    let definition = parse_app_definition(&app_json).expect("app definition should parse");
    assert_eq!(definition.id.as_deref(), Some("app-1"));
    assert_eq!(definition.key, "customerOperations");
    assert_eq!(definition.pages.len(), 4);
    assert_eq!(definition.references.len(), 4);
    assert_eq!(definition.pages[0].page_type, AppPageType::Process);
    assert_eq!(definition.pages[1].page_type, AppPageType::Decision);
    assert_eq!(definition.pages[2].page_type, AppPageType::Case);
    assert_eq!(definition.pages[3].page_type, AppPageType::Event);
    assert_eq!(
        definition.references[0].reference_type,
        AppReferenceType::Bpmn
    );
    assert_eq!(
        page_type_to_reference_type(&definition.pages[3]),
        AppReferenceType::EventRegistry
    );

    let serialized = app_definition_to_json(&definition).expect("app definition should serialize");
    assert_eq!(parse_value(&serialized), parse_value(&app_json));
}

#[test]
fn parses_page_and_reference_resources_individually() {
    let page_json = json!({
        "id": "page-process",
        "name": "Order process",
        "pageType": "process",
        "definitionKey": "orderProcess"
    })
    .to_string();
    let reference_json = json!({
        "id": "ref-bpmn",
        "name": "Order process",
        "referenceType": "bpmn",
        "definitionKey": "orderProcess"
    })
    .to_string();

    let page = parse_app_page(&page_json).expect("page should parse");
    let reference = parse_app_resource_reference(&reference_json).expect("reference should parse");

    assert_eq!(page.page_type, AppPageType::Process);
    assert_eq!(reference.reference_type, AppReferenceType::Bpmn);

    let serialized_page = app_page_to_json(&page).expect("page should serialize");
    let serialized_reference =
        app_resource_reference_to_json(&reference).expect("reference should serialize");

    assert_eq!(parse_value(&serialized_page), parse_value(&page_json));
    assert_eq!(
        parse_value(&serialized_reference),
        parse_value(&reference_json)
    );
}

#[test]
fn parses_app_metadata_and_common_page_reference_fields() {
    let app_json = json!({
        "key": "customerOperations",
        "name": "Customer Operations",
        "description": "Customer workspace",
        "theme": "flowable",
        "icon": "customer",
        "usersAccess": "admin,ops",
        "groupsAccess": "operations",
        "landingPage": "page-process",
        "pages": [
            {
                "id": "page-process",
                "name": "Order process",
                "description": "Start and track orders",
                "pageType": "process",
                "definitionKey": "orderProcess",
                "icon": "play",
                "order": 10
            }
        ],
        "references": [
            {
                "id": "ref-bpmn",
                "name": "Order process",
                "description": "Order process reference",
                "referenceType": "bpmn",
                "definitionKey": "orderProcess",
                "definitionId": "process-definition:orderProcess:1",
                "tenantId": "tenant-a"
            }
        ]
    })
    .to_string();

    let definition = parse_app_definition(&app_json).expect("app metadata should parse");
    assert_eq!(definition.theme.as_deref(), Some("flowable"));
    assert_eq!(definition.icon.as_deref(), Some("customer"));
    assert_eq!(definition.users_access.as_deref(), Some("admin,ops"));
    assert_eq!(definition.groups_access.as_deref(), Some("operations"));
    assert_eq!(definition.landing_page.as_deref(), Some("page-process"));
    assert_eq!(
        definition.pages[0].description.as_deref(),
        Some("Start and track orders")
    );
    assert_eq!(definition.pages[0].icon.as_deref(), Some("play"));
    assert_eq!(definition.pages[0].order, Some(10));
    assert_eq!(
        definition.references[0].description.as_deref(),
        Some("Order process reference")
    );
    assert_eq!(
        definition.references[0].definition_id.as_deref(),
        Some("process-definition:orderProcess:1")
    );
    assert_eq!(
        definition.references[0].tenant_id.as_deref(),
        Some("tenant-a")
    );

    let serialized = app_definition_to_json(&definition).expect("metadata should serialize");
    assert_eq!(parse_value(&serialized), parse_value(&app_json));
}

#[test]
fn rejects_unsupported_page_type_and_reference_type() {
    let unsupported_page_json = json!({
        "id": "page-custom",
        "pageType": "custom",
        "definitionKey": "customPage"
    })
    .to_string();
    let unsupported_reference_json = json!({
        "referenceType": "form",
        "definitionKey": "customerForm"
    })
    .to_string();

    let page_error =
        parse_app_page(&unsupported_page_json).expect_err("unsupported page type must fail");
    let reference_error = parse_app_resource_reference(&unsupported_reference_json)
        .expect_err("unsupported reference type must fail");

    assert!(
        page_error.to_string().contains("pageType") && page_error.to_string().contains("custom"),
        "unexpected error message: {page_error}"
    );
    assert!(
        reference_error.to_string().contains("referenceType")
            && reference_error.to_string().contains("form"),
        "unexpected error message: {reference_error}"
    );
}

#[test]
fn rejects_invalid_pages_and_references_shapes() {
    let invalid_shape_json = json!({
        "key": "customerOperations",
        "pages": {},
        "references": []
    })
    .to_string();
    let blank_definition_key_json = json!({
        "referenceType": "eventRegistry",
        "definitionKey": "   "
    })
    .to_string();

    let app_error =
        parse_app_definition(&invalid_shape_json).expect_err("invalid pages shape must fail");
    let reference_error = parse_app_resource_reference(&blank_definition_key_json)
        .expect_err("blank definition key must fail");

    assert!(
        app_error.to_string().contains("pages"),
        "unexpected error message: {app_error}"
    );
    assert!(
        reference_error.to_string().contains("definitionKey"),
        "unexpected error message: {reference_error}"
    );
}
