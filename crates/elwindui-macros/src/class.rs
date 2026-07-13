//! `#[elwindui_macros::class]` — automates the H.2.1a class-hierarchy convention
//! (docs/elwindui_spec.md 付録H.2.1a): `trait Class: SuperClass` + `struct ClassImpl { base:
//! SuperClassImpl, .. }`, with `ClassImpl` implementing `Class` plus every ancestor trait,
//! delegating ancestor methods to `self.base`.
//!
//! Applied to a bare `struct ClassName { .. }` (no `Impl` suffix, no `base` field written by
//! hand) and, separately, a bare `impl ClassName { .. }` (no `for`) — two independent attribute
//! invocations rather than one `mod`-wrapped pair. `inherits`/`struct_only`/`supertrait`/
//! `abstract_class`/`sealed`/`trait_only` are declared **once**, on the `struct`, even though
//! several of them (`struct_only`/`supertrait`/`abstract_class`) are only ever consumed while
//! expanding the `impl` —
//! the struct's own expansion (`store_class_args`) stashes a snapshot in a process-global map keyed
//! by class name, and the paired `impl ClassName { .. }`, written as a bare `#[class]`, reads it
//! back (`load_class_args`) instead of repeating the args. This makes `struct` before `impl`
//! textual order a real requirement now (not just a readability convention): the struct's attribute
//! must expand first so the map entry exists when the impl's attribute looks it up — true within a
//! single crate's compilation, where outer attribute macros on top-level items expand in source
//! order in one process. An impl may still pass args explicitly (the pre-existing form) instead of
//! relying on the store; explicit args always win.
//!
//! Ancestor delegation is **not** implemented as a fully generic cross-crate manifest/token-
//! accumulator (the `ambassador` crate's technique) — it doesn't need to be, because every
//! `inherits = ..` target's `base` field type genuinely implements `UIElement` itself
//! (`UIElementImpl` included — its own trivial, identity-function `impl UIElement for
//! UIElementImpl` is synthesized by this same macro's root-class-mode handling in `expand_impl`,
//! not hand-written; `LayoutImpl`'s is the ordinary `uielement_blind_forward` case like any other
//! non-root class), so a single blind `self.base.method(..)` forward (`uielement_blind_forward`) is
//! always correct; the macro never needs to know *which* ancestor it is.
//!
//! That blind forward only covers `UIElement` itself, though — an intermediate `inherits = ..`
//! trait with *its own* required methods beyond `UIElement`'s (`Shape::set_kind`/
//! `Control::set_padding`, unlike the genuinely empty `Layout`/`NativeControl<H>` marker traits)
//! has no generic name to blindly forward through. `#[ancestor]` on an `&self` method inside
//! `impl ClassName { .. }` opts it into that trait's own impl block instead of `UIElement`'s or
//! `ClassName`'s own (mirroring `#[inherent]`'s same attribute-driven routing, just to a different
//! bucket) — omitted entirely, the ancestor impl stays the empty `{}` block that's already correct
//! for a true marker trait.
//!
//! `inherits` omitted entirely puts a `struct`/`impl` pair into **root class mode** instead of
//! declaring an ordinary subclass — used for the one class with no ancestor of its own
//! (`UIElement`). There, `impl ClassName { .. }` isn't paired with a generated
//! `impl ClassName for ClassNameImpl` at all (that struct is never meant to implement the trait
//! itself — its descendants embed it as `base` and implement the trait *themselves*, the same way
//! every other `#[class]`-managed subclass already does): every method the user writes is instead
//! embedded, body and all, directly into the generated `pub trait ClassName { .. }` as a *default*
//! method, inherited for free by every descendant via Rust's own default-method dispatch. The one
//! exception is `base` itself, whose concrete location differs per implementor and so can't have a
//! shared default — the macro synthesizes its (required, body-less) signature itself, and errors
//! if the user tries to define it by hand. `supertrait = ..` adds any extra bound this root class
//! needs (e.g. `UIElement: AsAny`) — a different concept from `inherits`, which additionally drives
//! `base`-field insertion that a root class (having no ancestor) never wants.
//!
//! Field-driven `Cell`/`RefCell` accessor generation is likewise not implemented — those getters/
//! setters stay hand-written in `impl ClassName { .. }`, exactly as today; they are typically one
//! line each and the real win here is eliminating the `ClassNameImpl`/trait declaration and the
//! ancestor delegation boilerplate.

use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use syn::{
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
    Field, Fields, FnArg, Ident, ImplItem, ImplItemFn, Item, Path, Token, Type, Visibility,
};

/// Class names that `.elwind`'s `#[sealed]` already marks non-inheritable (docs/elwindui_spec.md
/// `builtins.elwind`) — hardcoded rather than derived from a cross-invocation manifest, matching
/// this module's overall simplification (see its own doc comment).
const SEALED_CLASSES: &[&str] = &["TextArea", "Button"];

/// The fixed method set every `UIElement` implementor may define — used to route a method written
/// inside `impl ClassName { .. }` to the generated `impl elwindui_core::ui::UIElement for
/// ClassNameImpl` block instead of to `ClassName`'s own trait impl.
const UI_ELEMENT_METHODS: &[&str] =
    &["as_ui_element", "visual_children", "measure_override", "arrange_override", "paint", "try_as_native_control"];

