//! XML nesting-depth guard (M3) for the DMN converter.
//!
//! The guard must run *before* roxmltree parses: roxmltree's parser recurses
//! per element, so a deeply-nested document overflows the thread stack inside
//! `Document::parse_with_options` and aborts the process. A post-parse tree
//! walk can never fire for the documents it exists to reject.
//!
//! Measured on this workspace (chain of nested elements, roxmltree 0.20):
//! debug build on a ~1 MiB main thread dies at ~200 levels, debug on a 2 MiB
//! spawned/test thread at ~400, release on a 2 MiB tokio worker at ~3000.
//! The cap therefore sits well below all of them; the deepest real DMN/CMMN
//! fixture in this repository is 7 levels.

use flowable_dmn_converter::{DmnConverterError, parse_dmn_definition};

/// Mirrors `MAX_XML_NESTING_DEPTH` in the converter (root element = depth 1).
const CAP: usize = 64;

const DMN_NS: &str = "https://www.omg.org/spec/DMN/20191111/MODEL/";

/// `<definitions>` plus `child_levels` nested `<a>` elements, so the deepest
/// element sits at depth `child_levels + 1`.
fn nested(child_levels: usize) -> String {
    let mut xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><definitions xmlns="{DMN_NS}">"#
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

fn is_nesting_rejection(error: &DmnConverterError) -> bool {
    matches!(error, DmnConverterError::InvalidXml(message) if message.contains("nesting"))
}

#[test]
fn depth_at_the_cap_is_not_rejected_for_nesting() {
    // Exactly at the cap: the guard must not fire. The converter may still
    // reject the document for structure (`<a>` is not a DMN element) -- that is
    // a different error, and it proves parsing was reached without crashing.
    let xml = nested(CAP - 1);
    if let Err(error) = parse_dmn_definition(&xml) {
        assert!(
            !is_nesting_rejection(&error),
            "depth {CAP} is at the cap and must not be rejected as too deep, got: {error:?}"
        );
    }
}

#[test]
fn depth_one_past_the_cap_is_rejected_for_nesting() {
    let error = parse_dmn_definition(&nested(CAP)).expect_err("depth CAP+1 must be rejected");
    assert!(
        is_nesting_rejection(&error),
        "expected a nesting rejection, got: {error:?}"
    );
}

#[test]
fn pathologically_deep_document_is_rejected_without_crashing() {
    // The regression that motivates the pre-parse design: this depth overflows
    // roxmltree's parser in a debug build. If the guard ran post-parse, this
    // test would abort the whole test process instead of failing.
    let error =
        parse_dmn_definition(&nested(5_000)).expect_err("a 5000-deep document must be rejected");
    assert!(
        is_nesting_rejection(&error),
        "expected a nesting rejection, got: {error:?}"
    );
}

#[test]
fn shallow_document_is_not_rejected_for_nesting() {
    if let Err(error) = parse_dmn_definition(&nested(3)) {
        assert!(
            !is_nesting_rejection(&error),
            "a 4-level document must not be rejected as too deep, got: {error:?}"
        );
    }
}

#[test]
fn many_siblings_do_not_accumulate_into_depth() {
    // Depth is nesting, not element count: siblings must not trip the cap.
    let mut xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><definitions xmlns="{DMN_NS}">"#
    );
    for _ in 0..(CAP * 20) {
        xml.push_str("<a/>");
    }
    xml.push_str("</definitions>");

    if let Err(error) = parse_dmn_definition(&xml) {
        assert!(
            !is_nesting_rejection(&error),
            "siblings must not count toward depth, got: {error:?}"
        );
    }
}
