mod test_support;

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_event_registry_service::{
    ChannelDefinitionUpdateRequest, EventDirection, EventInstanceStatus,
    EventRegistryDeploymentRequest, EventRegistryDeploymentResource, FlowableEventRegistryService,
    OutboundEventRequest,
};
use std::sync::Arc;
use native_tls::{Identity, TlsAcceptor};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc::{Receiver, channel};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use test_support::service;

struct CapturedRequest {
    request_line: String,
    body: String,
}

const LEAF_CERT_PEM: &str = r#"
-----BEGIN CERTIFICATE-----
MIIDIjCCAgqgAwIBAgIUGgfXqpFdtaiesHNFmhBuAdqQkocwDQYJKoZIhvcNAQEL
BQAwLDEqMCgGA1UEAwwhRmxvd2FibGUgRXZlbnQgUmVnaXN0cnkgVGVzdCBSb290
MB4XDTI2MDUzMDE5NTMxM1oXDTM2MDUyNzE5NTMxM1owFDESMBAGA1UEAwwJbG9j
YWxob3N0MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA1tVlUMUKajW0
99PRjijXVPG1wYN8uttDIQueqEsZ+HXt6LTd0UxaAI7gxAB8qmIPz8fo6HLVB/OO
L0+05f3uj6eTWLZTqD69Fk7PppUjg0uERefNsn+2c+boAmrMg1PYjtp7uaGILmKp
qLDM+SZG9nGpE8i7J++SJu5BGIOH7h81VNM+zQnqLMvLNB41hwjJqz5xCbeXvuAl
IwYAyHyF/hbcO0oFwy1fdlhZY1tEe3k8mKb5tabetv8v1cwpp5gdt13yXIdNaiwr
wR3+QZH3OUsvFPNI+o+f5YzOVNqRzsWsu7UtB1Cp5OUnE9AQyIVB33F6aRZt6qyj
47Tqu1vCsQIDAQABo1QwUjAaBgNVHREEEzARgglsb2NhbGhvc3SHBH8AAAEwDAYD
VR0TAQH/BAIwADAOBgNVHQ8BAf8EBAMCBaAwFgYDVR0lAQH/BAwwCgYIKwYBBQUH
AwEwDQYJKoZIhvcNAQELBQADggEBAFG1W2beqKADedOLIvuI1k5j6kYfZu6rr9ES
SbA6hc3sIofqt8cKT+aEdPbvlsSpG117Jv90Q46ErDNddjfCN4Z5I7rVnopxmySF
0dQRsfbbtfy3GVeZ5SugCqzbNINYH2bwJDL400s52ZONeLZd8sdv/vMS26QR6+mu
IFjVfqPi+votPWBcKFGFUHiFJhW/D+1P5BQVAqYgAIT4ne00+eWhVfl0oSyMKnHz
L5lJJ+ospmI6BgjHHpk1r2pr0fvF/kHQAsQDEXIVjWzUXBlazwN/rhaou55gp7jb
X1ft3e9zbSnsBTmhMIoQCLuv0JBixHFztW1PldGlLNNkoUVJAH8=
-----END CERTIFICATE-----
"#;

