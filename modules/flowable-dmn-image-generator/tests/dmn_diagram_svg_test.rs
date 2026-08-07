use flowable_dmn_converter::parse_dmn_definition;
use flowable_dmn_engine::{DmnDeploymentRequest, DmnEngine, DmnModel};
use flowable_dmn_image_generator::{
    DmnAdvancedSvgGeneratorOptions, DmnSvgGenerator, DmnSvgGeneratorError, DmnSvgGeneratorOptions,
};

const FIRST_HIT_DMN: &str = r#"
<definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/"
             id="loan-defs"
             name="Loan Decisions"
             namespace="http://flowable.org/dmn">
  <decision id="loanEligibility" name="Loan Eligibility">
    <decisionTable id="loanDecisionTable" hitPolicy="FIRST">
      <input id="input1" label="Credit score">
        <inputExpression id="inputExpression1" typeRef="number">
          <text>creditScore</text>
        </inputExpression>
      </input>
      <output id="output1" label="Approved" name="approved" typeRef="boolean" />
      <output id="output2" label="Risk band" name="riskBand" typeRef="string" />
      <rule id="rule1">
        <inputEntry id="inputEntry1"><text>730</text></inputEntry>
        <outputEntry id="outputEntry1"><text>true</text></outputEntry>
        <outputEntry id="outputEntry2"><text>'LOW'</text></outputEntry>
      </rule>
      <rule id="rule2">
        <inputEntry id="inputEntry2"><text>-</text></inputEntry>
        <outputEntry id="outputEntry3"><text>false</text></outputEntry>
        <outputEntry id="outputEntry4"><text>'HIGH'</text></outputEntry>
      </rule>
    </decisionTable>
  </decision>
</definitions>
"#;

#[test]
fn renders_owned_first_hit_policy_svg_deterministically() {
    let definition = parse_dmn_definition(FIRST_HIT_DMN).expect("FIRST fixture should parse");
    let generator = DmnSvgGenerator::new();

    let svg = generator
        .generate_definition_svg(&definition)
        .expect("owned DMN subset should render");

    let expected = concat!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"408\" height=\"186\" viewBox=\"0 0 408 186\" role=\"img\" aria-labelledby=\"dmn-title-loanEligibility\">\n",
        "<title id=\"dmn-title-loanEligibility\">Loan Eligibility</title>\n",
        "<rect x=\"1\" y=\"1\" width=\"406\" height=\"184\" rx=\"10\" fill=\"#fcfcfd\" stroke=\"#1f2937\" stroke-width=\"2\"/>\n",
        "<rect x=\"24\" y=\"24\" width=\"360\" height=\"44\" rx=\"8\" fill=\"#1f2937\"/>\n",
        "<text x=\"40\" y=\"51\" font-family=\"monospace\" font-size=\"18\" font-weight=\"700\" fill=\"#f9fafb\">Loan Eligibility</text>\n",
        "<rect x=\"307\" y=\"34\" width=\"61\" height=\"24\" rx=\"12\" fill=\"#f59e0b\"/>\n",
        "<text x=\"337\" y=\"50\" text-anchor=\"middle\" font-family=\"monospace\" font-size=\"12\" font-weight=\"700\" fill=\"#111827\">FIRST</text>\n",
        "<rect x=\"24\" y=\"80\" width=\"120\" height=\"34\" fill=\"#e2e8f0\" stroke=\"#475569\" stroke-width=\"1.5\"/>\n",
        "<text x=\"36\" y=\"101\" font-family=\"monospace\" font-size=\"13\" font-weight=\"700\" fill=\"#0f172a\">Credit score</text>\n",
        "<rect x=\"144\" y=\"80\" width=\"120\" height=\"34\" fill=\"#e2e8f0\" stroke=\"#475569\" stroke-width=\"1.5\"/>\n",
        "<text x=\"156\" y=\"101\" font-family=\"monospace\" font-size=\"13\" font-weight=\"700\" fill=\"#0f172a\">Approved</text>\n",
        "<rect x=\"264\" y=\"80\" width=\"120\" height=\"34\" fill=\"#e2e8f0\" stroke=\"#475569\" stroke-width=\"1.5\"/>\n",
        "<text x=\"276\" y=\"101\" font-family=\"monospace\" font-size=\"13\" font-weight=\"700\" fill=\"#0f172a\">Risk band</text>\n",
        "<rect x=\"24\" y=\"114\" width=\"120\" height=\"24\" fill=\"#ffffff\" stroke=\"#94a3b8\" stroke-width=\"1\"/>\n",
        "<text x=\"36\" y=\"130\" font-family=\"monospace\" font-size=\"12\" fill=\"#334155\">730</text>\n",
        "<rect x=\"144\" y=\"114\" width=\"120\" height=\"24\" fill=\"#ffffff\" stroke=\"#94a3b8\" stroke-width=\"1\"/>\n",
        "<text x=\"156\" y=\"130\" font-family=\"monospace\" font-size=\"12\" fill=\"#334155\">true</text>\n",
        "<rect x=\"264\" y=\"114\" width=\"120\" height=\"24\" fill=\"#ffffff\" stroke=\"#94a3b8\" stroke-width=\"1\"/>\n",
        "<text x=\"276\" y=\"130\" font-family=\"monospace\" font-size=\"12\" fill=\"#334155\">'LOW'</text>\n",
        "<rect x=\"24\" y=\"138\" width=\"120\" height=\"24\" fill=\"#ffffff\" stroke=\"#94a3b8\" stroke-width=\"1\"/>\n",
        "<text x=\"36\" y=\"154\" font-family=\"monospace\" font-size=\"12\" fill=\"#334155\">-</text>\n",
        "<rect x=\"144\" y=\"138\" width=\"120\" height=\"24\" fill=\"#ffffff\" stroke=\"#94a3b8\" stroke-width=\"1\"/>\n",
        "<text x=\"156\" y=\"154\" font-family=\"monospace\" font-size=\"12\" fill=\"#334155\">false</text>\n",
        "<rect x=\"264\" y=\"138\" width=\"120\" height=\"24\" fill=\"#ffffff\" stroke=\"#94a3b8\" stroke-width=\"1\"/>\n",
        "<text x=\"276\" y=\"154\" font-family=\"monospace\" font-size=\"12\" fill=\"#334155\">'HIGH'</text>\n",
        "</svg>\n"
    );

    assert_eq!(svg, expected);
}

