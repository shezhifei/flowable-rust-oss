//! P96 (WP9a) — DMN target-namespace gate on deployment parse.
//!
//! Java truth: the converter picks the XSD by the document namespace
//! (`DmnXMLConverter.java:83-87,135-162`: DMN 1.1 / 1.2 / 1.3 target
//! namespaces) and schema validation is on by default (`DmnParse.java:54`),
//! so an unknown namespace fails deployment. Java's per-deployment opt-out is
//! `DeploymentSettings.IS_DMN_XSD_VALIDATION_ENABLED`
//! (`ParsedDeploymentBuilder.java:81-82`), mirrored here by
//! `parse_definition_with_validation(xml, false)` (converter API only, not
//! exposed over REST). Rust deliberately keeps accepting documents without an
//! xmlns (legacy fixtures); only a present-but-foreign namespace is rejected.

use flowable_dmn_converter::{DmnXmlConverter, parse_dmn_definition};

fn definitions_with_xmlns(xmlns: &str) -> String {
    format!(
        r#"<definitions xmlns="{xmlns}" id="ns-test" name="Namespace Test">
  <decision id="d1" name="D1">
    <decisionTable id="t1" hitPolicy="FIRST">
      <input id="i1"><inputExpression id="ie1" typeRef="string"><text>x</text></inputExpression></input>
      <output id="o1" name="y" typeRef="string" />
      <rule id="r1">
        <inputEntry id="re1"><text>-</text></inputEntry>
        <outputEntry id="ro1"><text>'a'</text></outputEntry>
      </rule>
    </decisionTable>
  </decision>
</definitions>"#
    )
}

#[test]
fn accepts_dmn_11_12_13_namespaces() {
    for xmlns in [
        "http://www.omg.org/spec/DMN/20151101/dmn.xsd",
        "http://www.omg.org/spec/DMN/20180521/MODEL/",
        "https://www.omg.org/spec/DMN/20191111/MODEL/",
    ] {
        parse_dmn_definition(&definitions_with_xmlns(xmlns))
            .unwrap_or_else(|error| panic!("DMN namespace {xmlns} must parse: {error}"));
    }
}

#[test]
fn rejects_foreign_namespace_like_bpmn_or_cmmn() {
    for xmlns in [
        "http://www.omg.org/spec/BPMN/20100524/MODEL",
        "http://www.omg.org/spec/CMMN/20151109/MODEL",
        "https://example.com/not-dmn",
    ] {
        let error = parse_dmn_definition(&definitions_with_xmlns(xmlns))
            .expect_err("foreign namespace must be rejected");
        assert!(
            format!("{error}").contains("unsupported DMN namespace"),
            "error should name the namespace gate, got: {error}"
        );
    }
}

#[test]
fn accepts_document_without_xmlns() {
    let xml = r#"<definitions id="ns-test" name="Namespace Test">
  <decision id="d1" name="D1">
    <decisionTable id="t1" hitPolicy="FIRST">
      <input id="i1"><inputExpression id="ie1" typeRef="string"><text>x</text></inputExpression></input>
      <output id="o1" name="y" typeRef="string" />
      <rule id="r1">
        <inputEntry id="re1"><text>-</text></inputEntry>
        <outputEntry id="ro1"><text>'a'</text></outputEntry>
      </rule>
    </decisionTable>
  </decision>
</definitions>"#;
    parse_dmn_definition(xml).expect("missing xmlns stays accepted (legacy fixtures)");
}

#[test]
fn validation_opt_out_accepts_foreign_namespace() {
    let converter = DmnXmlConverter::new();
    converter
        .parse_definition_with_validation(
            &definitions_with_xmlns("http://www.omg.org/spec/BPMN/20100524/MODEL"),
            false,
        )
        .expect("opt-out disables the namespace gate (IS_DMN_XSD_VALIDATION_ENABLED)");
}