const CA_CERT_PEM: &str = r#"
-----BEGIN CERTIFICATE-----
MIIDCTCCAfGgAwIBAgIUP4GSZC2FFgMBrXXygAzFOrmvwIkwDQYJKoZIhvcNAQEL
BQAwLDEqMCgGA1UEAwwhRmxvd2FibGUgRXZlbnQgUmVnaXN0cnkgVGVzdCBSb290
MB4XDTI2MDUzMDE5NTMxM1oXDTM2MDUyNzE5NTMxM1owLDEqMCgGA1UEAwwhRmxv
d2FibGUgRXZlbnQgUmVnaXN0cnkgVGVzdCBSb290MIIBIjANBgkqhkiG9w0BAQEF
AAOCAQ8AMIIBCgKCAQEAp9/OG/yYWi31ErErfOxhux2CCk9ajJguQM1G6ZgqeZEt
zbbwhLrqWsjIkgWKJQVTO6JzT6ro1u9xTALGGO+Z8tqyjrFyELJMgEaTJg3DbPc8
ZHE0Vf83Dlp5kFHw++vfHOqp94SsS+/B91y4inud7BEQEu8Io/Oo6tcE+J6fqY+R
bDeWDa+50jFuCSrdv1zUpm0G4bMb7ZHRq5lWK+ZRma55ZcAU/5/zi9JuLYJnPzXF
QZBy8BugKSuyT9yiY0aAXzf19nDiYs5CM70+aDy1RMBzqSXqboARM+iBHaIrrPDp
4pE5jELkGItJgq7tVU1k86oByg9dnw22xv9+RrhKywIDAQABoyMwITAPBgNVHRMB
Af8EBTADAQH/MA4GA1UdDwEB/wQEAwIBhjANBgkqhkiG9w0BAQsFAAOCAQEAmdmQ
jfOBRXwWyLXLNbxFgZvoxA8PDCqiGSwfZXI4TlKtuTBEitDPv5KA/1KsP3QSRuSU
wuNHq15Tzex+jAgV/yzQgDux+1G//I0BTExKCx3BgeADE7ssqcPXM4ZrgueVMQUO
szEmm472abYYNIPrcWBBeg1AQdFlmBEc6+oz7XVrJNQGoLIsuGK0O7/v0iW2EIal
K2ltX9DriJn1412DTD7N7FDP1fVsAFcvQ/745oOAmFLZAbaKAVbCQkpqOoWB3aqV
M46mS/jrKL2pxkW0G+5uMBUkZ/nI+15sD5SApL41l6JxByAgisFoUa/DFaQYYgHH
UT+iOeHZVaSx9bvb3g==
-----END CERTIFICATE-----
"#;

const LEAF_KEY_PEM: &str = r#"
-----BEGIN PRIVATE KEY-----
MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQDW1WVQxQpqNbT3
09GOKNdU8bXBg3y620MhC56oSxn4de3otN3RTFoAjuDEAHyqYg/Px+joctUH844v
T7Tl/e6Pp5NYtlOoPr0WTs+mlSODS4RF582yf7Zz5ugCasyDU9iO2nu5oYguYqmo
sMz5Jkb2cakTyLsn75Im7kEYg4fuHzVU0z7NCeosy8s0HjWHCMmrPnEJt5e+4CUj
BgDIfIX+Ftw7SgXDLV92WFljW0R7eTyYpvm1pt62/y/VzCmnmB23XfJch01qLCvB
Hf5Bkfc5Sy8U80j6j5/ljM5U2pHOxay7tS0HUKnk5ScT0BDIhUHfcXppFm3qrKPj
tOq7W8KxAgMBAAECggEAEYzOdKIaiLSpSCzNcklgPFGgOuL5iu/W9KZU9R1os+uf
+Ah84NhvUQGOPdTNJyodoPUmz+s//4dKnUJr3ihq/a2yrTOWP/W06SA4Rgm28S0R
IDJtX/o46hAtJRo00AgTqXZFvaA2VmIiLZd/FyqkvDgEZPGOZuPJhiHYJYWEbDpW
eqT7Pv8myKzR6gCjTmIQzkUEerOFx8g3s2aT0BOn6s/YCBbLPDpztLtgUYpaUkT5
XSgKBu1Dav3yu/8MTGKN4DClvY7QOX2ABAK/RXZ1fx85mtv4jQ/PMo7636sYCgOz
92ZPKtM5ohUFBC+ns79MjJpBJ9ViPypPY+RKVht5pQKBgQD2MYWbpmVSinyQ+KMz
Xh1D5J9W6T6CZVSA/wDbjBC+wyCDm0TTW9cmewQkS9hmx1gZ0IIX/uSLPyksRPnB
L1KRPm6KpD0vNsR+14ZHFL7sOacxMFe8q2qxApyWQZgeTixhlEdW8Nlvt7I4XxZ0
J5ZcLGMfnypSKNR8CkXBdqOVSwKBgQDfZBd8Tk1UAEc+HS08BsBuTRIhwyKUVfp8
RTrN8v2ebhbtIJGoHgU3CopLhZJiYkDiaiCKbmiMZVK0Kg42Yp4rPresbpPrNsF4
Qt4qGMh1VXcktNcPXPItiYXHKLYr6nUu90/9pQub0A835zdCg3f2pAgtwrIB7d3F
fZ9oHlnWcwKBgQCtU9g46UFUh2ODvUlJFO2NqxvzWGtF6ok/+EhmSYpQg5gUj/A4
zeP/l4Qm+a71TUtdgUrWEgJddq5KGJWtyN9cmpPA0DizUN+uXZaP3K8+KKjpHJvo
nNaUoL4Vm0C5tVfRq08+inrLCI1U2r04MdbONgHjdW+aQFy4p5LMzfYFWQKBgAKN
lUshZfbYzfeiw7qU5Swdi2CBZ2rElMlIzUQ/S7C811w8bA280hhv8Watjx+6ub0c
s2SBoIZCPjC67lCmzeH2pIi+9sfQZ2Old/6JK/lTUbpEqtSNHmNw1+uPxo0378Dq
qKpgcYKFXTcpWFNVR1C1TTagrAIjos44AlNhTWuLAoGBAIC5CebIEx0w1bmydoc4
bi1HNvVp2rWvmQRuhrh/Nnkpew/ajzwJE1NAiZcraXARzOTUf1Q+X3S6AGR4EDTO
Xj1JN+mYBEDPsheK5oP4Ckf1J+hs0b/yvgBuK1GvPqNKlVHS+dxtUbWVWfke4vyq
eRQHAevrJ1slXhg5YmXdnTgE
-----END PRIVATE KEY-----
"#;

