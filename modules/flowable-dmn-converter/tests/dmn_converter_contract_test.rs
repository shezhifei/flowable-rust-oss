use flowable_dmn_converter::parse_dmn_definition;
use flowable_dmn_model::{CollectOperator, HitPolicy};

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

const OUTPUT_ORDER_DMN: &str = r#"
<definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/"
             id="risk-defs"
             name="Risk Decisions"
             namespace="http://flowable.org/dmn">
  <decision id="riskDecision" name="Risk Decision">
    <decisionTable id="riskDecisionTable" hitPolicy="OUTPUT ORDER">
      <input id="input1">
        <inputExpression id="inputExpression1" typeRef="string">
          <text>country</text>
        </inputExpression>
      </input>
      <output id="output1" name="riskBand" typeRef="string" />
      <rule id="rule1">
        <inputEntry id="inputEntry1"><text>'CN'</text></inputEntry>
        <outputEntry id="outputEntry1"><text>'LOW'</text></outputEntry>
      </rule>
    </decisionTable>
  </decision>
</definitions>
"#;

const PRIORITY_DMN: &str = r#"
<definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/"
             id="risk-defs"
             name="Risk Decisions"
             namespace="http://flowable.org/dmn">
  <decision id="riskDecision" name="Risk Decision">
    <decisionTable id="riskDecisionTable" hitPolicy="PRIORITY">
      <input id="input1">
        <inputExpression id="inputExpression1" typeRef="string">
          <text>country</text>
        </inputExpression>
      </input>
      <output id="output1" name="riskBand" typeRef="string">
        <outputValues id="riskValues">
          <text>'HIGH','MEDIUM','LOW'</text>
        </outputValues>
      </output>
      <rule id="rule1">
        <inputEntry><text>-</text></inputEntry>
        <outputEntry><text>'LOW'</text></outputEntry>
      </rule>
      <rule id="rule2">
        <inputEntry><text>'CN'</text></inputEntry>
        <outputEntry><text>'HIGH'</text></outputEntry>
      </rule>
    </decisionTable>
  </decision>
</definitions>
"#;

const RULE_ORDER_DMN: &str = r#"
<definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/"
             id="routing-defs"
             namespace="http://flowable.org/dmn">
  <decision id="routingDecision" name="Routing Decision">
    <decisionTable id="routingTable" hitPolicy="RULE ORDER">
      <input id="input1">
        <inputExpression id="inputExpression1" typeRef="string">
          <text>channel</text>
        </inputExpression>
      </input>
      <output id="output1" name="route" typeRef="string" />
      <rule id="rule1">
        <inputEntry><text>-</text></inputEntry>
        <outputEntry><text>'manual'</text></outputEntry>
      </rule>
      <rule id="rule2">
        <inputEntry><text>'email'</text></inputEntry>
        <outputEntry><text>'email-queue'</text></outputEntry>
      </rule>
    </decisionTable>
  </decision>
</definitions>
"#;

const UNIQUE_HIT_DMN: &str = r#"
<definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/"
             id="routing-defs"
             namespace="http://flowable.org/dmn">
  <decision id="routingDecision" name="Routing Decision">
    <decisionTable id="routingTable" hitPolicy="UNIQUE">
      <input id="input1">
        <inputExpression id="inputExpression1" typeRef="string">
          <text>channel</text>
        </inputExpression>
      </input>
      <output id="output1" name="route" typeRef="string" />
      <rule id="rule1">
        <inputEntry><text>'email'</text></inputEntry>
        <outputEntry><text>'email-queue'</text></outputEntry>
      </rule>
    </decisionTable>
  </decision>
</definitions>
"#;

const ANY_HIT_DMN: &str = r#"
<definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/"
             id="routing-defs"
             namespace="http://flowable.org/dmn">
  <decision id="routingDecision" name="Routing Decision">
    <decisionTable id="routingTable" hitPolicy="ANY">
      <input id="input1">
        <inputExpression id="inputExpression1" typeRef="string">
          <text>channel</text>
        </inputExpression>
      </input>
      <output id="output1" name="route" typeRef="string" />
      <rule id="rule1">
        <inputEntry><text>-</text></inputEntry>
        <outputEntry><text>'manual'</text></outputEntry>
      </rule>
      <rule id="rule2">
        <inputEntry><text>'email'</text></inputEntry>
        <outputEntry><text>'manual'</text></outputEntry>
      </rule>
    </decisionTable>
  </decision>
