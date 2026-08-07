use flowable_cmmn_converter::parse_cmmn_definitions;
use flowable_cmmn_image_generator::{
    CmmnAdvancedSvgGeneratorOptions, CmmnSvgGenerator, CmmnSvgGeneratorError,
    CmmnSvgGeneratorOptions,
};

const OWNED_SUBSET_CMMN: &str = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="caseA" name="Case A">
    <casePlanModel id="planModelA" name="Plan Model A" autoComplete="false">
      <planItem id="planItemStage" name="Review Stage" definitionRef="reviewStage" />
      <planItem id="planItemRootTask" name="Root Task" definitionRef="rootTask" />
      <stage id="reviewStage" name="Review Stage" autoComplete="true">
        <planItem id="planItemNestedTask" name="Prepare Review" definitionRef="prepareReview" />
        <humanTask id="prepareReview" name="Prepare Review" isBlocking="false" />
      </stage>
      <humanTask id="rootTask" name="Root Task" isBlocking="true" />
    </casePlanModel>
  </case>
</definitions>
"#;

#[test]
fn renders_owned_case_stage_human_task_svg_deterministically() {
    let definitions = parse_cmmn_definitions(OWNED_SUBSET_CMMN).expect("owned subset should parse");
    let generator = CmmnSvgGenerator::new();

    let svg = generator
        .generate_definitions_svg(&definitions)
        .expect("owned CMMN subset should render");

    let expected = concat!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"384\" height=\"300\" viewBox=\"0 0 384 300\" role=\"img\" aria-labelledby=\"cmmn-title-caseA\">\n",
        "<title id=\"cmmn-title-caseA\">Case A</title>\n",
        "<rect x=\"1\" y=\"1\" width=\"382\" height=\"298\" rx=\"12\" fill=\"#f8fafc\" stroke=\"#0f172a\" stroke-width=\"2\"/>\n",
        "<rect x=\"24\" y=\"24\" width=\"336\" height=\"44\" rx=\"10\" fill=\"#0f172a\"/>\n",
        "<text x=\"40\" y=\"51\" font-family=\"monospace\" font-size=\"18\" font-weight=\"700\" fill=\"#f8fafc\">Case A</text>\n",
        "<text x=\"40\" y=\"88\" font-family=\"monospace\" font-size=\"13\" font-weight=\"700\" fill=\"#334155\">Plan Model A</text>\n",
        "<rect x=\"24\" y=\"104\" width=\"336\" height=\"120\" rx=\"10\" fill=\"#dbeafe\" stroke=\"#2563eb\" stroke-width=\"2\"/>\n",
        "<text x=\"40\" y=\"128\" font-family=\"monospace\" font-size=\"14\" font-weight=\"700\" fill=\"#1e3a8a\">Stage: Review Stage</text>\n",
        "<rect x=\"40\" y=\"144\" width=\"304\" height=\"56\" rx=\"8\" fill=\"#ffffff\" stroke=\"#64748b\" stroke-width=\"1.5\"/>\n",
        "<text x=\"56\" y=\"167\" font-family=\"monospace\" font-size=\"13\" font-weight=\"700\" fill=\"#0f172a\">Prepare Review</text>\n",
        "<text x=\"56\" y=\"186\" font-family=\"monospace\" font-size=\"11\" fill=\"#475569\">humanTask | non-blocking</text>\n",
        "<rect x=\"24\" y=\"240\" width=\"336\" height=\"56\" rx=\"8\" fill=\"#ffffff\" stroke=\"#64748b\" stroke-width=\"1.5\"/>\n",
        "<text x=\"40\" y=\"263\" font-family=\"monospace\" font-size=\"13\" font-weight=\"700\" fill=\"#0f172a\">Root Task</text>\n",
        "<text x=\"40\" y=\"282\" font-family=\"monospace\" font-size=\"11\" fill=\"#475569\">humanTask | blocking</text>\n",
        "</svg>\n"
    );

    assert_eq!(svg, expected);
}

#[test]
fn rejects_advanced_cmmn_svg_options_structurally() {
    let definitions = parse_cmmn_definitions(OWNED_SUBSET_CMMN).expect("owned subset should parse");
    let generator = CmmnSvgGenerator::new();
    let options = CmmnSvgGeneratorOptions {
        advanced: CmmnAdvancedSvgGeneratorOptions {
            font_family: Some("Fira Sans".to_string()),
            color_scheme: Some("dark".to_string()),
            scale: None,
        },
    };

    let error = generator
        .generate_definitions_svg_with_options(&definitions, &options)
        .expect_err("advanced options must fail structurally");

    assert_eq!(
        error,
        CmmnSvgGeneratorError::UnsupportedOptions {
            options: vec!["font_family", "color_scheme"],
        }
    );
}