fn start_http_server(status_code: u16) -> (String, Receiver<CapturedRequest>, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (sender, receiver) = channel();

    let handle = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut request = Vec::new();
            let mut buffer = [0_u8; 512];
            let mut header_end = None;
            let mut content_length = 0_usize;

            loop {
                let bytes_read = stream.read(&mut buffer).unwrap();
                if bytes_read == 0 {
                    break;
                }

                request.extend_from_slice(&buffer[..bytes_read]);

                if header_end.is_none() {
                    header_end = find_header_end(&request);
                    if let Some(end) = header_end {
                        let headers = String::from_utf8_lossy(&request[..end]);
                        content_length = headers
                            .lines()
                            .find_map(|line| {
                                let (name, value) = line.split_once(':')?;
                                name.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().ok())
                                    .flatten()
                            })
                            .unwrap_or(0);
                    }
                }

                if let Some(end) = header_end
                    && request.len() >= end + 4 + content_length
                {
                    break;
                }
            }

            let end = header_end.unwrap();
            let request_line = String::from_utf8_lossy(&request[..end])
                .lines()
                .next()
                .unwrap()
                .to_string();
            let body = String::from_utf8_lossy(&request[end + 4..]).to_string();
            sender.send(CapturedRequest { request_line, body }).unwrap();

            let reason = if (200..300).contains(&status_code) {
                "OK"
            } else {
                "Error"
            };
            let response = format!(
                "HTTP/1.1 {status_code} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            stream.write_all(response.as_bytes()).unwrap();
        }
    });

    (format!("http://{address}/events"), receiver, handle)
}

fn start_https_server(status_code: u16) -> (String, Receiver<CapturedRequest>, JoinHandle<()>) {
    let identity = Identity::from_pkcs8(
        LEAF_CERT_PEM.trim().as_bytes(),
        LEAF_KEY_PEM.trim().as_bytes(),
    )
    .unwrap();
    let acceptor = TlsAcceptor::new(identity).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let (sender, receiver) = channel();

    let handle = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            match listener.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    let mut stream = acceptor.accept(stream).unwrap();
                    read_and_respond(&mut stream, status_code, sender);
                    return;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(_) => return,
            }
        }
    });

    (
        format!("https://localhost:{}/events", address.port()),
        receiver,
        handle,
    )
}

