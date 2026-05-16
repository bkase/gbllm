#![cfg(feature = "s3")]

use gbf_foundation::sha256;
use gbf_train::export_visitor::{
    EXPORT_VISITOR_ID, EXPORT_VISITOR_VERSION_HASH, EXPORT_VISITOR_VERSION_PREIMAGE, ExportVisitor,
};

#[test]
fn export_visitor_hash_is_pinned() {
    let visitor = ExportVisitor::pinned();

    assert_eq!(
        visitor.id().as_str(),
        "gbf-train.export_visitor.s3.reference_bundle.v1"
    );
    assert_eq!(EXPORT_VISITOR_ID, visitor.id().as_str());
    assert_eq!(
        visitor.version_hash(),
        EXPORT_VISITOR_VERSION_HASH,
        "pinned export visitor must expose the exported constant"
    );
    assert_eq!(
        sha256(EXPORT_VISITOR_VERSION_PREIMAGE),
        EXPORT_VISITOR_VERSION_HASH,
        "export visitor version hash must bind the stable source preimage"
    );
    assert_eq!(
        visitor.version_hash().to_string(),
        "sha256:ad94a0f4ef8a3d175955b866ce91eefe2c59e32152862371935fb491ef52c7d4"
    );
}
