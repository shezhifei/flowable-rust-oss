use flowable_cmmn_converter::{parse_cmmn_case_file_models, parse_cmmn_definitions};

#[test]
fn parses_nested_case_file_model_network() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL" targetNamespace="Examples">
  <case id="documentCase" name="Document case">
    <caseFileModel id="caseFile">
      <caseFileItemDefinition id="folderDef" name="Folder" definitionType="http://www.omg.org/spec/CMMN/DefinitionType/CMISFolder" />
      <caseFileItemDefinition id="documentDef" name="Document" definitionType="http://www.omg.org/spec/CMMN/DefinitionType/CMISDocument" />
      <caseFileItem id="rootFolder" name="Root" definitionRef="folderDef">
        <caseFileItem id="documentTemplate" name="Document" definitionRef="documentDef" />
      </caseFileItem>
    </caseFileModel>
    <casePlanModel id="plan" autoComplete="true" />
  </case>
</definitions>"#;

    parse_cmmn_definitions(xml).expect("full CMMN definitions should validate");
    let networks = parse_cmmn_case_file_models(xml).expect("nested case-file model should parse");
    let model = &networks[0].1;
    assert_eq!(model.item_definitions.len(), 2);
    assert_eq!(model.items[0].children[0].definition_ref, "documentDef");
}

#[test]
fn rejects_unknown_nested_case_file_definition_reference() {
    let xml = r#"<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL">
      <case id="c"><caseFileModel><caseFileItem id="x" definitionRef="missing" /></caseFileModel><casePlanModel id="p" /></case>
    </definitions>"#;
    let error = parse_cmmn_definitions(xml).unwrap_err();
    assert!(error.to_string().contains("unknown definition"));
}