fn read_and_respond<T>(
    stream: &mut T,
    status_code: u16,
    sender: std::sync::mpsc::Sender<CapturedRequest>,
) where
    T: Read + Write,
{
    let mut request = Vec::new();
    let mut buffer = [0_u8; 512];
    let mut header_end = None;
    let mut content_length = 0_usize;

    loop {
        let bytes_read = stream.read(&mut buffer).unwrap();
        if bytes_read == 0 {
            break;
        }

        request.extend_from_slice(&buffer[..bytes_read]);

        if header_end.is_none() {
            header_end = find_header_end(&request);
            if let Some(end) = header_end {
                let headers = String::from_utf8_lossy(&request[..end]);
                content_length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
            }
        }

        if let Some(end) = header_end
            && request.len() >= end + 4 + content_length
        {
            break;
        }
    }

    let end = header_end.unwrap();
    let request_line = String::from_utf8_lossy(&request[..end])
        .lines()
        .next()
        .unwrap()
        .to_string();
    let body = String::from_utf8_lossy(&request[end + 4..]).to_string();
    sender.send(CapturedRequest { request_line, body }).unwrap();

    let reason = if (200..300).contains(&status_code) {
        "OK"
    } else {
        "Error"
    };
    let response = format!(
        "HTTP/1.1 {status_code} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(response.as_bytes()).unwrap();
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

fn deploy_rest_outbound_channel(
    service_name: &str,
    endpoint: &str,
) -> flowable_event_registry_service::FlowableEventRegistryService {
    deploy_rest_outbound_channel_with_configuration(
        service_name,
        json!({
            "type": "rest",
            "destination": endpoint,
            "serializerType": "json"
        }),
    )
}

fn deploy_rest_outbound_channel_with_configuration(
    service_name: &str,
    configuration: Value,
) -> flowable_event_registry_service::FlowableEventRegistryService {
    let service = service(service_name);
    service
        .deploy(EventRegistryDeploymentRequest {
            name: "REST outbound deployment".to_string(),
            category: None,
            parent_deployment_id: None,
            tenant_id: None,
            resources: vec![
                EventRegistryDeploymentResource {
                    resource_name: "order-published-rest.event".to_string(),
                    resource: json!({
                        "key": "orderPublishedRest",
                        "name": "Order published REST",
                        "eventType": "order.published",
                        "channelKey": "ordersRestOutbound",
                        "resourceName": "order-published-rest.event",
                        "payload": [
                            { "name": "orderId", "type": "string" }
                        ]
                    })
                    .to_string(),
                },
                EventRegistryDeploymentResource {
                    resource_name: "orders-rest-outbound.channel".to_string(),
                    resource: {
                        let mut channel = json!({
                            "key": "ordersRestOutbound",
                            "name": "Orders REST outbound",
                            "channelType": "outbound",
                            "resourceName": "orders-rest-outbound.channel"
                        });
                        channel
                            .as_object_mut()
                            .unwrap()
                            .extend(configuration.as_object().unwrap().clone());
                        channel.to_string()
                    },
                },
            ],
        })
        .unwrap();
    service
}

#[test]
fn rest_outbound_channel_posts_event_payload_to_https_endpoint() {
    let (endpoint, receiver, handle) = start_https_server(204);
    let service = deploy_rest_outbound_channel_with_configuration(
        "event-registry-rest-channel-dispatch-https",
        json!({
            "type": "rest",
            "destination": endpoint,
            "serializerType": "json",
            "tlsRootCertificatePem": CA_CERT_PEM
        }),
    );

    let delivery = service
        .publish_outbound_event(OutboundEventRequest {
            event_definition_key: "orderPublishedRest".to_string(),
            event_payload: json!({ "orderId": "REST-HTTPS" }),
            tenant_id: None,
        })
        .expect("REST outbound channel should support HTTPS destinations");

    assert_eq!(delivery.status, EventInstanceStatus::Published);

    let request = receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("REST channel should POST the outbound event over HTTPS");
    handle.join().unwrap();

    assert_eq!(request.request_line, "POST /events HTTP/1.1");
    let body: Value = serde_json::from_str(&request.body).unwrap();
    assert_eq!(
        body,
        json!({
            "event_type": "order.published",
            "payload": { "orderId": "REST-HTTPS" }
        })
    );
}

#[test]
fn rest_outbound_channel_posts_event_payload_to_configured_endpoint() {
    let (endpoint, receiver, handle) = start_http_server(204);
    let service = deploy_rest_outbound_channel("event-registry-rest-channel-dispatch", &endpoint);

    let delivery = service
        .publish_outbound_event(OutboundEventRequest {
            event_definition_key: "orderPublishedRest".to_string(),
            event_payload: json!({ "orderId": "REST-100" }),
            tenant_id: None,
        })
        .unwrap();

    assert_eq!(delivery.status, EventInstanceStatus::Published);

    let request = receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("REST channel should POST the outbound event to the configured destination");
    handle.join().unwrap();

    assert_eq!(request.request_line, "POST /events HTTP/1.1");
    let body: Value = serde_json::from_str(&request.body).unwrap();
    assert_eq!(
        body,
        json!({
            "event_type": "order.published",
            "payload": { "orderId": "REST-100" }
        })
    );
}

#[test]
fn rest_outbound_ssrf_guard_rejects_private_destination_without_path_echo() {
    // Production-default service (strict SSRF) — not the test_support helper which
    // opts into private networks for local mock servers.
    let service = FlowableEventRegistryService::new(Arc::new(ProcessEngine::new(
        "event-registry-rest-ssrf-guard".to_string(),
    )));
    service
        .deploy(EventRegistryDeploymentRequest {
            name: "REST SSRF guard deployment".to_string(),
            category: None,
            parent_deployment_id: None,
            tenant_id: None,
            resources: vec![
                EventRegistryDeploymentResource {
                    resource_name: "order-published-ssrf.event".to_string(),
                    resource: json!({
                        "key": "orderPublishedSsrf",
                        "name": "Order published SSRF",
                        "eventType": "order.published.ssrf",
                        "channelKey": "ordersSsrfOutbound",
                        "resourceName": "order-published-ssrf.event",
                        "payload": [
                            { "name": "orderId", "type": "string" }
                        ]
                    })
                    .to_string(),
                },
                EventRegistryDeploymentResource {
                    resource_name: "orders-ssrf-outbound.channel".to_string(),
                    resource: json!({
                        "key": "ordersSsrfOutbound",
                        "name": "Orders SSRF outbound",
                        "channelType": "outbound",
                        "type": "rest",
                        "destination": "http://127.0.0.1:9/secret/path?token=abc",
                        "serializerType": "json",
                        "resourceName": "orders-ssrf-outbound.channel"
                    })
                    .to_string(),
                },
            ],
        })
        .unwrap();

    let result = service.publish_outbound_event(OutboundEventRequest {
        event_definition_key: "orderPublishedSsrf".to_string(),
        event_payload: json!({ "orderId": "SSRF-1" }),
        tenant_id: None,
    });

    let err = result.expect_err("private destination must be rejected by SSRF guard");
    let msg = err.to_string();
    assert!(
        msg.contains("SSRF guard") || msg.contains("blocked"),
        "expected SSRF denial message, got: {msg}"
    );
    assert!(
        msg.contains("allow_private_networks") || msg.contains("allowed_private_hosts"),
        "error should mention configuration escape hatches: {msg}"
    );
    assert!(
        !msg.contains("/secret") && !msg.contains("token=abc"),
        "error must not echo path/query for blind probing: {msg}"
    );
}

#[test]
fn rest_outbound_channel_marks_delivery_failed_when_endpoint_returns_non_success() {
    let (endpoint, receiver, handle) = start_http_server(500);
    let service =
        deploy_rest_outbound_channel("event-registry-rest-channel-dispatch-failure", &endpoint);

    let result = service.publish_outbound_event(OutboundEventRequest {
        event_definition_key: "orderPublishedRest".to_string(),
        event_payload: json!({ "orderId": "REST-500" }),
        tenant_id: None,
    });

    assert!(
        result.is_err(),
        "non-2xx REST dispatch should be reported as an outbound error"
    );

    let request = receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("REST channel should still attempt the outbound POST before reporting failure");
    handle.join().unwrap();
    assert_eq!(request.request_line, "POST /events HTTP/1.1");

    let deliveries = service
        .create_event_instance_delivery_query()
        .direction(EventDirection::Outbound)
        .status(EventInstanceStatus::Failed)
        .page(0, 10)
        .list_page()
        .unwrap();
    assert_eq!(deliveries.total, 1);
    let delivery = &deliveries.data[0];
    assert_eq!(delivery.status, EventInstanceStatus::Failed);
    assert_eq!(
        delivery.status_history,
        vec![EventInstanceStatus::Created, EventInstanceStatus::Failed]
    );
    assert_eq!(delivery.retry_count, 0);
    assert_eq!(delivery.last_retry_at, None);
    assert!(delivery.last_failure_at.is_some());
    assert!(delivery.next_retry_at.is_some());
    assert!(
        delivery
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("status 500")),
        "last_error should capture the REST failure status"
    );

    let created_deliveries = service
        .create_event_instance_delivery_query()
        .direction(EventDirection::Outbound)
        .status(EventInstanceStatus::Created)
        .page(0, 10)
        .list_page()
        .unwrap();
    assert_eq!(created_deliveries.total, 0);
}

#[test]
fn retry_event_delivery_redispatches_failed_rest_outbound_delivery() {
    let (failing_endpoint, failing_receiver, failing_handle) = start_http_server(503);
    let service = deploy_rest_outbound_channel(
        "event-registry-rest-channel-dispatch-retry",
        &failing_endpoint,
    );

    service
        .publish_outbound_event(OutboundEventRequest {
            event_definition_key: "orderPublishedRest".to_string(),
            event_payload: json!({ "orderId": "REST-RETRY" }),
            tenant_id: None,
        })
        .unwrap_err();
    failing_receiver
        .recv_timeout(Duration::from_secs(2))
        .unwrap();
    failing_handle.join().unwrap();

    let delivery = service
        .create_event_instance_delivery_query()
        .direction(EventDirection::Outbound)
        .status(EventInstanceStatus::Failed)
        .page(0, 10)
        .list_page()
        .unwrap()
        .data
        .pop()
        .unwrap();

    let (retry_endpoint, retry_receiver, retry_handle) = start_http_server(204);
    let channel = service
        .create_channel_definition_query()
        .key("ordersRestOutbound")
        .list()
        .unwrap()
        .pop()
        .unwrap();
    service
        .update_channel_definition(
            &channel.id,
            ChannelDefinitionUpdateRequest {
                name: None,
                configuration: Some(json!({
                    "type": "rest",
                    "destination": retry_endpoint,
                    "serializerType": "json"
                })),
            },
        )
        .unwrap();

    let retried = service.retry_event_delivery(&delivery.id).unwrap();

    assert_eq!(retried.status, EventInstanceStatus::Published);
    assert_eq!(
        retried.status_history,
        vec![
            EventInstanceStatus::Created,
            EventInstanceStatus::Failed,
            EventInstanceStatus::Published
        ]
    );
    assert_eq!(retried.retry_count, 1);
    assert!(retried.last_retry_at.is_some());
    assert_eq!(retried.last_error, None);
    assert_eq!(retried.last_failure_at, None);
    assert_eq!(retried.next_retry_at, None);

    let request = retry_receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("retry should redispatch the created REST outbound delivery");
    retry_handle.join().unwrap();
    let body: Value = serde_json::from_str(&request.body).unwrap();
    assert_eq!(
        body,
        json!({
            "event_type": "order.published",
            "payload": { "orderId": "REST-RETRY" }
        })
    );
}

#[test]
fn retry_event_delivery_keeps_failed_status_and_updates_retry_metadata_when_redispatch_fails() {
    let (failing_endpoint, failing_receiver, failing_handle) = start_http_server(500);
    let service = deploy_rest_outbound_channel(
        "event-registry-rest-channel-dispatch-retry-failure",
        &failing_endpoint,
    );

    service
        .publish_outbound_event(OutboundEventRequest {
            event_definition_key: "orderPublishedRest".to_string(),
            event_payload: json!({ "orderId": "REST-RETRY-FAIL" }),
            tenant_id: None,
        })
        .unwrap_err();
    failing_receiver
        .recv_timeout(Duration::from_secs(2))
        .unwrap();
    failing_handle.join().unwrap();

    let delivery = service
        .create_event_instance_delivery_query()
        .direction(EventDirection::Outbound)
        .status(EventInstanceStatus::Failed)
        .page(0, 10)
        .list_page()
        .unwrap()
        .data
        .pop()
        .unwrap();
    let first_failure_at = delivery.last_failure_at.unwrap();
    let first_next_retry_at = delivery.next_retry_at.unwrap();

    let (retry_endpoint, retry_receiver, retry_handle) = start_http_server(502);
    let channel = service
        .create_channel_definition_query()
        .key("ordersRestOutbound")
        .list()
        .unwrap()
        .pop()
        .unwrap();
    service
        .update_channel_definition(
            &channel.id,
            ChannelDefinitionUpdateRequest {
                name: None,
                configuration: Some(json!({
                    "type": "rest",
                    "destination": retry_endpoint,
                    "serializerType": "json"
                })),
            },
        )
        .unwrap();

    service.retry_event_delivery(&delivery.id).unwrap_err();

    retry_receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("failed retry should still redispatch the REST outbound delivery");
    retry_handle.join().unwrap();

    let retried = service
        .create_event_instance_delivery_query()
        .direction(EventDirection::Outbound)
        .status(EventInstanceStatus::Failed)
        .page(0, 10)
        .list_page()
        .unwrap()
        .data
        .pop()
        .unwrap();

    assert_eq!(retried.status, EventInstanceStatus::Failed);
    assert_eq!(
        retried.status_history,
        vec![EventInstanceStatus::Created, EventInstanceStatus::Failed]
    );
    assert_eq!(retried.retry_count, 1);
    assert!(retried.last_retry_at.is_some());
    assert!(retried.last_failure_at.unwrap() >= first_failure_at);
    assert!(retried.next_retry_at.unwrap() >= first_next_retry_at);
    assert!(
        retried
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("status 502")),
        "last_error should be replaced by the most recent REST failure"
    );
}
