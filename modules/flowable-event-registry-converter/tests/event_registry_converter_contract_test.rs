use flowable_event_registry_converter::{
    channel_definition_to_json, event_definition_to_json, parse_channel_definition,
    parse_event_definition,
};
use flowable_event_registry_model::{ChannelType, EventPayloadField};
use serde_json::{Value, json};

fn parse_value(json_text: &str) -> Value {
    serde_json::from_str(json_text).expect("test json should be valid")
}

#[test]
fn parses_and_serializes_supported_channel_definition_shape() {
    let channel_json = json!({
        "id": "channel-1",
        "key": "ordersInbound",
        "name": "Orders inbound",
        "description": "Inbound orders channel",
        "channelType": "inbound",
        "resourceName": "orders-inbound.channel",
        "configuration": {
            "type": "in-memory",
            "destination": "orders-inbound",
            "deserializerType": "json"
        }
    })
    .to_string();

    let channel = parse_channel_definition(&channel_json).expect("channel should parse");
    assert_eq!(channel.id.as_deref(), Some("channel-1"));
    assert_eq!(channel.key, "ordersInbound");
    assert_eq!(channel.channel_type, ChannelType::Inbound);
    assert_eq!(
        channel.configuration.get("destination"),
        Some(&json!("orders-inbound"))
    );

    let serialized =
        channel_definition_to_json(&channel).expect("channel should serialize to json");
    assert_eq!(parse_value(&serialized), parse_value(&channel_json));
}

#[test]
fn parses_and_serializes_supported_event_definition_shape() {
    let event_json = json!({
        "id": "event-1",
        "key": "orderReceived",
        "name": "Order received",
        "description": "Inbound order event",
        "eventType": "order.received",
        "channelKey": "ordersInbound",
        "resourceName": "order-received.event",
        "payload": [
            { "name": "orderId", "type": "string" },
            { "name": "amount", "type": "integer" }
        ]
    })
    .to_string();

    let event = parse_event_definition(&event_json).expect("event should parse");
    assert_eq!(event.id.as_deref(), Some("event-1"));
    assert_eq!(event.key, "orderReceived");
    assert_eq!(event.event_type, "order.received");
    assert_eq!(event.channel_key, "ordersInbound");
    assert_eq!(
        event.payload,
        vec![
            EventPayloadField {
                name: "orderId".to_string(),
                field_type: "string".to_string(),
                required: None,
            },
            EventPayloadField {
                name: "amount".to_string(),
                field_type: "integer".to_string(),
                required: None,
            },
        ]
    );

    let serialized = event_definition_to_json(&event).expect("event should serialize to json");
    assert_eq!(parse_value(&serialized), parse_value(&event_json));
}

#[test]
fn rejects_deprecated_channel_shape_with_top_level_transport_fields() {
    let deprecated_channel_json = json!({
        "key": "ordersInbound",
        "name": "Orders inbound",
        "channelType": "inbound",
        "resourceName": "orders-inbound.channel",
        "type": "in-memory",
        "destination": "orders-inbound",
        "deserializerType": "json"
    })
    .to_string();

    let error =
        parse_channel_definition(&deprecated_channel_json).expect_err("deprecated shape must fail");
    let message = error.to_string();
    assert!(
        message.contains("type")
            || message.contains("destination")
            || message.contains("deserializerType"),
        "unexpected error message: {message}"
    );
}

#[test]
fn rejects_event_shapes_outside_m13_subset() {
    let unsupported_event_json = json!({
        "key": "orderReceived",
        "eventType": "order.received",
        "channelKey": "ordersInbound",
        "payload": [
            { "name": "orderId", "type": "string", "correlationParameter": true }
        ],
        "correlationParameters": [
            { "name": "customerId", "type": "string" }
        ]
    })
    .to_string();

    let error =
        parse_event_definition(&unsupported_event_json).expect_err("unsupported shape must fail");
    let message = error.to_string();
    assert!(
        message.contains("correlationParameters") || message.contains("correlationParameter"),
        "unexpected error message: {message}"
    );
}

#[test]
fn rejects_channel_configuration_when_not_an_object() {
    let invalid_channel_json = json!({
        "key": "ordersInbound",
        "channelType": "inbound",
        "configuration": ["not", "an", "object"]
    })
    .to_string();

    let error =
        parse_channel_definition(&invalid_channel_json).expect_err("invalid shape must fail");
    assert!(
        error.to_string().contains("configuration"),
        "unexpected error message: {error}"
    );
}
