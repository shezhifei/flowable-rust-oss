use flowable_dmn_converter::parse_dmn_definition;

#[test]
fn dmn_xml_converter_parses_valid_decision_table() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/"
        xmlns:flowable="http://flowable.org/dmn"
        id="dishDecision" name="Dish Decision" namespace="examples">
        <decision id="dish" name="Dish Decision">
            <decisionTable id="dishDecisionTable" hitPolicy="UNIQUE">
                <input id="input1" label="Season">
                    <inputExpression id="inputExpression1" typeRef="string">
                        <text>season</text>
                    </inputExpression>
                </input>
                <output id="output1" label="Dish" name="desiredDish" typeRef="string" />
                <rule id="rule1">
                    <inputEntry id="inputEntry1"><text>"Fall"</text></inputEntry>
                    <outputEntry id="outputEntry1"><text>"Spareribs"</text></outputEntry>
                </rule>
                <rule id="rule2">
                    <inputEntry id="inputEntry2"><text>"Spring"</text></inputEntry>
                    <outputEntry id="outputEntry2"><text>"Salad"</text></outputEntry>
                </rule>
            </decisionTable>
        </decision>
    </definitions>"#;

    let result = parse_dmn_definition(xml);
    assert!(result.is_ok(), "Valid DMN XML should parse");
    let definition = result.unwrap();
    assert_eq!(definition.decisions.len(), 1);
    assert_eq!(definition.decisions[0].id, "dish");
    assert_eq!(
        definition.decisions[0].name.as_deref(),
        Some("Dish Decision")
    );
}

#[test]
fn dmn_xml_converter_rejects_malformed_xml() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/">
        <decision id="d1" name="D1">
            <decisionTable id="dt1" hitPolicy="UNIQUE">
                <input id="i1">
                    <inputExpression id="ie1" typeRef="string">
                        <text>x</text>
                    </inputExpression>
                </input>
                <output id="o1" typeRef="string" />
            </decisionTable>
        </decision>
    </unclosed>"#;

    let result = parse_dmn_definition(xml);
    assert!(result.is_err(), "Malformed XML should fail");
}

#[test]
fn dmn_xml_converter_parses_decision_with_collect_aggregation() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/"
        id="collectDecision" name="Collect Decision" namespace="examples">
        <decision id="collect" name="Collect Decision">
            <decisionTable id="collectTable" hitPolicy="COLLECT" aggregation="SUM">
                <input id="input1">
                    <inputExpression id="ie1" typeRef="integer">
                        <text>amount</text>
                    </inputExpression>
                </input>
                <output id="output1" typeRef="integer" />
                <rule id="r1">
                    <inputEntry id="ie1"><text>&gt; 10</text></inputEntry>
                    <outputEntry id="oe1"><text>100</text></outputEntry>
                </rule>
            </decisionTable>
        </decision>
    </definitions>"#;

    let result = parse_dmn_definition(xml);
    assert!(result.is_ok(), "DMN with COLLECT aggregation should parse");
}

#[test]
fn dmn_xml_converter_parses_empty_definitions() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/"
        id="empty" name="Empty" namespace="examples">
    </definitions>"#;

    let result = parse_dmn_definition(xml);
    assert!(result.is_ok(), "Empty definitions should parse");
    let definition = result.unwrap();
    assert!(definition.decisions.is_empty());
}
