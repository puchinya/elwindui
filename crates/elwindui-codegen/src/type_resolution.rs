//! Shared, deliberately lightweight type resolution for DSL expression paths.
//!
//! This module resolves only the static shape that the DSL already exposes through
//! [`ViewExpr`], [`SymbolTable`], and [`TypeInfo`]. It is not a Rust type checker: arbitrary Rust
//! expressions, methods, traits, associated types, closures, and generic inference remain outside
//! this boundary. Both lowering and validation use these facts so collection identity cannot drift
//! between the two consumers.

use crate::ast::{FieldKind, Module, ViewExpr};
use crate::codegen::{SymbolTable, TypeInfo};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CollectionIdentity {
    RcIdentity,
    Rebuild,
}

/// A concrete DSL type name carried by a lexical renderer parameter or a resolved field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedTypeRef {
    pub(crate) declared_type: String,
}

impl ResolvedTypeRef {
    pub(crate) fn new(declared_type: impl Into<String>) -> Self {
        Self {
            declared_type: declared_type.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExprTypeOrigin {
    OwnField,
    LexicalParameter,
    TemplateParentField,
    ResolvedField,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedExprType {
    pub(crate) declared_type: String,
    pub(crate) owner_type: Option<ResolvedTypeRef>,
    pub(crate) origin: ExprTypeOrigin,
    pub(crate) generated_viewmodel_observable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedCollection {
    pub(crate) expression: ResolvedExprType,
    pub(crate) effective_type: String,
    pub(crate) item_type: Option<ResolvedTypeRef>,
    pub(crate) identity: CollectionIdentity,
}

impl ResolvedCollection {
    pub(crate) fn collection_item_type(&self) -> Option<&ResolvedTypeRef> {
        self.item_type.as_ref()
    }

    pub(crate) fn collection_identity(&self) -> CollectionIdentity {
        self.identity
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ResolutionFailure {
    NotStaticPath,
    UnsupportedPath(String),
    UnknownOwnField(String),
    UnknownOwner(String),
    UnknownOwnerType(String),
    UnknownField { owner: String, field: String },
}

/// Names and types visible to a shared DSL expression resolver.
pub(crate) struct ResolutionContext<'a> {
    pub(crate) from: &'a Module,
    pub(crate) table: &'a SymbolTable,
    pub(crate) own_fields: &'a HashMap<String, String>,
    pub(crate) lexical_name: Option<&'a str>,
    pub(crate) lexical_type: Option<&'a ResolvedTypeRef>,
    pub(crate) template_parent_alias: Option<&'a str>,
    pub(crate) template_parent_type: Option<&'a ResolvedTypeRef>,
}

pub(crate) fn resolve_view_expr_type(
    expr: &ViewExpr,
    context: &ResolutionContext<'_>,
) -> Result<ResolvedExprType, ResolutionFailure> {
    let ViewExpr::Path(path) = expr else {
        return Err(ResolutionFailure::NotStaticPath);
    };
    match path.as_slice() {
        [field] => {
            if let Some(declared_type) = context.own_fields.get(field) {
                return Ok(ResolvedExprType {
                    declared_type: declared_type.clone(),
                    owner_type: None,
                    origin: ExprTypeOrigin::OwnField,
                    generated_viewmodel_observable: false,
                });
            }
            if context.lexical_name == Some(field.as_str()) {
                if let Some(lexical_type) = context.lexical_type {
                    return Ok(ResolvedExprType {
                        declared_type: lexical_type.declared_type.clone(),
                        owner_type: None,
                        origin: ExprTypeOrigin::LexicalParameter,
                        generated_viewmodel_observable: false,
                    });
                }
            }
            Err(ResolutionFailure::UnknownOwnField(field.clone()))
        }
        [owner, field] => {
            let (owner_type, origin) = if context.lexical_name == Some(owner.as_str()) {
                let owner_type = context
                    .lexical_type
                    .cloned()
                    .ok_or_else(|| ResolutionFailure::UnknownOwner(owner.clone()))?;
                (owner_type, ExprTypeOrigin::ResolvedField)
            } else if context.template_parent_alias == Some(owner.as_str()) {
                let owner_type = context
                    .template_parent_type
                    .cloned()
                    .ok_or_else(|| ResolutionFailure::UnknownOwner(owner.clone()))?;
                (owner_type, ExprTypeOrigin::TemplateParentField)
            } else if let Some(declared_type) = context.own_fields.get(owner) {
                (
                    ResolvedTypeRef::new(strip_rc_wrapper(declared_type)),
                    ExprTypeOrigin::ResolvedField,
                )
            } else {
                return Err(ResolutionFailure::UnknownOwner(owner.clone()));
            };

            resolve_field_type(&owner_type, field, origin, context)
        }
        _ => Err(ResolutionFailure::UnsupportedPath(path.join("."))),
    }
}

pub(crate) fn resolve_field_type(
    owner_type: &ResolvedTypeRef,
    field: &str,
    origin: ExprTypeOrigin,
    context: &ResolutionContext<'_>,
) -> Result<ResolvedExprType, ResolutionFailure> {
    let owner_info = resolve_type_ref_info(owner_type, context.from, context.table)
        .ok_or_else(|| ResolutionFailure::UnknownOwnerType(owner_type.declared_type.clone()))?;
    let declared_type = owner_info
        .value_field_types
        .get(field)
        .or_else(|| owner_info.field_types.get(field))
        .ok_or_else(|| ResolutionFailure::UnknownField {
            owner: owner_type.declared_type.clone(),
            field: field.to_string(),
        })?;
    let generated_viewmodel_observable = owner_info.is_viewmodel
        && matches!(owner_info.fields.get(field), Some(FieldKind::Observable))
        && nested_vec_item_type(declared_type, context.from, context.table).is_some();
    Ok(ResolvedExprType {
        declared_type: declared_type.clone(),
        owner_type: Some(owner_type.clone()),
        origin,
        generated_viewmodel_observable,
    })
}

pub(crate) fn resolve_collection(
    expr: &ViewExpr,
    context: &ResolutionContext<'_>,
) -> Result<ResolvedCollection, ResolutionFailure> {
    let expression = resolve_view_expr_type(expr, context)?;
    let item_type = explicit_rc_vec_item_type(&expression.declared_type)
        .or_else(|| vec_item_type(&expression.declared_type))
        .map(ResolvedTypeRef::new);
    let identity = if explicit_rc_vec_item_type(&expression.declared_type).is_some()
        || expression.generated_viewmodel_observable
    {
        CollectionIdentity::RcIdentity
    } else {
        CollectionIdentity::Rebuild
    };
    let effective_type = if expression.generated_viewmodel_observable {
        item_type.as_ref().map_or_else(
            || expression.declared_type.clone(),
            |item| format!("Vec<Rc<{}>>", item.declared_type),
        )
    } else {
        expression.declared_type.clone()
    };
    Ok(ResolvedCollection {
        expression,
        effective_type,
        item_type,
        identity,
    })
}

pub(crate) fn resolve_type_ref_info<'a>(
    type_ref: &ResolvedTypeRef,
    from: &Module,
    table: &'a SymbolTable,
) -> Option<&'a TypeInfo> {
    table
        .resolve(from, &type_ref.declared_type)
        .or_else(|| table.resolve_unqualified(&type_ref.declared_type))
}

/// `Vec<Model>` fields on generated viewmodels use Rc-backed item storage when the item is a DSL
/// model type. The capitalized fallback preserves the attribute frontend's isolated expansion
/// behavior, where a sibling model may not yet be in the local table; `String` is explicitly not a
/// model type. This helper is also used by viewmodel generation itself, keeping the declaration
/// and collection-resolver semantics in one place.
pub(crate) fn nested_vec_item_type(ty: &str, from: &Module, table: &SymbolTable) -> Option<String> {
    let inner = vec_item_type(ty)?;
    if explicit_rc_vec_item_type(ty).is_some() {
        return None;
    }
    let known = table.resolve(from, inner).is_some() || table.resolve_unqualified(inner).is_some();
    let looks_nested = inner.chars().next().is_some_and(|c| c.is_uppercase()) && inner != "String";
    (known || looks_nested).then(|| inner.to_string())
}

fn vec_item_type(ty: &str) -> Option<&str> {
    let ty = ty.trim();
    ["Vec<", "std::vec::Vec<"]
        .iter()
        .find_map(|prefix| ty.strip_prefix(prefix))
        .and_then(|inner| inner.strip_suffix('>'))
        .map(str::trim)
}

fn explicit_rc_vec_item_type(ty: &str) -> Option<&str> {
    let inner = vec_item_type(ty)?;
    let item = strip_rc_wrapper(inner);
    (item != inner).then_some(item)
}

fn strip_rc_wrapper(ty: &str) -> &str {
    let ty = ty.trim();
    ["std::rc::Rc<", "rc::Rc<", "Rc<"]
        .iter()
        .find_map(|prefix| ty.strip_prefix(prefix))
        .and_then(|inner| inner.strip_suffix('>'))
        .map(str::trim)
        .unwrap_or(ty)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Item, Module};
    use crate::attr_frontend::viewmodel_def_from_item_mod;
    use crate::codegen::build_symbol_table;

    fn viewmodel_item(src: &str) -> Item {
        let item_mod: syn::ItemMod = syn::parse_str(src).expect("viewmodel module should parse");
        Item::ViewModel(viewmodel_def_from_item_mod(&item_mod).expect("viewmodel should build"))
    }

    fn context<'a>(
        module: &'a Module,
        table: &'a SymbolTable,
        own_fields: &'a HashMap<String, String>,
        lexical_name: Option<&'a str>,
        lexical_type: Option<&'a ResolvedTypeRef>,
    ) -> ResolutionContext<'a> {
        ResolutionContext {
            from: module,
            table,
            own_fields,
            lexical_name,
            lexical_type,
            template_parent_alias: None,
            template_parent_type: None,
        }
    }

    #[test]
    fn resolves_generated_observable_plain_and_lexical_collections() {
        let module = Module {
            items: vec![
                viewmodel_item(
                    r#"
                    mod child_model { struct ChildModel { #[observable(default = String::new())] label: String } }
                    "#,
                ),
                viewmodel_item(
                    r#"
                    mod outer_model { struct OuterModel { #[observable(default = Vec::new())] children: Vec<ChildModel> } }
                    "#,
                ),
                viewmodel_item(
                    r#"
                    mod root_model {
                        struct RootModel {
                            #[observable(default = Vec::new())] items: Vec<OuterModel>,
                            #[observable(default = Vec::new())] names: Vec<String>,
                        }
                    }
                    "#,
                ),
            ],
            ..Default::default()
        };
        let table = build_symbol_table(std::slice::from_ref(&module));
        let own_fields = HashMap::from([(String::from("vm"), String::from("Rc<RootModel>"))]);
        let direct = context(&module, &table, &own_fields, None, None);

        let outer = resolve_collection(&ViewExpr::Path(vec!["vm".into(), "items".into()]), &direct)
            .expect("generated observable collection should resolve");
        assert_eq!(outer.item_type.unwrap().declared_type, "OuterModel");
        assert_eq!(outer.identity, CollectionIdentity::RcIdentity);
        assert_eq!(outer.effective_type, "Vec<Rc<OuterModel>>");

        let names = resolve_collection(&ViewExpr::Path(vec!["vm".into(), "names".into()]), &direct)
            .expect("plain observable collection should resolve");
        assert_eq!(names.item_type.unwrap().declared_type, "String");
        assert_eq!(names.identity, CollectionIdentity::Rebuild);

        let lexical_type = ResolvedTypeRef::new("OuterModel");
        let empty_fields = HashMap::new();
        let lexical = context(
            &module,
            &table,
            &empty_fields,
            Some("outer_item"),
            Some(&lexical_type),
        );
        let nested = resolve_collection(
            &ViewExpr::Path(vec!["outer_item".into(), "children".into()]),
            &lexical,
        )
        .expect("lexical renderer parameter field should resolve");
        assert_eq!(nested.item_type.unwrap().declared_type, "ChildModel");
        assert_eq!(nested.identity, CollectionIdentity::RcIdentity);
    }

    #[test]
    fn unresolved_collection_is_conservatively_rebuild_or_fails_static_resolution() {
        let module = Module::default();
        let table = build_symbol_table(std::slice::from_ref(&module));
        let own_fields = HashMap::from([(String::from("items"), String::from("Vec<Unknown>"))]);
        let context = context(&module, &table, &own_fields, None, None);
        let unresolved = resolve_collection(&ViewExpr::Path(vec!["items".into()]), &context)
            .expect("declared Vec shape is still statically known");
        assert_eq!(unresolved.item_type.unwrap().declared_type, "Unknown");
        assert_eq!(unresolved.identity, CollectionIdentity::Rebuild);
        assert!(matches!(
            resolve_collection(
                &ViewExpr::Path(vec!["unknown".into(), "items".into()]),
                &context,
            ),
            Err(ResolutionFailure::UnknownOwner(_))
        ));
    }
}
