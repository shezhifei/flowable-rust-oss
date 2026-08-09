use flowable_dmn_converter::{parse_dmn_definition, write_dmn_definition};
use flowable_dmn_model::{
    AuthorityRequirement, CollectOperator, Decision, DecisionRule, DecisionService, DecisionTable,
    DmnDefinition, HitPolicy, InputClause, KnowledgeSource, LiteralExpression, OutputClause,
    UnaryTests,
};

fn table(id: &str, policy: HitPolicy, operator: Option<CollectOperator>) -> DecisionTable {
    DecisionTable {
        id: id.to_string(),
        hit_policy: policy,
        collect_operator: operator,
        inputs: vec![InputClause {
            id: Some(format!("{id}_input")),
            label: Some("Age".to_string()),
            input_number: 1,
            input_expression: LiteralExpression {
                id: Some(format!("{id}_expression")),
                type_ref: Some("integer".to_string()),
                text: Some("age".to_string()),
            },
        }],
        outputs: vec![OutputClause {
            id: Some(format!("{id}_output")),
            label: Some("Band".to_string()),
            name: Some("band".to_string()),
            type_ref: Some("string".to_string()),
            output_values: Some(UnaryTests {
                id: Some(format!("{id}_allowed")),
                text: Some("\"minor\",\"adult\"".to_string()),
            }),
            output_number: 1,
        }],
        rules: vec![DecisionRule {
            id: Some(format!("{id}_rule")),
            rule_number: 1,
            input_entries: vec![UnaryTests {
                id: Some(format!("{id}_input_entry")),
                text: Some("< 18".to_string()),
            }],
            output_entries: vec![LiteralExpression {
                id: Some(format!("{id}_output_entry")),
                type_ref: None,
                text: Some("\"minor\"".to_string()),
            }],
        }],
    }
}

fn definition(policy: HitPolicy, operator: Option<CollectOperator>) -> DmnDefinition {
    DmnDefinition {
        id: Some("definitions_1".to_string()),
        name: Some("Age decisions".to_string()),
        namespace: Some("https://flowable.org/modeler/tests".to_string()),
        expression_language: Some("https://www.omg.org/spec/DMN/20191111/FEEL/".to_string()),
        type_language: Some("http://www.w3.org/2001/XMLSchema".to_string()),
        exporter: Some("Flowable Modeler".to_string()),
        exporter_version: Some("1.0".to_string()),
        decisions: vec![Decision {
            id: "ageDecision".to_string(),
            name: Some("Age decision".to_string()),
            decision_table: table("ageTable", policy, operator),
            required_decisions: Vec::new(),
        }],
        ..DmnDefinition::default()
    }
}

fn assert_roundtrip(mut expected: DmnDefinition) {
    let xml = write_dmn_definition(&expected).unwrap();
    let parsed = parse_dmn_definition(&xml).unwrap();
    let editor_json = serde_json::to_vec(&parsed).unwrap();
    let restored: DmnDefinition = serde_json::from_slice(&editor_json).unwrap();
    let restored_xml = write_dmn_definition(&restored).unwrap();
    let mut actual = parse_dmn_definition(&restored_xml).unwrap();
    expected.namespaces.clear();
    actual.namespaces.clear();
    assert_eq!(actual, expected, "round-trip XML:\n{restored_xml}");
}

#[test]
fn writes_first_hit_policy_table() {
    assert_roundtrip(definition(HitPolicy::First, None));
}

#[test]
fn writes_collect_table_with_aggregation() {
    assert_roundtrip(definition(HitPolicy::Collect, Some(CollectOperator::Sum)));
}

#[test]
fn writes_unique_table_with_output_values() {
    assert_roundtrip(definition(HitPolicy::Unique, None));
}

#[test]
fn writes_decision_references_and_decision_service() {
    let mut model = definition(HitPolicy::Any, None);
    model.decisions.push(Decision {
        id: "approvalDecision".to_string(),
        name: Some("Approval".to_string()),
        decision_table: table("approvalTable", HitPolicy::Priority, None),
        required_decisions: vec!["ageDecision".to_string()],
    });
    model.decision_services.push(DecisionService {
        id: "service_1".to_string(),
        name: "Eligibility service".to_string(),
        required_decisions: vec!["ageDecision".to_string()],
        output_decisions: vec!["approvalDecision".to_string()],
    });
    assert_roundtrip(model);
}

#[test]
fn writes_knowledge_sources_and_authority_requirements() {
    let mut model = definition(HitPolicy::RuleOrder, None);
    model.knowledge_sources.push(KnowledgeSource {
        id: "policy".to_string(),
        name: "Eligibility policy".to_string(),
        description: Some("Policy maintained by Operations".to_string()),
        type_: None,
        owner: None,
    });
    model.authority_requirements.push(AuthorityRequirement {
        id: "authority_1".to_string(),
        required_authority: Some("policy".to_string()),
        required_decision: Some("ageDecision".to_string()),
        decision: Some("ageDecision".to_string()),
    });
    assert_roundtrip(model);
}
