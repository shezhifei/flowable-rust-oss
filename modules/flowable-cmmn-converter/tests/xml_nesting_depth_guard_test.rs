//! M3 guard: XML element nesting is bounded *before* roxmltree parses.
//!
//! roxmltree's parser recurses per element, so an over-deep document overflows
//! the thread stack inside `Document::parse` and aborts the process. These tests
//! exist to prove the rejection happens pre-parse: without the prescan, the
//! `far_beyond_the_cap` case below does not fail, it kills the test binary.

use flowable_cmmn_converter::{CmmnConverterError, parse_cmmn_definitions};

/// Must match `MAX_XML_NESTING_DEPTH` in the converter.
const CAP: usize = 64;

/// `<definitions>` wrapping a chain of `child_levels` nested elements, so total
/// element depth is `child_levels + 1` (root element counts as level 1).
fn nested_document(child_levels: usize) -> String {
    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL">"#,
    );
    for _ in 0..child_levels {
        xml.push_str("<a>");
    }
    for _ in 0..child_levels {
        xml.push_str("</a>");
    }
    xml.push_str("</definitions>");
    xml
}

fn is_nesting_rejection(error: &CmmnConverterError) -> bool {
    matches!(error, CmmnConverterError::InvalidXml(message) if message.contains("nesting"))
}

#[test]
fn nesting_at_the_cap_is_not_rejected_for_depth() {
    // Depth exactly CAP: allowed past the depth guard. The converter may still
    // reject it for unknown elements -- that is fine, it must just not be the
    // nesting error, and it must not crash.
    let xml = nested_document(CAP - 1);
    if let Err(error) = parse_cmmn_definitions(&xml) {
        assert!(
            !is_nesting_rejection(&error),
            "depth {CAP} is at the cap and must not be rejected as too deep, got: {error:?}"
        );
    }
}

#[test]
fn nesting_one_past_the_cap_is_rejected() {
    let xml = nested_document(CAP);
    let error = parse_cmmn_definitions(&xml).expect_err("depth CAP+1 must be rejected");
    assert!(
        is_nesting_rejection(&error),
        "expected a nesting rejection, got: {error:?}"
    );
}

#[test]
fn far_beyond_the_cap_is_rejected_without_reaching_the_parser() {
    // 5000 levels overflows roxmltree's parser on every build measured. Reaching
    // this assertion at all is the point: the prescan rejected it first.
    let xml = nested_document(5000);
    let error = parse_cmmn_definitions(&xml).expect_err("extreme depth must be rejected");
    assert!(
        is_nesting_rejection(&error),
        "expected a nesting rejection, got: {error:?}"
    );
}

#[test]
fn many_siblings_do_not_accumulate_into_depth() {
    // Depth is nesting, not element count: 5000 siblings stay at depth 2.
    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL">"#,
    );
    for _ in 0..5000 {
        xml.push_str("<a/>");
    }
    xml.push_str("</definitions>");
    if let Err(error) = parse_cmmn_definitions(&xml) {
        assert!(
            !is_nesting_rejection(&error),
            "sibling breadth must not count as nesting depth, got: {error:?}"
        );
    }
}

#[test]
fn a_shallow_document_is_never_rejected_for_nesting() {
    let xml = nested_document(3);
    if let Err(error) = parse_cmmn_definitions(&xml) {
        assert!(
            !is_nesting_rejection(&error),
            "a depth-4 document must not be rejected as too deep, got: {error:?}"
        );
    }
}
