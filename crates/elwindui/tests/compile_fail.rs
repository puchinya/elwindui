//! Issue #166 compile-fail fixtures (`tests/ui/*.rs`), checked against their exact rustc output.

#[test]
fn ancestor_state_is_not_inherited_by_a_derived_view() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/inherited_state_not_inherited.rs");
}

#[test]
fn explicit_base_hook_over_a_generated_base_is_rejected() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/explicit_base_hook_over_generated_base.rs");
}

#[test]
fn class_backed_components_without_a_public_constructor_reject_new() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/class_backed_unconstructible.rs");
}

#[test]
fn class_backed_construct_misuse_is_rejected() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/class_backed_construct_rules.rs");
}

#[test]
fn class_backed_and_declarative_shapes_are_never_mixed() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/class_backed_shape_mixing.rs");
}