#[test]
fn rejects_advanced_dmn_svg_options_structurally() {
    let definition = parse_dmn_definition(FIRST_HIT_DMN).expect("FIRST fixture should parse");
    let generator = DmnSvgGenerator::new();
    let options = DmnSvgGeneratorOptions {
        advanced: DmnAdvancedSvgGeneratorOptions {
            font_family: Some("Fira Code".to_string()),
            color_scheme: None,
            scale: Some(2.0),
        },
    };

    let error = generator
        .generate_definition_svg_with_options(&definition, &options)
        .expect_err("advanced options must fail structurally");

    assert_eq!(
        error,
        DmnSvgGeneratorError::UnsupportedOptions {
            options: vec!["font_family", "scale"],
        }
    );
}

#[test]
fn renders_engine_svg_label_for_ends_with_string_function_unary_test() {
    let mut definition = parse_dmn_definition(FIRST_HIT_DMN).expect("FIRST fixture should parse");
    definition.decisions[0].decision_table.inputs[0]
        .input_expression
        .type_ref = Some("string".to_string());
    definition.decisions[0].decision_table.rules[0].input_entries[0].text =
        Some("ends with(?, \"suffix\")".to_string());
    let model = DmnModel::try_from(definition).expect("ends with unary test should parse");
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("ends-with-svg-label")
                .with_resource("loan-decision.dmn", model),
        )
        .expect("deployment");
    let definitions = engine
        .repository_service()
        .create_decision_query()
        .key("loanEligibility")
        .list()
        .expect("definitions");
    let generator = DmnSvgGenerator::new();

    let svg = generator
        .generate_engine_definition_svg(&definitions[0])
        .expect("engine DMN subset should render");

    assert!(
        svg.contains("ends with(?, &quot;suffix&quot;)"),
        "unexpected svg: {svg}"
    );
}

#[test]
fn renders_engine_svg_label_for_lower_case_string_transform_unary_test() {
    let mut definition = parse_dmn_definition(FIRST_HIT_DMN).expect("FIRST fixture should parse");
    definition.decisions[0].decision_table.inputs[0]
        .input_expression
        .type_ref = Some("string".to_string());
    definition.decisions[0].decision_table.rules[0].input_entries[0].text =
        Some("lower case(?) = \"approved\"".to_string());
    let model = DmnModel::try_from(definition).expect("lower case unary test should parse");
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("lower-case-svg-label")
                .with_resource("loan-decision.dmn", model),
        )
        .expect("deployment");
    let definitions = engine
        .repository_service()
        .create_decision_query()
        .key("loanEligibility")
        .list()
        .expect("definitions");
    let generator = DmnSvgGenerator::new();

    let svg = generator
        .generate_engine_definition_svg(&definitions[0])
        .expect("engine DMN subset should render");

    assert!(
        svg.contains("lower case(?) = &quot;approved&quot;"),
        "unexpected svg: {svg}"
    );
}

#[test]
fn renders_engine_svg_label_for_not_wrapped_unary_test() {
    let mut definition = parse_dmn_definition(FIRST_HIT_DMN).expect("FIRST fixture should parse");
    definition.decisions[0].decision_table.inputs[0]
        .input_expression
        .type_ref = Some("string".to_string());
    definition.decisions[0].decision_table.rules[0].input_entries[0].text =
        Some("not(contains(?, \"vip\"))".to_string());
    let model = DmnModel::try_from(definition).expect("not unary test should parse");
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("not-svg-label").with_resource("loan-decision.dmn", model),
        )
        .expect("deployment");
    let definitions = engine
        .repository_service()
        .create_decision_query()
        .key("loanEligibility")
        .list()
        .expect("definitions");
    let generator = DmnSvgGenerator::new();

    let svg = generator
        .generate_engine_definition_svg(&definitions[0])
        .expect("engine DMN subset should render");

    assert!(
        svg.contains("not(contains(?, &quot;vip&quot;))"),
        "unexpected svg: {svg}"
    );
}