/// Parsed `#[class(inherits = .., struct_only = .., supertrait = .., abstract_class, sealed)]`
/// arguments — every key is optional and any subset/order is accepted.
#[derive(Default)]
struct ClassArgs {
    inherits: Option<Type>,
    /// Declares `ClassName` a pure struct implementor of an *existing* trait — no new `pub trait
    /// ClassName` is generated; the given path is implemented directly for `ClassNameImpl` instead.
    /// The `trait_only`/`struct_only` pair mirrors a trait definition and its implementation living
    /// in separate files/crates: `trait_only` declares the trait (no backing struct), `struct_only`
    /// declares a concrete struct implementing a trait declared elsewhere (no new trait of its own).
    struct_only: Option<Path>,
    /// A supertrait bound unrelated to the H.2.1a "class hierarchy" concept `inherits` models
    /// (no `base` field is inserted for it) — e.g. `UIElement: AsAny`. Only meaningful in "root
    /// class mode" (`inherits` omitted): see `expand_impl`'s own doc comment.
    supertrait: Option<Path>,
    abstract_class: bool,
    sealed: bool,
    /// Declares `ClassName` a pure trait — no `ClassNameImpl` struct, no `base` field, no `impl
    /// ClassName for ClassNameImpl` at all. The paired `struct ClassName { .. }` body must be empty
    /// (there's nowhere for fields to go); no paired `impl ClassName { .. }` is needed either (there
    /// are no methods to attach anywhere). Used for a marker ancestor trait with no real backing
    /// implementation of its own in this crate (e.g. `elwindui_core::ui::NativeControl` — each
    /// backend provides its own concrete implementor instead). See `NativeControl`'s own doc comment.
    trait_only: bool,
}

impl Parse for ClassArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut args = ClassArgs::default();
        let items = Punctuated::<ClassArg, Token![,]>::parse_terminated(input)?;
        for item in items {
            match item {
                ClassArg::Inherits(ty) => args.inherits = Some(ty),
                ClassArg::StructOnly(path) => args.struct_only = Some(path),
                ClassArg::Supertrait(path) => args.supertrait = Some(path),
                ClassArg::AbstractClass => args.abstract_class = true,
                ClassArg::Sealed => args.sealed = true,
                ClassArg::TraitOnly => args.trait_only = true,
            }
        }
        Ok(args)
    }
}

enum ClassArg {
    Inherits(Type),
    StructOnly(Path),
    Supertrait(Path),
    AbstractClass,
    Sealed,
    TraitOnly,
}

impl Parse for ClassArg {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let ident: Ident = input.parse()?;
        match ident.to_string().as_str() {
            "inherits" => {
                input.parse::<Token![=]>()?;
                Ok(ClassArg::Inherits(input.parse()?))
            }
            "struct_only" => {
                input.parse::<Token![=]>()?;
                Ok(ClassArg::StructOnly(input.parse()?))
            }
            "supertrait" => {
                input.parse::<Token![=]>()?;
                Ok(ClassArg::Supertrait(input.parse()?))
            }
            "abstract_class" => Ok(ClassArg::AbstractClass),
            "sealed" => Ok(ClassArg::Sealed),
            "trait_only" => Ok(ClassArg::TraitOnly),
            other => Err(syn::Error::new(ident.span(), format!("#[class]: unknown argument `{other}`"))),
        }
    }
}

/// `ClassArgs`, but with `Type`/`Path` fields flattened to their token-string form so a snapshot can
/// be held in a process-global store between the `struct ClassName`  and `impl ClassName` attribute
/// invocations (proc-macro types aren't worth threading `Send`/`Sync` bounds through for this).
struct StoredClassArgs {
    inherits: Option<String>,
    struct_only: Option<String>,
    supertrait: Option<String>,
    abstract_class: bool,
    sealed: bool,
    trait_only: bool,
}