</definitions>
"#;

const COLLECT_HIT_DMN: &str = r#"
<definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/"
             id="routing-defs"
             namespace="http://flowable.org/dmn">
  <decision id="routingDecision" name="Routing Decision">
    <decisionTable id="routingTable" hitPolicy="COLLECT">
      <input id="input1">
        <inputExpression id="inputExpression1" typeRef="string">
          <text>channel</text>
        </inputExpression>
      </input>
      <output id="output1" name="route" typeRef="string" />
      <rule id="rule1">
        <inputEntry><text>-</text></inputEntry>
        <outputEntry><text>'manual'</text></outputEntry>
      </rule>
      <rule id="rule2">
        <inputEntry><text>'email'</text></inputEntry>
        <outputEntry><text>'email-queue'</text></outputEntry>
      </rule>
    </decisionTable>
  </decision>
</definitions>
"#;

const COLLECT_SUM_DMN: &str = r#"
<definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/"
             id="score-defs"
             namespace="http://flowable.org/dmn">
  <decision id="scoreDecision" name="Score Decision">
    <decisionTable id="scoreTable" hitPolicy="COLLECT" aggregation="SUM">
      <input id="input1">
        <inputExpression id="inputExpression1" typeRef="string">
          <text>channel</text>
        </inputExpression>
      </input>
      <output id="output1" name="score" typeRef="number" />
      <rule id="rule1">
        <inputEntry><text>-</text></inputEntry>
        <outputEntry><text>1</text></outputEntry>
      </rule>
    </decisionTable>
  </decision>
</definitions>
"#;

const COLLECT_COUNT_OPERATOR_DMN: &str = r#"
<definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/"
             id="score-defs"
             namespace="http://flowable.org/dmn">
  <decision id="scoreDecision" name="Score Decision">
    <decisionTable id="scoreTable" hitPolicy="COLLECT" collectOperator="COUNT">
      <input id="input1">
        <inputExpression id="inputExpression1" typeRef="string">
          <text>channel</text>
        </inputExpression>
      </input>
      <output id="output1" name="score" typeRef="number" />
      <rule id="rule1">
        <inputEntry><text>-</text></inputEntry>
        <outputEntry><text>1</text></outputEntry>
      </rule>
    </decisionTable>
  </decision>
</definitions>
"#;

#[test]
fn parses_owned_first_hit_policy_decision_table_fixture() {
    let definition = parse_dmn_definition(FIRST_HIT_DMN).expect("FIRST fixture should parse");

    assert_eq!(definition.id.as_deref(), Some("loan-defs"));
    assert_eq!(definition.name.as_deref(), Some("Loan Decisions"));
    assert_eq!(
        definition.namespace.as_deref(),
        Some("http://flowable.org/dmn")
    );
    assert_eq!(definition.decisions.len(), 1);

    let decision = &definition.decisions[0];
    assert_eq!(decision.id, "loanEligibility");
    assert_eq!(decision.name.as_deref(), Some("Loan Eligibility"));
    assert_eq!(decision.decision_table.id, "loanDecisionTable");
    assert_eq!(decision.decision_table.hit_policy, HitPolicy::First);
    assert_eq!(decision.decision_table.inputs.len(), 1);
    assert_eq!(decision.decision_table.outputs.len(), 2);
    assert_eq!(decision.decision_table.rules.len(), 2);

    let input = &decision.decision_table.inputs[0];
    assert_eq!(input.input_number, 1);
    assert_eq!(
        input.input_expression.id.as_deref(),
        Some("inputExpression1")
    );
    assert_eq!(input.input_expression.type_ref.as_deref(), Some("number"));
    assert_eq!(input.input_expression.text.as_deref(), Some("creditScore"));

    let first_output = &decision.decision_table.outputs[0];
    assert_eq!(first_output.output_number, 1);
    assert_eq!(first_output.id.as_deref(), Some("output1"));
    assert_eq!(first_output.name.as_deref(), Some("approved"));
    assert_eq!(first_output.type_ref.as_deref(), Some("boolean"));

    let first_rule = &decision.decision_table.rules[0];
    assert_eq!(first_rule.rule_number, 1);
    assert_eq!(first_rule.input_entries.len(), 1);
    assert_eq!(first_rule.output_entries.len(), 2);
    assert_eq!(
        first_rule.input_entries[0].id.as_deref(),
        Some("inputEntry1")
    );
    assert_eq!(first_rule.input_entries[0].text.as_deref(), Some("730"));
    assert_eq!(first_rule.output_entries[0].text.as_deref(), Some("true"));
    assert_eq!(first_rule.output_entries[1].text.as_deref(), Some("'LOW'"));
}

