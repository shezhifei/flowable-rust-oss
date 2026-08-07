use flowable_cmmn_engine::{CaseFileGraph, CmmnCaseFileItem, CmmnCaseFileItemState};

#[test]
fn graph_supports_nested_instances_and_recursive_removal() {
    let mut items = Vec::new();
    let mut graph = CaseFileGraph::new(&mut items).unwrap();
    graph
        .insert(CmmnCaseFileItem::new("folder-1", "Folder").with_definition_ref("folder"))
        .unwrap();
    graph
        .insert(
            CmmnCaseFileItem::new("doc-1", "Document")
                .with_definition_ref("document")
                .with_parent("folder-1"),
        )
        .unwrap();
    graph
        .insert(
            CmmnCaseFileItem::new("page-1", "Page")
                .with_definition_ref("page")
                .with_parent("doc-1"),
        )
        .unwrap();

    assert_eq!(graph.children("folder-1").len(), 1);
    assert_eq!(graph.descendants("folder-1").len(), 2);
    assert_eq!(graph.get("page-1").unwrap().path, "/folder-1/doc-1/page-1");
    assert_eq!(
        graph.ancestry_definition_refs("page-1"),
        vec!["page", "document", "folder"]
    );

    assert_eq!(
        graph.remove_subtree("doc-1").unwrap(),
        vec!["doc-1", "page-1"]
    );
    assert_eq!(
        graph.get("page-1").unwrap().state,
        CmmnCaseFileItemState::Removed
    );
}

#[test]
fn graph_allows_multiple_instances_of_the_same_definition_under_one_parent() {
    let mut items = Vec::new();
    let mut graph = CaseFileGraph::new(&mut items).unwrap();
    graph
        .insert(CmmnCaseFileItem::new("folder", "Folder"))
        .unwrap();
    graph
        .insert(
            CmmnCaseFileItem::new("doc-1", "Document 1")
                .with_definition_ref("document")
                .with_parent("folder"),
        )
        .unwrap();
    graph
        .insert(
            CmmnCaseFileItem::new("doc-2", "Document 2")
                .with_definition_ref("document")
                .with_parent("folder"),
        )
        .unwrap();
    assert_eq!(graph.children("folder").len(), 2);
}