/// Keyed by bare class name (e.g. `"VerticalLayout"`) — populated by `struct ClassName`'s own
/// `#[class(..)]` invocation, read by the paired `impl ClassName { .. }`'s bare `#[class]` (see
/// `expand`/`expand_impl`). Relies on the struct's attribute expanding before its paired impl's
/// within the same crate compilation — see this module's own doc comment.
///
/// The bare class name isn't unique crate-wide (e.g. `TabViewImpl` names both
/// `elwindui-backend-appkit`'s top-level native facade and its unrelated
/// `builtins::tab_view::TabViewImpl`) — `load_class_args` therefore *removes* the entry it reads
/// (see its own doc comment) rather than merely reading it, so each struct/impl pair only ever
/// collides with a same-named pair that's still "open" (pushed but not yet consumed), which the
/// struct-immediately-followed-by-its-own-impl convention this whole macro depends on never
/// produces.
fn class_arg_store() -> &'static Mutex<HashMap<String, StoredClassArgs>> {
    static STORE: OnceLock<Mutex<HashMap<String, StoredClassArgs>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn store_class_args(class_name: &str, args: &ClassArgs) {
    let stored = StoredClassArgs {
        inherits: args.inherits.as_ref().map(|t| quote! { #t }.to_string()),
        struct_only: args.struct_only.as_ref().map(|p| quote! { #p }.to_string()),
        supertrait: args.supertrait.as_ref().map(|p| quote! { #p }.to_string()),
        abstract_class: args.abstract_class,
        sealed: args.sealed,
        trait_only: args.trait_only,
    };
    class_arg_store().lock().unwrap().insert(class_name.to_string(), stored);
}

/// `None` means no `struct ClassName` has been seen yet under this name — the caller (`expand_impl`)
/// turns that into a `compile_error!` pointing at the missing/misordered struct declaration. Removes
/// the entry on success (see `class_arg_store`'s own doc comment for why this must consume, not just
/// read, the stored snapshot).
fn load_class_args(class_name: &str) -> Option<ClassArgs> {
    let mut store = class_arg_store().lock().unwrap();
    let stored = store.remove(class_name)?;
    let parse_type = |s: &String| syn::parse_str::<Type>(s).expect("#[class]: internal: failed to reparse stored `inherits` type");
    let parse_path = |s: &String| syn::parse_str::<Path>(s).expect("#[class]: internal: failed to reparse stored path");
    Some(ClassArgs {
        inherits: stored.inherits.as_ref().map(parse_type),
        struct_only: stored.struct_only.as_ref().map(parse_path),
        supertrait: stored.supertrait.as_ref().map(parse_path),
        abstract_class: stored.abstract_class,
        sealed: stored.sealed,
        trait_only: stored.trait_only,
    })
}

/// Permanent (never-removed — unlike `class_arg_store`, which is consumed on read) map from a bare,
/// non-`Impl`-suffixed class name to its own `inherits` type token-string (`None` for root mode).
/// Populated by every `struct ClassName { .. }` expansion (see `register_ancestor`) and walked by
/// `expand_impl` (see its own doc comment) to generate `as_<snake(ancestor)>()` accessors reaching
/// beyond a class's immediate parent.
fn ancestor_registry() -> &'static Mutex<HashMap<String, Option<String>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The registration key is `Impl`-stripped exactly like `bare_class_name` strips a *lookup* key from
/// a type reference (same rule, applied to the plain written identifier instead of a `Type`) — an
/// explicit-facade class's own written name (e.g. `NativeTabViewImpl`) is how *other* classes'
/// `inherits = ..` references spell it, so the two must normalize the same way or a lookup for it
/// can never hit.
///
/// First-write-wins (`Entry::or_insert`, never overwrites an existing key) rather than always
/// overwriting: this stripping means an explicit-facade class's key (e.g.
/// `elwindui-backend-appkit::builtins::TextAreaImpl` stripping to `"TextArea"`) can collide with a
/// *different*, unrelated class's own genuine bare name (`appkit::TextArea`'s own struct) — but the
/// genuine bare-named class is always declared, and so always registers, first (native leaf structs
/// live directly in a backend crate's `lib.rs`; `mod builtins;`, which wraps them, is declared
/// *after* every one of them specifically so this holds — see `elwindui-backend-appkit`'s own
/// `lib.rs`). First-write-wins means the later, colliding registration is silently ignored instead
/// of corrupting the correct entry.
fn register_ancestor(class_name: &str, inherits: &Option<Type>) {
    let key = class_name.strip_suffix("Impl").unwrap_or(class_name).to_string();
    let value = inherits.as_ref().map(|t| quote! { #t }.to_string());
    ancestor_registry().lock().unwrap().entry(key).or_insert(value);
}

/// `Some(None)` = `bare_name` is a registered local root (root mode, no further ancestor). `None` =
/// not found — either a genuinely external/cross-crate type, or not yet registered (this class's
/// struct hasn't expanded yet — see `register_ancestor`'s own doc comment on why declaration order
/// matters). Both cases tell `expand_impl` to stop walking the chain there.
fn lookup_ancestor(bare_name: &str) -> Option<Option<Type>> {
    let registry = ancestor_registry().lock().unwrap();
    let entry = registry.get(bare_name)?;
    Some(entry.as_ref().map(|s| syn::parse_str::<Type>(s).expect("#[class]: internal: failed to reparse stored ancestor type")))
}

/// `elwindui_core::ui::NativeControl<AnyView>` -> `"NativeControl"`; `appkit::TextAreaImpl` (already
/// `Impl`-suffixed) -> `"TextArea"` — the bare, un-suffixed display name used both as an
/// `ancestor_registry` lookup key and to derive an `as_<snake(name)>()` accessor's method name.
fn bare_class_name(ty: &Type) -> Option<String> {
    let Type::Path(tp) = ty else { return None };
    let seg = tp.path.segments.last()?;
    let name = seg.ident.to_string();
    Some(name.strip_suffix("Impl").map(str::to_string).unwrap_or(name))
}

/// Like `bare_class_name`, but *without* stripping a trailing `Impl` — used where the distinction
/// itself matters (an already-`Impl`-suffixed reference is an "explicit facade type", pure struct
/// composition with no separate ancestor trait to `impl`; a bare name is a real trait to implement).
fn raw_last_segment_name(ty: &Type) -> Option<String> {
    let Type::Path(tp) = ty else { return None };
    Some(tp.path.segments.last()?.ident.to_string())
}

/// `VerticalLayout` -> `"vertical_layout"`; `UIElement` -> `"ui_element"` (a run of uppercase letters
/// is treated as one unit — the underscore goes before the *last* uppercase letter in the run when
/// it's followed by a lowercase letter, not before every uppercase letter).
fn to_snake_case(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::new();
    for (i, &ch) in chars.iter().enumerate() {
        if ch.is_uppercase() {
            let prev_is_lower = i > 0 && chars[i - 1].is_lowercase();
            let prev_is_upper_and_next_is_lower = i > 0 && chars[i - 1].is_uppercase() && i + 1 < chars.len() && chars[i + 1].is_lowercase();
            if i > 0 && (prev_is_lower || prev_is_upper_and_next_is_lower) {
                out.push('_');
            }
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// `NativeControl` -> `NativeControlImpl`; `crate::TextAreaImpl` (already `Impl`-suffixed — the
/// "explicit facade type" form, docs/elwindui_spec.md 付録H.2.1a's discussion of the backend
/// DSL-facing wrapper layer) -> used as-is.
fn base_impl_type(ty: &Type) -> Type {
    let mut ty = ty.clone();
    if let Type::Path(tp) = &mut ty {
        if let Some(seg) = tp.path.segments.last_mut() {
            let name = seg.ident.to_string();
            if !name.ends_with("Impl") {
                seg.ident = Ident::new(&format!("{name}Impl"), seg.ident.span());
            }
        }
    }
    ty
}

/// `VerticalLayout` -> `VerticalLayoutImpl`; `TabViewImpl` (already `Impl`-suffixed — needed when
/// the bare class name is already taken by a hand-written trait declaration in the same module,
/// e.g. appkit's local `TabView` trait, so the `#[class]` struct/impl pair is itself named
/// `TabViewImpl` directly) -> used as-is, mirroring `base_impl_type`'s same heuristic.
fn to_impl_name(class_name: &Ident) -> Ident {
    let name = class_name.to_string();
    if name.ends_with("Impl") { class_name.clone() } else { format_ident!("{name}Impl") }
}

pub fn expand(attr: TokenStream2, item: TokenStream2) -> TokenStream2 {
    // Checked before parsing: a bare `#[class]` on an `impl` block means "reuse the args already
    // declared on the paired `struct`" (see `expand_impl`) — `ClassArgs::default()` from parsing an
    // empty token stream would be indistinguishable from that intent otherwise.
    let attr_is_empty = attr.is_empty();
    let args = match syn::parse2::<ClassArgs>(attr) {
        Ok(args) => args,
        Err(e) => return e.to_compile_error(),
    };
    let parsed_item = match syn::parse2::<Item>(item) {
        Ok(item) => item,
        Err(e) => return e.to_compile_error(),
    };
    match parsed_item {
        Item::Struct(item_struct) => {
            store_class_args(&item_struct.ident.to_string(), &args);
            register_ancestor(&item_struct.ident.to_string(), &args.inherits);
            expand_struct(&args, item_struct)
        }
        Item::Impl(item_impl) => expand_impl(args, item_impl, attr_is_empty),
        other => {
            let msg = "#[class]: expected a `struct ClassName { .. }` or `impl ClassName { .. }` item";
            let mut ts = syn::Error::new_spanned(&other, msg).to_compile_error();
            ts.extend(quote! { #other });
            ts
        }
    }
}

/// `elwindui_core::ui::X` from every other crate, but `crate::ui::X` when `#[class]` is itself
/// expanding *inside* `elwindui-core` (its own `ui.rs` uses this macro too, e.g. for
/// `NativeControl<H>`) — a crate cannot refer to itself by its external `extern crate` name.
/// `CARGO_PKG_NAME` is set by cargo to whichever crate is currently being compiled, i.e. exactly
/// the crate this macro invocation is expanding within.
fn core_path() -> TokenStream2 {
    match std::env::var("CARGO_PKG_NAME").as_deref() {
        Ok("elwindui-core") => quote! { crate },
        _ => quote! { elwindui_core },
    }
}

fn expand_struct(args: &ClassArgs, item: syn::ItemStruct) -> TokenStream2 {
    let class_name = &item.ident;
    let impl_name = to_impl_name(class_name);
    let vis = &item.vis;
    let attrs = &item.attrs;
    let generics = &item.generics;

    let existing_fields: Vec<Field> = match &item.fields {
        Fields::Named(named) => named.named.iter().cloned().collect(),
        Fields::Unit => Vec::new(),
        Fields::Unnamed(_) => {
            return syn::Error::new_spanned(&item, "#[class]: struct must use named fields, not a tuple struct")
                .to_compile_error();
        }
    };

    if args.trait_only {
        if !existing_fields.is_empty() {
            let msg = "#[class]: `trait_only` classes have no backing struct — `struct ClassName { .. }` must be \
                       empty (there's nowhere for fields to go)";
            return syn::Error::new_spanned(&item, msg).to_compile_error();
        }
        // No `ClassNameImpl`, no `base` field, no `impl ClassName for ClassNameImpl` — just the bare
        // trait declaration. A concrete implementor (each backend's own `NativeControlImpl`, for
        // `NativeControl`) provides everything else. No paired `impl ClassName { .. }` is expected —
        // there are no methods to attach anywhere in `trait_only` mode.
        let bound = args.inherits.as_ref().map(|t| quote! { : #t });
        return quote! {
            #(#attrs)*
            #vis trait #class_name #generics #bound {}
        };
    }

    // `pub` (matching H.2.1a's convention): plenty of call sites reach through `.base` across
    // module/crate boundaries (`self.base.handle`, `self.base.as_ui_element()`, ...).
    let base_field = args.inherits.as_ref().map(|ty| {
        let base_ty = base_impl_type(ty);
        quote! { pub base: #base_ty, }
    });

    quote! {
        #(#attrs)*
        #vis struct #impl_name #generics {
            #base_field
            #(#existing_fields,)*
        }
    }
}

fn expand_impl(attr_args: ClassArgs, item: syn::ItemImpl, attr_is_empty: bool) -> TokenStream2 {
    if item.trait_.is_some() {
        return syn::Error::new_spanned(
            &item,
            "#[class]: write `impl ClassName { .. }` (no `for`) — the macro routes each method to the right \
             generated `impl .. for ClassNameImpl` block itself",
        )
        .to_compile_error();
    }
    let class_name = match &*item.self_ty {
        Type::Path(tp) => match tp.path.segments.last() {
            Some(seg) => seg.ident.clone(),
            None => return syn::Error::new_spanned(&item.self_ty, "#[class]: empty type path").to_compile_error(),
        },
        other => return syn::Error::new_spanned(other, "#[class]: unsupported `impl` self type").to_compile_error(),
    };
    // A bare `#[class]` on the impl means "use whatever `#[class(..)]` args the paired `struct
    // ClassName` declared" (see `store_class_args`/`load_class_args`) — explicit args on the impl
    // (the old, still-supported form) always win over anything stored.
    let args = if attr_is_empty {
        match load_class_args(&class_name.to_string()) {
            Some(args) => args,
            None => {
                let msg = format!(
                    "#[class]: no matching `struct {class_name}` with #[elwindui_macros::class(..)] found earlier in \
                     this file — declare the struct (with any inherits/struct_only/supertrait/... args) before this \
                     impl block, or pass args explicitly here"
                );
                return syn::Error::new_spanned(&item.self_ty, msg).to_compile_error();
            }
        }
    } else {
        attr_args
    };
    let args = &args;
    let impl_name = to_impl_name(&class_name);
    // `<H>`/`<H: 'static>`/where-clause, threaded through every generated block below so a generic
    // class (e.g. `NativeControl<H>`) works the same as a non-generic one.
    let (impl_generics, ty_generics, where_clause) = item.generics.split_for_impl();

    if let Some(inh) = &args.inherits {
        if let Type::Path(tp) = inh {
            if let Some(seg) = tp.path.segments.last() {
                if SEALED_CLASSES.contains(&seg.ident.to_string().as_str()) {
                    let msg = format!("class `{}` is #[sealed] and cannot be inherited", seg.ident);
                    return syn::Error::new_spanned(inh, msg).to_compile_error();
                }
            }
        }
    }

    // `as_<snake(ancestor)>()` accessors (docs/elwindui_spec.md 付録H.2.1a's `base`-chain convention,
    // exposed as named methods instead of the ancestor-skipping `base()` trait method they replace —
    // see `crates/elwindui-macros/src/class.rs`'s own module doc comment). The immediate parent
    // always gets one (`&self.base`, no registry lookup needed — works across crate boundaries since
    // it's a plain field access). Deeper ancestors are chain-walked via `ancestor_registry`, one
    // level at a time, delegating to `self.base`'s own same-named accessor (which that class's own
    // `#[class]` expansion already generated the same way) — so this reaches every ancestor declared
    // in the same crate, and exactly one level further into an external crate before the registry
    // (necessarily local-only) runs out and the walk stops there. `UIElement` itself is always
    // skipped: `as_ui_element()` is generated unconditionally elsewhere (`uielement_blind_forward`)
    // regardless of how many hops away the root actually is, so repeating it here would conflict.
    let mut accessor_methods: Vec<TokenStream2> = Vec::new();
    // Ancestor-trait `impl`s found by continuing the walk *beyond* the immediate `args.inherits`
    // target (hop 0 — handled separately, below, with `#[ancestor]`-tagged method routing for a
    // trait that has real required methods). Every further hop found via `ancestor_registry` is a
    // *bare* trait name reached transitively through however many `Impl`-suffixed "explicit facade
    // type" links lie in between (e.g. a DSL wrapper composing a hand-written native leaf that itself
    // composes a shared `NativeControlImpl`) — always given an *empty* body, since there's no
    // `#[ancestor]`-style mechanism to attach real methods to a hop this deep. Correct as long as
    // every such transitively-reached trait is a zero-method marker (`NativeControl`, `Layout`, ...);
    // a trait with real methods found this way would instead surface as a "missing trait items"
    // compile error right here, which is an acceptable, loud failure mode rather than silently wrong
    // behavior.
    let mut transitive_ancestor_impls: Vec<TokenStream2> = Vec::new();
    if let Some(parent_ty) = &args.inherits {
        if let Some(mut current_display) = bare_class_name(parent_ty) {
            let mut current_ty = parent_ty.clone();
            let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
            visited.insert(class_name.to_string());
            let mut hop_index = 0usize;
            loop {
                if current_display == "UIElement" {
                    break;
                }
                if !visited.insert(current_display.clone()) {
                    // Cycle — almost certainly a same-crate name collision the registry can't tell
                    // apart (see `register_ancestor`'s doc comment). Stop rather than loop forever or
                    // generate an accessor built from the wrong ancestor's data.
                    break;
                }
                let current_impl_ty = base_impl_type(&current_ty);
                let method_name = format_ident!("as_{}", to_snake_case(&current_display));
                let body = if accessor_methods.is_empty() {
                    quote! { &self.base }
                } else {
                    quote! { self.base.#method_name() }
                };
                accessor_methods.push(quote! {
                    pub fn #method_name(&self) -> &#current_impl_ty { #body }
                });
                if hop_index > 0 {
                    let raw_name = raw_last_segment_name(&current_ty);
                    if raw_name.as_deref().is_some_and(|n| n != "UIElement" && !n.ends_with("Impl")) {
                        transitive_ancestor_impls.push(quote! {
                            impl #current_ty for #impl_name #ty_generics { }
                        });
                    }
                }
                hop_index += 1;
                match lookup_ancestor(&current_display) {
                    Some(Some(next_ty)) => match bare_class_name(&next_ty) {
                        Some(next_display) => {
                            current_ty = next_ty;
                            current_display = next_display;
                        }
                        None => break,
                    },
                    // Local root (`inherits` omitted) or not found (external/cross-crate, or an
                    // explicit-facade class never registered) — stop; the one hop already generated
                    // above is as far as this walk goes.
                    _ => break,
                }
            }
        }
    }

    // `bool` = force this entry to `pub` regardless of what the user wrote — true constructors
    // (no `self`) always did in the hand-written original, but an `#[inherent]` `&self` helper
    // keeps whatever visibility the user actually gave it (some, like `rebuild`/
    // `sync_dynamic_entries` in the appkit `TabView` facade, are deliberately private).
    let mut ctor_methods: Vec<(ImplItemFn, bool)> = Vec::new();
    let mut instance_methods: Vec<ImplItemFn> = Vec::new();
    // `#[ancestor]`-marked methods (below) collect here instead — routed into the intermediate
    // `inherits` trait's own impl block rather than `UIElement`'s or `ClassName`'s own.
    let mut ancestor_methods: Vec<ImplItemFn> = Vec::new();
    for impl_item in item.items {
        match impl_item {
            ImplItem::Fn(mut f) => {
                // `#[inherent]` opts a `&self` method *out* of trait-impl routing entirely — for
                // helpers that aren't part of any trait (ancestor's or `ClassName`'s own), e.g. the
                // backend facade layer's `into_any_view`/`set_on_text_change`. It lands as a plain
                // `impl ClassNameImpl { .. }` method, alongside constructors.
                let is_inherent = f.attrs.iter().any(|a| a.path().is_ident("inherent"));
                // `#[ancestor]` opts a `&self` method *into* the `inherits` trait's own impl block
                // (e.g. `Shape::set_kind`/`Control::set_padding` — a real, non-`UIElement` required
                // method on the immediate ancestor trait, as opposed to `UI_ELEMENT_METHODS`, which
                // covers `UIElement` itself, or an unmarked method, which lands on `ClassName`'s own
                // trait). Needed because unlike `Layout`/`NativeControl<H>` (empty marker traits —
                // see this module's own doc comment), a trait like `Shape`/`Control` has required
                // methods of its own that a blind `self.base.method(..)` forward can't discover
                // without a name to key off; `#[ancestor]` is that name.
                let is_ancestor = f.attrs.iter().any(|a| a.path().is_ident("ancestor"));
                // `#[overridable]`/`#[overrides]` are accepted and stripped but not yet validated
                // against an ancestor's virtual-method list — see this module's own doc comment.
                f.attrs.retain(|a| {
                    !(a.path().is_ident("overridable")
                        || a.path().is_ident("overrides")
                        || a.path().is_ident("inherent")
                        || a.path().is_ident("ancestor"))
                });
                if is_inherent {
                    ctor_methods.push((f, false));
                    continue;
                }
                let has_self = matches!(f.sig.inputs.first(), Some(FnArg::Receiver(_)));
                if has_self {
                    // Every instance method lands in a trait impl (either an ancestor's or
                    // `ClassName`'s own) — trait impl items always inherit the trait's own
                    // visibility and reject an explicit qualifier (E0449).
                    f.vis = Visibility::Inherited;
                    if is_ancestor {
                        ancestor_methods.push(f);
                    } else {
                        instance_methods.push(f);
                    }
                } else {
                    ctor_methods.push((f, true));
                }
            }
            other => {
                return syn::Error::new_spanned(other, "#[class]: only `fn` items are supported inside `impl ClassName { .. }`")
                    .to_compile_error();
            }
        }
    }
    if !ancestor_methods.is_empty() && args.inherits.is_none() {
        let names: Vec<String> = ancestor_methods.iter().map(|f| f.sig.ident.to_string()).collect();
        let msg = format!("#[class]: #[ancestor] methods {names:?} require `inherits = ..` (root class mode has no ancestor trait)");
        return syn::Error::new_spanned(&item.self_ty, msg).to_compile_error();
    }

    // Root-class mode (`inherits` omitted, e.g. `UIElement` itself): there's no ancestor to route
    // anything to, so every method the user wrote is an "own" method — no `UI_ELEMENT_METHODS`
    // partitioning happens at all. See `expand_impl`'s own doc comment for the full shape.
    let is_root_mode = args.inherits.is_none();
    let (ui_element_methods, own_methods): (Vec<ImplItemFn>, Vec<ImplItemFn>) = if is_root_mode {
        (Vec::new(), instance_methods)
    } else {
        instance_methods.into_iter().partition(|f| UI_ELEMENT_METHODS.contains(&f.sig.ident.to_string().as_str()))
    };

    let core = core_path();
    let mut ancestor_impls = Vec::new();
    if let Some(inh) = &args.inherits {
        // `UIElementImpl` and `LayoutImpl` both implement `UIElement` themselves now (see
        // `expand_impl`'s own doc comment), so *every* `inherits = ..` target's `base` field type
        // genuinely implements `UIElement` — blind `self.base.method(..)` forwarding is always
        // correct, with no need to special-case which ancestor this is.
        ancestor_impls.push(uielement_blind_forward(&core, &impl_name, &impl_generics, &ty_generics, &where_clause, &ui_element_methods));
        // Any intermediate trait between `UIElement` and this class (`Layout`, `NativeControl<H>`,
        // `Shape`, `Control`, ...) also needs an `impl` of its own — skipped for `UIElement` itself
        // (already fully covered by the blind forward above) and for the "explicit facade type"
        // form (an already-`Impl`-suffixed concrete struct, not a trait, so there's nothing to
        // `impl`). Empty for a pure marker trait with no required methods (`Layout`/
        // `NativeControl<H>`); populated from `#[ancestor]`-marked methods (above) for one with real
        // required methods of its own (`Shape`/`Control`) — see `#[ancestor]`'s own doc comment.
        if let Type::Path(tp) = inh {
            if let Some(seg) = tp.path.segments.last() {
                let name = seg.ident.to_string();
                if name != "UIElement" && !name.ends_with("Impl") {
                    let bodies = ancestor_methods.iter().map(|f| quote! { #f });
                    ancestor_impls.push(quote! {
                        impl #impl_generics #inh for #impl_name #ty_generics #where_clause { #(#bodies)* }
                    });
                } else if !ancestor_methods.is_empty() {
                    let names: Vec<String> = ancestor_methods.iter().map(|f| f.sig.ident.to_string()).collect();
                    let msg = format!(
                        "#[class]: #[ancestor] methods {names:?} have nowhere to go — `inherits = {name}` has no \
                         separate ancestor trait to implement"
                    );
                    return syn::Error::new_spanned(inh, msg).to_compile_error();
                }
            }
        }
    }
    // Ancestor traits found beyond the immediate `inherits` target (see `transitive_ancestor_impls`'s
    // own doc comment above, at the accessor-walk loop that builds it).
    ancestor_impls.extend(transitive_ancestor_impls);

    let (trait_decl, trait_impl) = if let Some(existing) = &args.struct_only {
        let bodies = own_methods.iter().map(|f| quote! { #f });
        (
            TokenStream2::new(),
            quote! {
                impl #impl_generics #existing for #impl_name #ty_generics #where_clause {
                    #(#bodies)*
                }
            },
        )
    } else if is_root_mode {
        // Root class mode (`inherits` omitted, e.g. `UIElement` itself): every method the user
        // wrote becomes a *default* trait method (body embedded directly in the trait declaration,
        // shared by every future `inherits = ClassName` descendant for free via Rust's own
        // default-method dispatch) rather than a required method paired with a separate `impl
        // ClassName for ClassNameImpl` — that pairing doesn't apply here since `ClassNameImpl`
        // itself is never meant to implement `ClassName` (descendants embed it as `base` and
        // implement the trait themselves, exactly like `UIElementImpl` does today). `as_ui_element`
        // is the one method every implementor must supply itself (its concrete location differs per
        // type), so the macro synthesizes it as the sole required signature — the user must not
        // define it by hand.
        if let Some(base_fn) = own_methods.iter().find(|f| f.sig.ident == "as_ui_element") {
            let msg = "#[class]: root class's `as_ui_element` is auto-generated; do not define it";
            return syn::Error::new_spanned(&base_fn.sig, msg).to_compile_error();
        }
        let bound = args.supertrait.as_ref().map(|t| quote! { : #t });
        let default_methods = own_methods.iter().map(|f| quote! { #f });
        (
            quote! {
                pub trait #class_name #impl_generics #bound #where_clause {
                    fn as_ui_element(&self) -> &#impl_name;
                    #(#default_methods)*
                }
            },
            // `ClassNameImpl` is a genuine `ClassName` implementor itself — trivially, since
            // `as_ui_element` (or whatever the root's own required accessor is) is just the identity
            // function here. This is what lets every `#[class(inherits = ..)]`-managed subclass's
            // ancestor delegation reduce to a single uniform `self.base.method(..)` forward
            // (`uielement_blind_forward`), regardless of how many `base` hops it has to pass through
            // to reach the root.
            quote! {
                impl #impl_generics #class_name #ty_generics for #impl_name #ty_generics #where_clause {
                    fn as_ui_element(&self) -> &#impl_name { self }
                }
            },
        )
    } else if class_name.to_string().ends_with("Impl") {
        // The class's own local name is already `Impl`-suffixed (the "explicit facade type" form
        // — see `to_impl_name`'s doc comment) — there's no bare trait name to derive one from, and
        // in practice every method on this kind of facade is either an ancestor delegate or an
        // `#[inherent]` helper, so there's nothing left needing a trait home at all.
        if !own_methods.is_empty() {
            let names: Vec<String> = own_methods.iter().map(|f| f.sig.ident.to_string()).collect();
            let msg = format!(
                "#[class]: `{class_name}` has no bare class name to declare a trait under (its own name is already \
                 `Impl`-suffixed) and no `struct_only = ..` was given, but these methods aren't ancestor methods \
                 either: {names:?} — mark them `#[inherent]` if they're plain helpers"
            );
            return syn::Error::new_spanned(&item.self_ty, msg).to_compile_error();
        }
        (TokenStream2::new(), TokenStream2::new())
    } else {
        let bound = args.inherits.as_ref().map(|t| quote! { : #t });
        let sigs = own_methods.iter().map(|f| {
            let sig = &f.sig;
            quote! { #sig; }
        });
        let bodies = own_methods.iter().map(|f| quote! { #f });
        (
            quote! { pub trait #class_name #impl_generics #bound #where_clause { #(#sigs)* } },
            quote! {
                impl #impl_generics #class_name #ty_generics for #impl_name #ty_generics #where_clause {
                    #(#bodies)*
                }
            },
        )
    };

    if args.abstract_class {
        if let Some((new_fn, _)) = ctor_methods.iter().find(|(f, _)| f.sig.ident == "new") {
            let msg = format!("abstract_class `{class_name}` must not define `new`");
            return syn::Error::new_spanned(&new_fn.sig, msg).to_compile_error();
        }
    }

    let mut ctor_block = TokenStream2::new();
    if !ctor_methods.is_empty() || !accessor_methods.is_empty() {
        let fns: Vec<TokenStream2> = ctor_methods
            .iter()
            .map(|(f, force_pub)| {
                let mut f = f.clone();
                if *force_pub {
                    f.vis = Visibility::Public(Default::default());
                }
                quote! { #f }
            })
            .collect();
        ctor_block = quote! {
            impl #impl_generics #impl_name #ty_generics #where_clause {
                #(#fns)*
                #(#accessor_methods)*
            }
        };
    }

    let out = quote! {
        #trait_decl
        #trait_impl
        #(#ancestor_impls)*
        #ctor_block
    };
    if std::env::var("ELWINDUI_CLASS_DEBUG").is_ok() {
        eprintln!("=== #[class] impl {class_name} expansion ===\n{out}\n===");
    }
    out
}

/// Builds `impl elwindui_core::ui::UIElement for #impl_name #ty_generics { .. }` for the
/// [`AncestorShape::BlindDelegate`] case: each of the four methods either comes from
/// `user_methods` (an explicit override) or falls back to a hardcoded `self.base.method(..)`
/// forward.
fn uielement_blind_forward(
    core: &TokenStream2,
    impl_name: &Ident,
    impl_generics: &syn::ImplGenerics,
    ty_generics: &syn::TypeGenerics,
    where_clause: &Option<&syn::WhereClause>,
    user_methods: &[ImplItemFn],
) -> TokenStream2 {
    let find = |name: &str| user_methods.iter().find(|f| f.sig.ident == name).map(|f| quote! { #f });
    let as_ui_element = find("as_ui_element").unwrap_or(quote! {
        fn as_ui_element(&self) -> &#core::ui::UIElementImpl { self.base.as_ui_element() }
    });
    let measure_override = find("measure_override").unwrap_or(quote! {
        fn measure_override(&self, available: #core::base::Size) -> #core::base::Size {
            self.base.measure_override(available)
        }
    });
    let arrange_override = find("arrange_override").unwrap_or(quote! {
        fn arrange_override(&self, final_size: #core::base::Size) -> #core::base::Size {
            self.base.arrange_override(final_size)
        }
    });
    let try_as_native_control = find("try_as_native_control").unwrap_or(quote! {
        fn try_as_native_control(&self) -> Option<&dyn std::any::Any> { self.base.try_as_native_control() }
    });
    let visual_children = find("visual_children");
    let paint = find("paint");
    quote! {
        impl #impl_generics #core::ui::UIElement for #impl_name #ty_generics #where_clause {
            #as_ui_element
            #measure_override
            #arrange_override
            #try_as_native_control
            #visual_children
            #paint
        }
    }
}