#[test]
fn parses_unique_and_any_hit_policy_decision_tables() {
    let unique = parse_dmn_definition(UNIQUE_HIT_DMN).expect("UNIQUE fixture should parse");
    assert_eq!(
        unique.decisions[0].decision_table.hit_policy,
        HitPolicy::Unique
    );

    let any = parse_dmn_definition(ANY_HIT_DMN).expect("ANY fixture should parse");
    assert_eq!(any.decisions[0].decision_table.hit_policy, HitPolicy::Any);
}

#[test]
fn parses_rule_order_hit_policy_decision_table() {
    let definition = parse_dmn_definition(RULE_ORDER_DMN).expect("RULE ORDER fixture should parse");

    assert_eq!(
        definition.decisions[0].decision_table.hit_policy,
        HitPolicy::RuleOrder
    );
    assert_eq!(definition.decisions[0].decision_table.rules.len(), 2);
}

#[test]
fn parses_output_order_and_priority_output_values() {
    let output_order =
        parse_dmn_definition(OUTPUT_ORDER_DMN).expect("OUTPUT ORDER fixture should parse");
    assert_eq!(
        output_order.decisions[0].decision_table.hit_policy,
        HitPolicy::OutputOrder
    );

    let priority = parse_dmn_definition(PRIORITY_DMN).expect("PRIORITY fixture should parse");
    let table = &priority.decisions[0].decision_table;
    assert_eq!(table.hit_policy, HitPolicy::Priority);
    let output_values = table.outputs[0]
        .output_values
        .as_ref()
        .expect("outputValues should be stored");
    assert_eq!(output_values.id.as_deref(), Some("riskValues"));
    assert_eq!(output_values.text.as_deref(), Some("'HIGH','MEDIUM','LOW'"));
}

#[test]
fn parses_collect_hit_policy_without_aggregation() {
    let collect = parse_dmn_definition(COLLECT_HIT_DMN).expect("COLLECT fixture should parse");

    assert_eq!(
        collect.decisions[0].decision_table.hit_policy,
        HitPolicy::Collect
    );
    assert_eq!(collect.decisions[0].decision_table.rules.len(), 2);
}

#[test]
fn parses_collect_hit_policy_with_aggregation_attribute() {
    let definition = parse_dmn_definition(COLLECT_SUM_DMN).expect("COLLECT SUM should parse");

    let table = &definition.decisions[0].decision_table;
    assert_eq!(table.hit_policy, HitPolicy::Collect);
    assert_eq!(table.collect_operator, Some(CollectOperator::Sum));
}

#[test]
fn parses_collect_hit_policy_with_collect_operator_attribute() {
    let definition =
        parse_dmn_definition(COLLECT_COUNT_OPERATOR_DMN).expect("COLLECT COUNT should parse");

    let table = &definition.decisions[0].decision_table;
    assert_eq!(table.hit_policy, HitPolicy::Collect);
    assert_eq!(table.collect_operator, Some(CollectOperator::Count));
}

#[test]
fn rejects_collect_aggregation_on_non_collect_hit_policy() {
    let dmn = COLLECT_SUM_DMN.replace("hitPolicy=\"COLLECT\"", "hitPolicy=\"FIRST\"");
    let error =
        parse_dmn_definition(&dmn).expect_err("aggregation without COLLECT hit policy must fail");

    assert!(
        error.to_string().contains("aggregation") && error.to_string().contains("COLLECT"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_unsupported_hit_policy_structurally() {
    let dmn = OUTPUT_ORDER_DMN.replace("OUTPUT ORDER", "UNSUPPORTED");
    let error = parse_dmn_definition(&dmn).expect_err("unsupported hit policy must fail");
    assert!(
        error.to_string().contains("hitPolicy") && error.to_string().contains("UNSUPPORTED"),
        "unexpected error: {error}"
    );
}
