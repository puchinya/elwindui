//! Issue #166 compile-fail fixtures (`tests/ui/*.rs`), checked against their exact rustc output.

#[test]
fn ancestor_state_is_not_inherited_by_a_derived_view() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/inherited_state_not_inherited.rs");
}
