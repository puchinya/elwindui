//! `#[elwindui_macros::class]` — automates the H.2.1a class-hierarchy convention
//! (docs/elwindui_spec.md 付録H.2.1a): `struct ClassName { base: SuperClassName, .. }` implementing
//! `trait ClassNameExt: SuperClassNameExt` (plus every ancestor trait), delegating ancestor methods
//! to `self.base`.
//!
//! **Naming**: the struct is always compiled under exactly the identifier written in source (no
//! suffix ever appended/stripped) — `struct ClassName { .. }` compiles to `ClassName`, full stop.
//! The class's own derived trait (when it has one) is always `{ClassName}Ext` — an ordinary Rust
//! trait/struct pair can't share a bare name (same type namespace within a module), and `ClassName`
//! itself reads far better in a debugger/IDE than a synthesized `ClassNameImpl` ever did, so the
//! suffix moved onto the trait instead of the struct.
//!
//! Applied to a bare `struct ClassName { .. }` (no `base` field written by hand) and, separately, a
//! bare `impl ClassName { .. }` (no `for`) — two independent attribute invocations rather than one
//! `mod`-wrapped pair. `inherits`/`struct_only`/`abstract_class`/`sealed` are declared **once**, on
//! the `struct`, even though several of them (`struct_only`/`abstract_class`) are only ever consumed
//! while expanding the `impl` — the struct's own expansion (`store_class_args`) stashes a snapshot
//! in a process-global map keyed by class name, and the paired `impl ClassName { .. }`, written as a
//! bare `#[class]`, reads it back (`load_class_args`) instead of repeating the args. This makes
//! `struct` before `impl` textual order a real requirement (not just a readability convention): the
//! struct's attribute must expand first so the map entry exists when the impl's attribute looks it
//! up — true within a single crate's compilation, where outer attribute macros on top-level items
//! expand in source order in one process. An impl may still pass args explicitly (the pre-existing
//! form) instead of relying on the store; explicit args always win.
//!
//! `trait_only` is a different shape entirely (see its own paragraph below): it attaches to a bare
//! `trait ClassName { .. }` item directly, in a *single* self-contained invocation — no paired
//! `struct`/`impl`, no store/load round-trip.
//!
//! Ancestor delegation is **not** implemented as a fully generic cross-crate manifest/token-
//! accumulator (the `ambassador` crate's technique) — it doesn't need to be, because every
//! `inherits = ..` target's `base` field type genuinely implements `UIElementExt` itself
//! (`UIElement` included — its own trivial, identity-function `impl UIElementExt for UIElement` is
//! synthesized by this same macro's root-class-mode handling in `expand_impl`, not hand-written;
//! `Layout`'s is the ordinary `uielement_blind_forward` case like any other non-root class), so a
//! single blind `self.base.method(..)` forward (`uielement_blind_forward`) is always correct; the
//! macro never needs to know *which* ancestor it is.
//!
//! That blind forward only covers `UIElementExt` itself, though — an intermediate `inherits = ..`
//! trait with *its own* required methods beyond `UIElementExt`'s (`ShapeExt::set_kind`/
//! `ControlExt::set_padding`, unlike the genuinely empty `LayoutExt`/`NativeControlExt<H>` marker
//! traits) has no generic name to blindly forward through. `#[ancestor]` on an `&self` method inside
//! `impl ClassName { .. }` opts it into that trait's own impl block instead of `UIElementExt`'s or
//! `ClassNameExt`'s own (mirroring `#[inherent]`'s same attribute-driven routing, just to a
//! different bucket) — omitted entirely, the ancestor impl stays the empty `{}` block that's
//! already correct for a true marker trait.
//!
//! `inherits` omitted entirely puts a `struct`/`impl` pair into **root class mode** instead of
//! declaring an ordinary subclass — used for the one class with no ancestor of its own
//! (`UIElement`). There, `impl ClassName { .. }` isn't paired with a generated
//! `impl ClassNameExt for ClassName` at all in the usual sense — every method the user writes is
//! instead embedded, body and all, directly into the generated `pub trait ClassNameExt { .. }` as a
//! *default* method, inherited for free by every descendant via Rust's own default-method dispatch,
//! and `ClassName` implements that trait trivially (`as_ui_element(&self) -> &ClassName { self }`).
//! The one exception is `base` itself, whose concrete location differs per implementor and so can't
//! have a shared default — the macro synthesizes its (required, body-less) signature itself, and
//! errors if the user tries to define it by hand. A root class's generated trait always carries `:
//! base::AsAny` as a supertrait bound (e.g. `UIElementExt: AsAny`) — a different concept from
//! `inherits`, which additionally drives `base`-field insertion that a root class (having no
//! ancestor) never wants.
//!
//! Field-driven `Cell`/`RefCell` accessor generation is likewise not implemented — those getters/
//! setters stay hand-written in `impl ClassName { .. }`, exactly as today; they are typically one
//! line each and the real win here is eliminating the `ClassNameExt` trait declaration and the
//! ancestor delegation boilerplate.
//!
//! A `&self`-free method literally named `construct` (any signature, returning `Self`) opts a class
//! into an *auto-generated* `new`: `#[class]` emits a matching `pub fn new(<same params>) ->
//! std::rc::Rc<Self> { std::rc::Rc::new(Self::construct(<forwarded args>)) }` alongside it, so every
//! adopting class's constructor uniformly returns `Rc<Self>` (this crate's universal DSL-facing
//! convention) without hand-maintaining that wrapper itself. `construct`'s own body is entirely
//! hand-written, exactly like the `new() -> Self` it replaces: for an `inherits = Base` class it
//! typically builds its own `base` field by calling `Base::construct(..)`, recursively, all the way
//! to the root's `Default::default()`. A class not using this convention (a hand-written `new`
//! returning `Rc<Self>` directly, the pre-existing form) is entirely unaffected; defining *both*
//! `construct` and `new` in the same `impl` block is also fine — the hand-written `new` simply wins
//! (no auto-generation) — for a class whose `new` needs real post-construction work beyond
//! `Rc::new(Self::construct(..))` itself (e.g. rewiring parent pointers), while `construct` still
//! exists for other classes to call when they only need the bare, unwrapped value.
//!
//! `trait_only` declares a pure interface/marker trait with no backing struct anywhere in *this*
//! crate at all — every concrete implementor (in this crate or a backend crate) provides its own
//! struct via `struct_only = ..` instead (e.g. `elwindui_core::ui::MenuItemExt`, implemented by each
//! backend's own `MenuItem` struct). It attaches directly to a real `pub trait ClassName { .. }`
//! item (`expand_trait_only`), renaming it to `ClassNameExt` the same way every other shape does —
//! not a `struct`, since a `struct` has nowhere to put the trait's own method signatures, and the
//! alternative (a paired `impl` block) would need meaningless dummy bodies just to satisfy Rust's
//! impl-block syntax, only to have those bodies thrown away. The user's own trait items (already
//! bodyless, ordinary trait-method syntax) pass through unchanged; the macro's own job is to rename
//! the trait and add the supertrait bound — `inherits = ..` if given (rewritten to `..Ext`), or
//! `base::AsAny` automatically otherwise, the same "no ancestor -> `AsAny`" rule root class mode
//! uses above. No paired `impl ClassName { .. }` is expected or consumed — this is a single,
//! self-contained invocation, unlike every other shape this macro supports.
//!
//! **`struct_only`**: declares `ClassName` a pure struct implementor of an *existing* trait — no new
//! `pub trait ClassNameExt` is generated; the given path (which must itself already be a real trait,
//! typically another class's own `..Ext`) is implemented directly for `ClassName` instead. A
//! `struct_only` class therefore has no "own trait" of its own — `ancestor_registry` records this
//! (`has_own_trait = false`) so any subclass whose `inherits = ..` names it knows not to generate an
//! intermediate ancestor-trait `impl` for that hop, and not to add a (nonexistent) `..Ext` supertrait
//! bound to its own generated trait either.

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

/// The fixed method set every `UIElementExt` implementor may define — used to route a method
/// written inside `impl ClassName { .. }` to the generated `impl elwindui_core::ui::UIElementExt for
/// ClassName` block instead of to `ClassNameExt`'s own trait impl.
const UI_ELEMENT_METHODS: &[&str] =
    &["as_ui_element", "visual_children", "measure_override", "arrange_override", "paint", "try_as_native_control"];

/// Parsed `#[class(inherits = .., struct_only = .., abstract_class, sealed, no_ui_element)]`
/// arguments — every key is optional and any subset/order is accepted.
#[derive(Default)]
struct ClassArgs {
    inherits: Option<Type>,
    /// Declares `ClassName` a pure struct implementor of an *existing* trait — no new `pub trait
    /// ClassNameExt` is generated; the given path is implemented directly for `ClassName` instead.
    /// The `trait_only`/`struct_only` pair mirrors a trait definition and its implementation living
    /// in separate files/crates: `trait_only` declares the trait (no backing struct), `struct_only`
    /// declares a concrete struct implementing a trait declared elsewhere (no new trait of its own).
    struct_only: Option<Path>,
    abstract_class: bool,
    sealed: bool,
    /// Declares `ClassName` a pure trait — no backing struct, no `base` field, no `impl ClassNameExt
    /// for ClassName` at all. Attaches to a bare `trait ClassName { .. }` item directly
    /// (`expand_trait_only`), not a `struct` — see this module's own doc comment. Used for a
    /// marker/interface trait with no real backing implementation of its own in this crate (e.g.
    /// `elwindui_core::ui::NativeControlExt`/`MenuItemExt` — each concrete implementor provides its
    /// own `struct_only = ..` struct instead).
    trait_only: bool,
    /// `inherits = ..`'s target does *not* itself relate to `UIElement` (e.g. `Window`, deliberately
    /// outside the `UIElement` tree — see `elwindui_core::ui`'s own top doc comment). Without this,
    /// `expand_impl` unconditionally assumes any `inherits = ..` target's `base` field implements
    /// `UIElementExt` and blind-forwards to it (`uielement_blind_forward`) — which would fail to
    /// compile against a base with no `as_ui_element`/etc. of its own. Set, this skips that blind
    /// forward entirely and folds the `UI_ELEMENT_METHODS` partitioning away (every instance method
    /// becomes an "own" method, the same as root class mode) — the ancestor trait's own required
    /// methods (`#[ancestor]`-tagged, e.g. `Window`'s `set_title`/..) are unaffected either way.
    no_ui_element: bool,
    /// Only meaningful alongside `struct_only`: declares that the given trait must *not* be blindly
    /// forwarded (an empty `impl` block) when this class is later named as someone else's `inherits
    /// = ..` target — because, unlike a marker trait with zero required methods (e.g.
    /// `NativeControlExt`), it has real required methods a blind forward can't satisfy (e.g.
    /// `NativeTabView`'s `struct_only = TabView`, where `TabView` is a hand-written trait with
    /// `insert_tab`/`remove_tab`/etc.). Without this, `expand_impl`'s hop-0/transitive ancestor-impl
    /// generation would try to `impl` that trait with an empty body for any subclass composing this
    /// one as its `base`, and fail with a "missing trait items" error. A subclass that needs those
    /// methods reaches them through this class's own accessor (`self.base.foo()`) instead, never by
    /// implementing the trait itself.
    no_ancestor_forward: bool,
}

impl Parse for ClassArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut args = ClassArgs::default();
        let items = Punctuated::<ClassArg, Token![,]>::parse_terminated(input)?;
        for item in items {
            match item {
                ClassArg::Inherits(ty) => args.inherits = Some(ty),
                ClassArg::StructOnly(path) => args.struct_only = Some(path),
                ClassArg::AbstractClass => args.abstract_class = true,
                ClassArg::Sealed => args.sealed = true,
                ClassArg::TraitOnly => args.trait_only = true,
                ClassArg::NoUiElement => args.no_ui_element = true,
                ClassArg::NoAncestorForward => args.no_ancestor_forward = true,
            }
        }
        Ok(args)
    }
}

enum ClassArg {
    Inherits(Type),
    StructOnly(Path),
    AbstractClass,
    Sealed,
    TraitOnly,
    NoUiElement,
    NoAncestorForward,
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
            "abstract_class" => Ok(ClassArg::AbstractClass),
            "sealed" => Ok(ClassArg::Sealed),
            "trait_only" => Ok(ClassArg::TraitOnly),
            "no_ui_element" => Ok(ClassArg::NoUiElement),
            "no_ancestor_forward" => Ok(ClassArg::NoAncestorForward),
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
    abstract_class: bool,
    sealed: bool,
    trait_only: bool,
    no_ui_element: bool,
    no_ancestor_forward: bool,
}

/// Keyed by class name (e.g. `"VerticalLayout"`) — populated by `struct ClassName`'s own
/// `#[class(..)]` invocation, read by the paired `impl ClassName { .. }`'s bare `#[class]` (see
/// `expand`/`expand_impl`). Relies on the struct's attribute expanding before its paired impl's
/// within the same crate compilation — see this module's own doc comment.
fn class_arg_store() -> &'static Mutex<HashMap<String, StoredClassArgs>> {
    static STORE: OnceLock<Mutex<HashMap<String, StoredClassArgs>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn store_class_args(class_name: &str, args: &ClassArgs) {
    let stored = StoredClassArgs {
        inherits: args.inherits.as_ref().map(|t| quote! { #t }.to_string()),
        struct_only: args.struct_only.as_ref().map(|p| quote! { #p }.to_string()),
        abstract_class: args.abstract_class,
        sealed: args.sealed,
        trait_only: args.trait_only,
        no_ui_element: args.no_ui_element,
        no_ancestor_forward: args.no_ancestor_forward,
    };
    class_arg_store().lock().unwrap().insert(class_name.to_string(), stored);
}

/// `None` means no `struct ClassName` has been seen yet under this name — the caller (`expand_impl`)
/// turns that into a `compile_error!` pointing at the missing/misordered struct declaration. Removes
/// the entry on success — a bare class name isn't unique crate-wide (e.g. a backend's own
/// `NativeControl` vs. an unrelated same-named class elsewhere), so consuming on read means each
/// struct/impl pair only ever collides with a same-named pair that's still "open" (pushed but not
/// yet consumed), which the struct-immediately-followed-by-its-own-impl convention this whole macro
/// depends on never produces.
fn load_class_args(class_name: &str) -> Option<ClassArgs> {
    let mut store = class_arg_store().lock().unwrap();
    let stored = store.remove(class_name)?;
    let parse_type = |s: &String| syn::parse_str::<Type>(s).expect("#[class]: internal: failed to reparse stored `inherits` type");
    let parse_path = |s: &String| syn::parse_str::<Path>(s).expect("#[class]: internal: failed to reparse stored path");
    Some(ClassArgs {
        inherits: stored.inherits.as_ref().map(parse_type),
        struct_only: stored.struct_only.as_ref().map(parse_path),
        abstract_class: stored.abstract_class,
        sealed: stored.sealed,
        trait_only: stored.trait_only,
        no_ui_element: stored.no_ui_element,
        no_ancestor_forward: stored.no_ancestor_forward,
    })
}

/// Per-class bookkeeping needed by a *subclass* resolving its own `inherits = ..` target: what that
/// target's own `inherits` is (to keep walking the chain for `as_<ancestor>()` accessors); the trait
/// a further subclass should `impl`/supertrait-bound against when *this* class appears as one of
/// *its* ancestors (`{Name}Ext` for an ordinary/root/`trait_only` class, or whatever `struct_only`'s
/// path argument was for a `struct_only` one — see this module's own doc comment on `struct_only`);
/// whether that trait can be safely blind-forwarded with an empty `impl` block (false for a
/// `struct_only` class explicitly marked `no_ancestor_forward`, e.g. `NativeTabView` composing a
/// hand-written trait with real required methods); and whether this class is itself `struct_only`
/// (composition, not subclassing — used only to exempt it from the `#[sealed]` check, since a
/// backend's own raw native leaf can share a sealed DSL builtin's bare name without *being* it).
struct AncestorInfo {
    inherits: Option<String>,
    own_trait: String,
    forwardable: bool,
    is_struct_only: bool,
}

/// Keyed by class name — populated by every `struct ClassName { .. }`/`trait_only` expansion
/// (`register_ancestor`) and walked by `expand_impl` to generate `as_<snake(ancestor)>()` accessors
/// reaching beyond a class's immediate parent, and to decide whether an intermediate ancestor needs
/// its own trait `impl`/supertrait bound. First-write-wins (`Entry::or_insert`) since a bare class
/// name isn't unique crate-wide; reaches every ancestor declared in the same crate, and exactly one
/// hop further into an external crate before the registry (necessarily local-only) runs out — a
/// lookup miss defaults to "ordinary, forwardable" (true for every real cross-crate class this macro
/// manages; a `struct_only` ancestor needs the negative answer, and those are always resolvable
/// within the same crate in practice).
fn ancestor_registry() -> &'static Mutex<HashMap<String, AncestorInfo>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, AncestorInfo>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn register_ancestor(class_name: &str, inherits: &Option<Type>, own_trait: String, forwardable: bool, is_struct_only: bool) {
    let value = inherits.as_ref().map(|t| quote! { #t }.to_string());
    ancestor_registry()
        .lock()
        .unwrap()
        .entry(class_name.to_string())
        .or_insert(AncestorInfo { inherits: value, own_trait, forwardable, is_struct_only });
}

/// `Some((inherits, own_trait))` when `bare_name` is a registered local class — `inherits` is `None`
/// for a registered local root (root mode, no further ancestor). `None` = not found — either a
/// genuinely external/cross-crate type, or not yet registered (this class's struct hasn't expanded
/// yet — see `register_ancestor`'s own doc comment on why declaration order matters).
fn lookup_ancestor(bare_name: &str) -> Option<(Option<Type>, Type)> {
    let registry = ancestor_registry().lock().unwrap();
    let entry = registry.get(bare_name)?;
    let inh = entry.inherits.as_ref().map(|s| syn::parse_str::<Type>(s).expect("#[class]: internal: failed to reparse stored ancestor type"));
    let own_trait = syn::parse_str::<Type>(&entry.own_trait).expect("#[class]: internal: failed to reparse stored own_trait type");
    Some((inh, own_trait))
}

/// The trait a subclass should `impl`/supertrait-bound against when naming `name` (bare) as its
/// `inherits = ..` target — `{name}Ext` on a lookup miss (assumes an ordinary cross-crate class; see
/// `ancestor_registry`'s own doc comment).
fn ancestor_own_trait(name: &str, fallback: &Type) -> Type {
    lookup_ancestor(name).map(|(_, own_trait)| own_trait).unwrap_or_else(|| ext_trait_type(fallback))
}

/// Whether the registered (or, on a lookup miss, assumed) class named `name` can be safely blind-
/// forwarded — see `AncestorInfo`'s own doc comment.
fn ancestor_forwardable(name: &str) -> bool {
    let registry = ancestor_registry().lock().unwrap();
    registry.get(name).map(|e| e.forwardable).unwrap_or(true)
}

/// Whether the registered class named `name` is `struct_only` (composition, not subclassing) — used
/// only to exempt it from the `#[sealed]` check (a `struct_only` composing a sealed DSL builtin's
/// bare-named raw leaf isn't itself "inheriting" that sealed class). Assumes not on a lookup miss.
fn ancestor_is_struct_only(name: &str) -> bool {
    let registry = ancestor_registry().lock().unwrap();
    registry.get(name).map(|e| e.is_struct_only).unwrap_or(false)
}

/// `elwindui_core::ui::NativeControl<AnyView>` -> `"NativeControl"`; the bare display name used both
/// as an `ancestor_registry` lookup key and to derive an `as_<snake(name)>()` accessor's method name.
fn last_segment_name(ty: &Type) -> Option<String> {
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

/// `VerticalLayout` -> `VerticalLayoutExt`: the class's own derived-trait name, used for its trait
/// declaration and `impl` target.
fn to_ext_ident(name: &str, span: proc_macro2::Span) -> Ident {
    Ident::new(&format!("{name}Ext"), span)
}

/// `elwindui_core::ui::Shape` -> `elwindui_core::ui::ShapeExt`: rewrites just the last path segment,
/// for referencing an ancestor's derived trait (supertrait bounds, ancestor-trait `impl` targets) —
/// the rest of the path (module qualifiers) passes through untouched.
fn ext_trait_type(ty: &Type) -> Type {
    let mut ty = ty.clone();
    if let Type::Path(tp) = &mut ty {
        if let Some(seg) = tp.path.segments.last_mut() {
            seg.ident = Ident::new(&format!("{}Ext", seg.ident), seg.ident.span());
        }
    }
    ty
}

/// `ContentControlExt` -> `__dyn_content_control`; `ContentControl` -> the same (the `Ext` suffix is
/// optional in the input so callers can pass either a class's bare name or its already-`Ext`-suffixed
/// trait name interchangeably). This is the one required, non-default method every `{Name}Ext` trait
/// declares (mirroring `UIElementExt`'s own pre-existing `as_ui_element`, generalized): it returns
/// `&dyn {Name}Ext` rather than a concrete struct type specifically so it works uniformly across
/// every class shape this macro supports — ordinary/root classes (one well-known concrete struct),
/// *and* `trait_only` marker/interface traits (no concrete struct at all in this crate; each backend
/// provides its own via `struct_only`, so there's no single type name the trait declaration itself
/// could hard-code). Every other method of `{Name}Ext` becomes a *default* method with body
/// `self.__dyn_x().method(args)` — dispatched dynamically through this accessor — so a descendant
/// only ever has to implement the accessor itself (typically one line, `&self.base` or
/// `self.base.__dyn_x()`) to inherit every one of the ancestor's real methods for free, without the
/// macro needing to know their names/signatures at each *use* site the way the old `build_forward_macro`
/// design did (removed — it required a macro-exported `macro_rules!`, which hit an unresolvable
/// cross-crate reachability wall; see git history if curious). The *declaring* class itself
/// (`ClassName`'s own `impl ClassNameExt for ClassName`) is the one implementor that must override
/// every method with its real body instead of relying on the default — relying on the default there
/// would recurse forever, since its own `__dyn_x` is reflexive (`{ self }`).
///
/// Requires every `{Name}Ext` method to be dyn-compatible (object-safety rules: no generics, no `Self`
/// by value, ...) — true of every class in this codebase today (plain property-style getters/setters).
/// A future method violating this would surface as a loud "the trait cannot be made into an object"
/// compile error right at the trait declaration, rather than silently misbehaving.
fn dyn_accessor_ident(bare_name: &str) -> Ident {
    let bare = bare_name.strip_suffix("Ext").unwrap_or(bare_name);
    format_ident!("__dyn_{}", to_snake_case(bare))
}

/// `#[overridable]`/`#[overrides]` validation: `#[overridable] fn foo(&self, ..) -> T { .. }` on a
/// class's own method (`own_methods`) additionally generates a one-method marker trait,
/// `__Overridable_{ClassName}_{foo}`, declared right alongside `{ClassName}Ext` itself (same module).
/// A descendant tagging its own override of that ancestor method `#[overrides]` (typically alongside
/// `#[ancestor]`, since only an *immediate* ancestor's own method is currently resolvable this way —
/// see `expand_impl`'s hop-0 handling) gets an *additional* `impl __Overridable_{ClassName}_{foo} for
/// Self { fn foo(..) { <same body> } }` alongside its regular ancestor-trait impl.
///
/// No bespoke checking is done beyond generating these two pieces — validation is entirely rustc's
/// own trait-impl checking, for free: overriding a method the ancestor never marked `#[overridable]`
/// means this marker trait path simply doesn't exist (E0433, unresolved path); overriding with the
/// wrong signature fails the same way any mismatched trait impl does (E0050/E0053). This is
/// deliberately not the same mechanism as the spec's component-level `#[overrides(builtin::X)]`
/// shadowing rule (docs/elwindui_spec.md 付録E) — same attribute name, different syntactic position
/// (component-level vs. method-level), no conflict.
fn overridable_marker_ident(class_bare_name: &str, method: &str) -> Ident {
    format_ident!("__Overridable_{}_{}", class_bare_name, method)
}

/// Builds the marker trait's *path* (module-qualified the same way `base_ty` already is) for a
/// descendant's `#[overrides]` impl to target — `base_ty` is the ancestor's own `inherits`-target
/// type (hop-0's `inh`), whose module prefix is exactly where that ancestor's own `{ClassName}Ext`
/// (and so its sibling marker traits) were declared.
fn overridable_marker_path(base_ty: &Type, class_bare_name: &str, method: &str) -> Type {
    let mut ty = base_ty.clone();
    if let Type::Path(tp) = &mut ty {
        if let Some(seg) = tp.path.segments.last_mut() {
            seg.ident = overridable_marker_ident(class_bare_name, method);
            seg.arguments = syn::PathArguments::None;
        }
    }
    ty
}

/// Builds the `fn #dyn_ident(&self) -> &dyn #ext_ty;` (no default) plus one default method per `sig`
/// (`{ #ext_ty::#name(self.#dyn_ident(), args) }`) — shared by `expand_trait_only` and
/// `expand_impl`'s ordinary-class branch, the two places that declare a *new* `{Name}Ext` trait
/// (`struct_only` never does; root mode has its own pre-existing, differently-shaped `as_ui_element`
/// version of this same idea and isn't routed through here).
///
/// Calls through fully-qualified syntax (`#ext_ty::#name(receiver, ..)`), not `receiver.#name(..)` —
/// deliberately: a subclass is free to reuse an ancestor's method name for its own, differently-typed
/// concept (e.g. `ContentControl::padding(&self) -> Option<f32>` alongside `Control::padding(&self)
/// -> f32`, forwarded via `#[ancestor]`), which makes `ContentControlExt: ControlExt` declare two
/// same-named methods. Plain dot-syntax on a `&dyn ContentControlExt` receiver is ambiguous between
/// the two (E0034) regardless of their differing signatures — Rust's method resolution doesn't
/// disambiguate on return type — so the default body must name which trait's method it means.
fn build_dyn_default_methods(dyn_ident: &Ident, ext_ty: &TokenStream2, sigs: &[syn::Signature]) -> Vec<TokenStream2> {
    sigs.iter()
        .map(|sig| {
            let name = &sig.ident;
            let arg_names: Vec<TokenStream2> = sig
                .inputs
                .iter()
                .filter_map(|arg| match arg {
                    FnArg::Typed(pat_type) => {
                        let pat = &pat_type.pat;
                        Some(quote! { #pat })
                    }
                    FnArg::Receiver(_) => None,
                })
                .collect();
            quote! {
                #sig {
                    #ext_ty::#name(self.#dyn_ident() #(, #arg_names)*)
                }
            }
        })
        .chain(std::iter::once(quote! { fn #dyn_ident(&self) -> &dyn #ext_ty; }))
        .collect()
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
            if args.trait_only {
                let msg = "#[class]: `trait_only` attaches to a `trait ClassName { .. }` item directly (its own \
                           method signatures live in it, ordinary bodyless trait syntax) — declare `pub trait \
                           ClassName { .. }` here instead of `struct ClassName {}`";
                return syn::Error::new_spanned(&item_struct, msg).to_compile_error();
            }
            store_class_args(&item_struct.ident.to_string(), &args);
            let class_name = item_struct.ident.to_string();
            let own_trait = match &args.struct_only {
                Some(existing) => quote! { #existing }.to_string(),
                None => format!("{class_name}Ext"),
            };
            register_ancestor(&class_name, &args.inherits, own_trait, !args.no_ancestor_forward, args.struct_only.is_some());
            expand_struct(&args, item_struct)
        }
        Item::Impl(item_impl) => expand_impl(args, item_impl, attr_is_empty),
        Item::Trait(item_trait) => expand_trait_only(&args, item_trait),
        other => {
            let msg = "#[class]: expected a `struct ClassName { .. }`, `impl ClassName { .. }`, or (with \
                       `trait_only`) `trait ClassName { .. }` item";
            let mut ts = syn::Error::new_spanned(&other, msg).to_compile_error();
            ts.extend(quote! { #other });
            ts
        }
    }
}

/// Three possible compiling contexts for `#[class]`, distinguished by `CARGO_PKG_NAME` (set by
/// cargo to whichever crate is currently being compiled, i.e. exactly the crate this macro
/// invocation is expanding within — a crate cannot refer to itself by its external `extern crate`
/// name, hence the first case):
/// - Inside `elwindui-core` itself (its own `ui.rs` uses this macro too, e.g. for
///   `NativeControl<H>`) -> `crate::ui::X`.
/// - Inside one of the (hardcoded, like `SEALED_CLASSES` — see this module's own doc comment for why
///   a manifest-derived list isn't worth it here) backend crates, which depend on `elwindui-core`
///   directly -> `elwindui_core::ui::X`.
/// - Anywhere else — a consumer's own crate, where `elwindui-codegen`-generated code (`.elwind` ->
///   Rust) is the only other place this macro runs, and that consumer only ever has `elwindui`
///   itself (the facade) as a direct dependency, never `elwindui_core` -> `elwindui::core::ui::X`.
fn core_path() -> TokenStream2 {
    match std::env::var("CARGO_PKG_NAME").as_deref() {
        Ok("elwindui-core") => quote! { crate },
        Ok("elwindui-backend-appkit" | "elwindui-backend-winui3" | "elwindui-backend-gtk4") => quote! { elwindui_core },
        _ => quote! { elwindui::core },
    }
}

fn expand_struct(args: &ClassArgs, item: syn::ItemStruct) -> TokenStream2 {
    let class_name = &item.ident;
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

    // `pub` (matching H.2.1a's convention): plenty of call sites reach through `.base` across
    // module/crate boundaries (`self.base.handle`, `self.base.as_ui_element()`, ...). The field's
    // type is the `inherits = ..` target exactly as written — struct names are never rewritten, so
    // no suffix transform is needed here.
    let base_field = args.inherits.as_ref().map(|ty| quote! { pub base: #ty, });

    quote! {
        #(#attrs)*
        #vis struct #class_name #generics {
            #base_field
            #(#existing_fields,)*
        }
    }
}

/// `trait_only` (see this module's own doc comment): a single, self-contained invocation on a bare
/// `trait ClassName { .. }` item — the user's own trait items (already ordinary, bodyless trait
/// methods) pass through unchanged; the macro renames the trait to `ClassNameExt` and adds the
/// supertrait bound (`inherits = ..` if given, rewritten to `..Ext`, or `base::AsAny` automatically
/// otherwise, mirroring root class mode's own "no ancestor -> `AsAny`" rule in `expand_impl`). No
/// paired `struct`/`impl` is expected — there's no `store_class_args`/`load_class_args` round-trip
/// here at all.
fn expand_trait_only(args: &ClassArgs, item: syn::ItemTrait) -> TokenStream2 {
    if !args.trait_only {
        let msg = "#[class]: a bare `trait ClassName { .. }` item is only valid with `trait_only` — declare a \
                   `struct ClassName { .. }` for an ordinary class, or add `trait_only` here for a pure \
                   marker/interface trait";
        return syn::Error::new_spanned(&item, msg).to_compile_error();
    }
    let class_name = &item.ident;
    let ext_name = to_ext_ident(&class_name.to_string(), class_name.span());
    let vis = &item.vis;
    let attrs = &item.attrs;
    let generics = &item.generics;
    let where_clause = &item.generics.where_clause;
    let items = &item.items;
    let user_supertraits = &item.supertraits;
    let core = core_path();
    let bound = match &args.inherits {
        Some(t) => {
            let name = last_segment_name(t);
            let own_trait = name.as_deref().map(|n| ancestor_own_trait(n, t));
            let bound_t = own_trait.unwrap_or_else(|| ext_trait_type(t));
            quote! { #bound_t }
        }
        None => quote! { #core::base::AsAny },
    };
    let colon_bound = if user_supertraits.is_empty() { quote! { : #bound } } else { quote! { : #user_supertraits + #bound } };
    register_ancestor(&class_name.to_string(), &args.inherits, format!("{class_name}Ext"), true, false);
    let sigs: Vec<syn::Signature> = items
        .iter()
        .filter_map(|item| match item {
            syn::TraitItem::Fn(f) => Some(f.sig.clone()),
            _ => None,
        })
        .collect();
    let dyn_ident = dyn_accessor_ident(&class_name.to_string());
    let ext_ty = quote! { #ext_name };
    let dyn_methods = build_dyn_default_methods(&dyn_ident, &ext_ty, &sigs);
    quote! {
        #(#attrs)*
        #vis trait #ext_name #generics #colon_bound #where_clause {
            #(#dyn_methods)*
        }
    }
}

fn expand_impl(attr_args: ClassArgs, item: syn::ItemImpl, attr_is_empty: bool) -> TokenStream2 {
    if item.trait_.is_some() {
        return syn::Error::new_spanned(
            &item,
            "#[class]: write `impl ClassName { .. }` (no `for`) — the macro routes each method to the right \
             generated `impl .. for ClassName` block itself",
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
                     this file — declare the struct (with any inherits/struct_only/... args) before this \
                     impl block, or pass args explicitly here"
                );
                return syn::Error::new_spanned(&item.self_ty, msg).to_compile_error();
            }
        }
    } else {
        attr_args
    };
    let args = &args;
    // The compiled struct is always exactly `class_name` — no suffix transform.
    let impl_name = class_name.clone();
    // `<H>`/`<H: 'static>`/where-clause, threaded through every generated block below so a generic
    // class (e.g. `NativeControl<H>`) works the same as a non-generic one.
    let (impl_generics, ty_generics, where_clause) = item.generics.split_for_impl();

    if let Some(inh) = &args.inherits {
        if let Type::Path(tp) = inh {
            if let Some(seg) = tp.path.segments.last() {
                // Only a real `{Name}Ext` trait can be "inherited" in the sealed sense — a
                // `struct_only` target (e.g. a backend's own raw native leaf struct, composed as a
                // `base` field rather than subclassed via a derived trait) can share a sealed
                // class's bare name without actually being that sealed class, so it must not trip
                // this check.
                let name = seg.ident.to_string();
                if SEALED_CLASSES.contains(&name.as_str()) && !ancestor_is_struct_only(&name) {
                    let msg = format!("class `{}` is #[sealed] and cannot be inherited", seg.ident);
                    return syn::Error::new_spanned(inh, msg).to_compile_error();
                }
            }
        }
    }

    // `as_<snake(ancestor)>()` accessors (docs/elwindui_spec.md 付録H.2.1a's `base`-chain convention,
    // exposed as named methods instead of the ancestor-skipping `base()` trait method they replace).
    // The immediate parent always gets one (`&self.base`, no registry lookup needed — works across
    // crate boundaries since it's a plain field access). Deeper ancestors are chain-walked via
    // `ancestor_registry`, one level at a time, delegating to `self.base`'s own same-named accessor
    // (which that class's own `#[class]` expansion already generated the same way) — so this reaches
    // every ancestor declared in the same crate, and exactly one level further into an external crate
    // before the registry (necessarily local-only) runs out and the walk stops there. `UIElement`
    // itself is always skipped: `as_ui_element()` is generated unconditionally elsewhere
    // (`uielement_blind_forward`) regardless of how many hops away the root actually is, so repeating
    // it here would conflict.
    let mut accessor_methods: Vec<TokenStream2> = Vec::new();
    // Ancestor-trait `impl`s found by continuing the walk *beyond* the immediate `args.inherits`
    // target (hop 0 — handled separately, below, with `#[ancestor]`-tagged method routing for a
    // trait that has real required methods). Every further hop found via `ancestor_registry` is
    // auto-forwarded via that hop's own `__dyn_x` accessor method (`dyn_accessor_ident`/
    // `build_dyn_default_methods`): `self.base`'s type already implements this hop's trait itself
    // (it's *that* hop's own `base` field, one level closer), so `self.base.#dyn_ident()` reaches it —
    // no need to know any of the trait's real method names/signatures at this transitively-reached
    // use site, since every method beyond the accessor itself is a default method on the trait
    // declaration (see `build_dyn_default_methods`'s own doc comment).
    let mut transitive_ancestor_impls: Vec<TokenStream2> = Vec::new();
    if let Some(parent_ty) = &args.inherits {
        if let Some(mut current_display) = last_segment_name(parent_ty) {
            let mut current_ty = parent_ty.clone();
            // Not pre-seeded with `class_name` itself: under the new bare-name convention it's
            // common (and correct) for a class to share its bare display name with an ancestor in a
            // *different* module/crate (e.g. a DSL wrapper and the raw native leaf it composes, both
            // legitimately called `TextArea`) — pre-seeding would flag that as a false-positive
            // cycle. A genuine cycle in the chain itself is still caught below, since each hop's own
            // display name is inserted as the walk visits it.
            let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut hop_index = 0usize;
            loop {
                if current_display == "UIElement" {
                    break;
                }
                if !visited.insert(current_display.clone()) {
                    // Cycle — almost certainly a same-crate name collision the registry can't tell
                    // apart. Stop rather than loop forever or generate an accessor built from the
                    // wrong ancestor's data.
                    break;
                }
                let method_name = format_ident!("as_{}", to_snake_case(&current_display));
                let body = if accessor_methods.is_empty() {
                    quote! { &self.base }
                } else {
                    quote! { self.base.#method_name() }
                };
                accessor_methods.push(quote! {
                    pub fn #method_name(&self) -> &#current_ty { #body }
                });
                if hop_index > 0 && ancestor_forwardable(&current_display) {
                    let own_trait_ty = ancestor_own_trait(&current_display, &current_ty);
                    let dyn_ident = dyn_accessor_ident(&current_display);
                    transitive_ancestor_impls.push(quote! {
                        impl #impl_generics #own_trait_ty for #impl_name #ty_generics #where_clause {
                            fn #dyn_ident(&self) -> &dyn #own_trait_ty { self.base.#dyn_ident() }
                        }
                    });
                }
                hop_index += 1;
                match lookup_ancestor(&current_display) {
                    Some((Some(next_ty), _)) => match last_segment_name(&next_ty) {
                        Some(next_display) => {
                            current_ty = next_ty;
                            current_display = next_display;
                        }
                        None => break,
                    },
                    // Local root (`inherits` omitted) or not found (external/cross-crate, or a
                    // `struct_only` class never registered under this name) — stop; the one hop
                    // already generated above is as far as this walk goes.
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
    // `inherits` trait's own impl block rather than `UIElementExt`'s or `ClassNameExt`'s own.
    let mut ancestor_methods: Vec<ImplItemFn> = Vec::new();
    // `#[overridable]`-tagged own method names (see `overridable_marker_ident`'s own doc comment) —
    // names only, matched back against `own_methods` once it's computed below, since `own_methods`
    // isn't partitioned out of `instance_methods` until after this loop.
    let mut overridable_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    // `#[overrides]`-tagged methods, kept as full clones (signature + body) alongside whichever
    // bucket they were also routed into above (`#[overrides]` doesn't change *where* a method is
    // implemented — that's still `#[ancestor]`'s/plain routing's job — it only additionally opts the
    // method into a *second*, separate marker-trait impl; see `overridable_marker_ident`'s own doc
    // comment).
    let mut overrides_methods: Vec<ImplItemFn> = Vec::new();
    for impl_item in item.items {
        match impl_item {
            ImplItem::Fn(mut f) => {
                // `#[inherent]` opts a `&self` method *out* of trait-impl routing entirely — for
                // helpers that aren't part of any trait (ancestor's or `ClassNameExt`'s own), e.g. the
                // backend facade layer's `into_any_view`/`set_on_text_change`. It lands as a plain
                // `impl ClassName { .. }` method, alongside constructors.
                let is_inherent = f.attrs.iter().any(|a| a.path().is_ident("inherent"));
                // `#[ancestor]` opts a `&self` method *into* the `inherits` trait's own impl block
                // (e.g. `ShapeExt::set_kind`/`ControlExt::set_padding` — a real, non-`UIElementExt`
                // required method on the immediate ancestor trait, as opposed to `UI_ELEMENT_METHODS`,
                // which covers `UIElementExt` itself, or an unmarked method, which lands on
                // `ClassNameExt`'s own trait). Needed because unlike `LayoutExt`/`NativeControlExt<H>`
                // (empty marker traits — see this module's own doc comment), a trait like
                // `ShapeExt`/`ControlExt` has required methods of its own that a blind
                // `self.base.method(..)` forward can't discover without a name to key off;
                // `#[ancestor]` is that name.
                let is_ancestor = f.attrs.iter().any(|a| a.path().is_ident("ancestor"));
                let is_overridable = f.attrs.iter().any(|a| a.path().is_ident("overridable"));
                let is_overrides = f.attrs.iter().any(|a| a.path().is_ident("overrides"));
                if is_overridable {
                    overridable_names.insert(f.sig.ident.to_string());
                }
                f.attrs.retain(|a| {
                    !(a.path().is_ident("overridable")
                        || a.path().is_ident("overrides")
                        || a.path().is_ident("inherent")
                        || a.path().is_ident("ancestor"))
                });
                if is_overrides {
                    overrides_methods.push(f.clone());
                }
                if is_inherent {
                    ctor_methods.push((f, false));
                    continue;
                }
                let has_self = matches!(f.sig.inputs.first(), Some(FnArg::Receiver(_)));
                if has_self {
                    // Every instance method lands in a trait impl (either an ancestor's or
                    // `ClassNameExt`'s own) — trait impl items always inherit the trait's own
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
    // `no_ui_element` (e.g. `Window`) folds the same way: its `inherits = ..` target isn't part of
    // the `UIElement` tree either, so there's no `UIElementExt`-override bucket to partition into.
    let is_root_mode = args.inherits.is_none();
    let (ui_element_methods, own_methods): (Vec<ImplItemFn>, Vec<ImplItemFn>) = if is_root_mode || args.no_ui_element {
        (Vec::new(), instance_methods)
    } else {
        instance_methods.into_iter().partition(|f| UI_ELEMENT_METHODS.contains(&f.sig.ident.to_string().as_str()))
    };

    // `#[overridable]` marker trait declarations (see `overridable_marker_ident`'s own doc comment) —
    // one per own method so tagged, declared alongside this class's own `{ClassName}Ext` regardless
    // of which shape branch (root/ordinary/`struct_only`) this class turns out to be below.
    let overridable_marker_decls: Vec<TokenStream2> = own_methods
        .iter()
        .filter(|f| overridable_names.contains(&f.sig.ident.to_string()))
        .map(|f| {
            let marker_ident = overridable_marker_ident(&class_name.to_string(), &f.sig.ident.to_string());
            let sig = &f.sig;
            quote! { pub trait #marker_ident { #sig; } }
        })
        .collect();

    let core = core_path();
    let mut ancestor_impls = Vec::new();
    if let Some(inh) = &args.inherits {
        // `UIElement` and `Layout` both implement `UIElementExt` themselves now (see
        // `expand_impl`'s own doc comment), so *every* `inherits = ..` target's `base` field type
        // genuinely implements `UIElementExt` — blind `self.base.method(..)` forwarding is always
        // correct, with no need to special-case which ancestor this is — *except* a `no_ui_element`
        // class (e.g. `Window`), whose `base` genuinely has no `as_ui_element`/etc. of its own to
        // forward to, so the blind forward is skipped entirely for it.
        if !args.no_ui_element {
            ancestor_impls.push(uielement_blind_forward(&core, &impl_name, &impl_generics, &ty_generics, &where_clause, &ui_element_methods));
        }
        // Any intermediate trait between `UIElementExt` and this class (`LayoutExt`,
        // `NativeControlExt<H>`, `ShapeExt`, `ControlExt`, ...) also needs an `impl` of its own —
        // skipped for `UIElement` itself (already fully covered by the blind forward above) and for
        // a `struct_only` ancestor (a concrete struct, not a trait, so there's nothing to `impl`).
        // Auto-forwarded via that trait's own `__dyn_x` accessor (`dyn_accessor_ident`): the required
        // `fn __dyn_x(&self) -> &dyn XExt { &self.base }` is *always* generated (one line, `self.base`
        // already implements `XExt` itself), and every other method of `XExt` is already a default
        // method dispatching through it (`build_dyn_default_methods`) — so this single method is
        // enough to inherit the whole trait for free. Any `#[ancestor]`-tagged methods the user did
        // write are simply appended as additional explicit overrides in the same `impl` block (Rust
        // allows overriding only *some* of a trait's default methods) — no more "all or nothing"
        // choice between auto-forward and hand-written.
        if let Type::Path(tp) = inh {
            if let Some(seg) = tp.path.segments.last() {
                let name = seg.ident.to_string();
                let hop0_forwardable = name != "UIElement" && ancestor_forwardable(&name);
                let own_trait_ty = hop0_forwardable.then(|| ancestor_own_trait(&name, inh));
                // If this class is itself `struct_only` for the *same* trait its immediate ancestor
                // already implements (e.g. a DSL wrapper's own `struct_only = ..TextAreaExt`
                // composing a raw native leaf that's *also* `struct_only = ..TextAreaExt`), the
                // `struct_only` branch below already provides that one `impl` — generating it again
                // here would conflict (E0119).
                let already_covered_by_struct_only = own_trait_ty
                    .as_ref()
                    .zip(args.struct_only.as_ref())
                    .is_some_and(|(a, b)| quote! { #a }.to_string() == quote! { #b }.to_string());
                if hop0_forwardable && !already_covered_by_struct_only {
                    let own_trait_ty = own_trait_ty.unwrap();
                    let dyn_ident = dyn_accessor_ident(&name);
                    let overrides = ancestor_methods.iter().map(|f| quote! { #f });
                    ancestor_impls.push(quote! {
                        impl #impl_generics #own_trait_ty for #impl_name #ty_generics #where_clause {
                            fn #dyn_ident(&self) -> &dyn #own_trait_ty { &self.base }
                            #(#overrides)*
                        }
                    });
                    // `#[overrides]` (see `overridable_marker_ident`'s own doc comment): any
                    // `#[ancestor]`-tagged method also tagged `#[overrides]` additionally implements
                    // this immediate ancestor's `#[overridable]`-declared marker trait for the same
                    // method — a real, rustc-checked "is this ancestor method actually overridable,
                    // and does this override's signature match" validation.
                    for f in overrides_methods.iter().filter(|f| ancestor_methods.iter().any(|a| a.sig.ident == f.sig.ident)) {
                        let marker_path = overridable_marker_path(inh, &name, &f.sig.ident.to_string());
                        ancestor_impls.push(quote! {
                            impl #impl_generics #marker_path for #impl_name #ty_generics #where_clause { #f }
                        });
                    }
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

    let ext_ident = to_ext_ident(&class_name.to_string(), class_name.span());
    let (trait_decl, trait_impl) = if let Some(existing) = &args.struct_only {
        // The concrete, hand-written implementor of someone else's `{Name}Ext` trait — this is the
        // one shape besides root mode that provides *real* bodies directly rather than relying on
        // `{Name}Ext`'s own default methods, so it must also supply that trait's required `__dyn_x`
        // accessor itself (reflexively — `self` already *is* the concrete implementor). Skipped for
        // `no_ancestor_forward` (e.g. `NativeTabView`'s `struct_only = TabView`): that target is a
        // hand-written trait predating this macro's `__dyn_x` convention, with no such method
        // declared on it at all — adding one here would be E0407 (method not a member of trait).
        let existing_name = existing.segments.last().map(|s| s.ident.to_string()).unwrap_or_default();
        let dyn_accessor = (!args.no_ancestor_forward).then(|| {
            let dyn_ident = dyn_accessor_ident(&existing_name);
            quote! { fn #dyn_ident(&self) -> &dyn #existing { self } }
        });
        let bodies = own_methods.iter().map(|f| quote! { #f });
        (
            TokenStream2::new(),
            quote! {
                impl #impl_generics #existing for #impl_name #ty_generics #where_clause {
                    #dyn_accessor
                    #(#bodies)*
                }
            },
        )
    } else if is_root_mode {
        // Root class mode (`inherits` omitted, e.g. `UIElement` itself): every method the user
        // wrote becomes a *default* trait method (body embedded directly in the trait declaration,
        // shared by every future `inherits = ClassName` descendant for free via Rust's own
        // default-method dispatch) rather than a required method paired with a separate `impl
        // ClassNameExt for ClassName` — that pairing doesn't apply here since `ClassName` itself is
        // never meant to implement `ClassNameExt` via a "real" impl body (descendants embed it as
        // `base` and implement the trait themselves) — except that `ClassName` *does* trivially
        // implement it, since `as_ui_element` is just the identity function here. `as_ui_element`
        // is the one method every implementor must supply itself (its concrete location differs per
        // type), so the macro synthesizes it as the sole required signature — the user must not
        // define it by hand.
        if let Some(base_fn) = own_methods.iter().find(|f| f.sig.ident == "as_ui_element") {
            let msg = "#[class]: root class's `as_ui_element` is auto-generated; do not define it";
            return syn::Error::new_spanned(&base_fn.sig, msg).to_compile_error();
        }
        // Every root class implicitly needs `AsAny` (e.g. `UIElementExt: AsAny`) — see this
        // function's own doc comment on root class mode.
        let bound = quote! { : #core::base::AsAny };
        let default_methods = own_methods.iter().map(|f| quote! { #f });
        (
            quote! {
                pub trait #ext_ident #impl_generics #bound #where_clause {
                    fn as_ui_element(&self) -> &#impl_name;
                    #(#default_methods)*
                }
            },
            // `ClassName` is a genuine `ClassNameExt` implementor itself — trivially, since
            // `as_ui_element` (or whatever the root's own required accessor is) is just the identity
            // function here. This is what lets every `#[class(inherits = ..)]`-managed subclass's
            // ancestor delegation reduce to a single uniform `self.base.method(..)` forward
            // (`uielement_blind_forward`), regardless of how many `base` hops it has to pass through
            // to reach the root.
            quote! {
                impl #impl_generics #ext_ident #ty_generics for #impl_name #ty_generics #where_clause {
                    fn as_ui_element(&self) -> &#impl_name { self }
                }
            },
        )
    } else {
        // Ordinary class: always declares its own `{ClassName}Ext` trait, even when `own_methods` is
        // empty (a class composed purely of `#[inherent]` helpers over an `inherits = ..` base, e.g.
        // a thin DSL-facing wrapper) — an empty trait/impl pair is harmless, and generating it
        // unconditionally means naming alone never has to signal "skip trait generation" (that used
        // to be spelled by writing the class's own name already `Impl`-suffixed; now the struct is
        // always just its own bare name, so that signal doesn't exist any more — `struct_only` is the
        // real, explicit way to opt out of an own trait).
        // The immediate ancestor's own supertrait bound is only added when that ancestor is
        // `forwardable` — the same condition hop-0 ancestor-impl generation above uses, and for the
        // same reason: a `no_ancestor_forward` `struct_only` ancestor's target trait has real
        // required methods this class doesn't implement, so bounding against it would be as invalid
        // as blindly `impl`-ing it.
        let bound = args.inherits.as_ref().and_then(|t| {
            let name = last_segment_name(t)?;
            (name == "UIElement" || ancestor_forwardable(&name)).then(|| {
                let own_trait_ty = ancestor_own_trait(&name, t);
                quote! { : #own_trait_ty }
            })
        });
        // Every own method becomes a *default* trait method, dispatching through the required
        // `__dyn_x` accessor (`build_dyn_default_methods`) — mirroring root mode's own
        // `as_ui_element`-based default-method pattern (see this function's own doc comment on root
        // mode), generalized so any ordinary class's descendants inherit its methods for free without
        // needing to know their names/signatures. `ClassName` itself, the declaring class, still
        // overrides every one of them with its real body below — relying on the default there would
        // recurse forever (its own `__dyn_x` is reflexive).
        let dyn_ident = dyn_accessor_ident(&class_name.to_string());
        let own_sigs: Vec<syn::Signature> = own_methods.iter().map(|f| f.sig.clone()).collect();
        let ext_ty = quote! { #ext_ident #ty_generics };
        let dyn_methods = build_dyn_default_methods(&dyn_ident, &ext_ty, &own_sigs);
        let bodies = own_methods.iter().map(|f| quote! { #f });
        (
            quote! { pub trait #ext_ident #impl_generics #bound #where_clause { #(#dyn_methods)* } },
            quote! {
                impl #impl_generics #ext_ident #ty_generics for #impl_name #ty_generics #where_clause {
                    fn #dyn_ident(&self) -> &dyn #ext_ty { self }
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

    // `construct` (a bare-value initializer, typically calling `Base::construct(..)` for its own
    // `base` field when `inherits = ..`) opts a class into an *auto-generated* `new` — see this
    // function's own doc comment. A class not using this convention (a hand-written `new` returning
    // `Rc<Self>` directly, the pre-existing form) is entirely unaffected. If the user hand-writes
    // *both* `construct` and `new` in the same block, the hand-written `new` simply wins (no
    // auto-generation, no error) — a legitimate shape for a class whose `new` needs to do real work
    // beyond `Rc::new(Self::construct(..))` itself (e.g. `ContentControl::new`'s post-construction
    // parent-pointer rewiring), while `construct` still exists for other classes to call directly
    // when they only need the bare, unwrapped value. `abstract_class` (e.g. `Layout`) never gets an
    // auto-generated `new` either way — an abstract class is never meant to be directly, publicly
    // instantiated as its own `Rc<Self>` — but it can still define `construct` (and only
    // `construct`) purely so its own concrete subclasses (`VerticalLayout`/`HorizontalLayout`) have
    // something mechanical to call for their own `base` field, instead of re-building `Layout { .. }`
    // by hand in each one.
    let has_hand_written_new = ctor_methods.iter().any(|(f, _)| f.sig.ident == "new");
    let construct_fn = if has_hand_written_new || args.abstract_class {
        None
    } else {
        ctor_methods.iter().find(|(f, _)| f.sig.ident == "construct").map(|(f, _)| f.clone())
    };
    let auto_new = construct_fn.as_ref().map(|f| {
        let params = &f.sig.inputs;
        let arg_names: Vec<TokenStream2> = params
            .iter()
            .filter_map(|arg| match arg {
                FnArg::Typed(pat_type) => {
                    let pat = &pat_type.pat;
                    Some(quote! { #pat })
                }
                FnArg::Receiver(_) => None,
            })
            .collect();
        quote! {
            pub fn new(#params) -> std::rc::Rc<Self> {
                std::rc::Rc::new(Self::construct(#(#arg_names),*))
            }
        }
    });

    let mut ctor_block = TokenStream2::new();
    if !ctor_methods.is_empty() || !accessor_methods.is_empty() || auto_new.is_some() {
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
                #auto_new
                #(#accessor_methods)*
            }
        };
    }

    let out = quote! {
        #(#overridable_marker_decls)*
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

/// Builds `impl elwindui_core::ui::UIElementExt for #impl_name #ty_generics { .. }` for the blind-
/// delegate case: each of the four methods either comes from `user_methods` (an explicit override)
/// or falls back to a hardcoded `self.base.method(..)` forward.
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
        fn as_ui_element(&self) -> &#core::ui::UIElement { self.base.as_ui_element() }
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
        impl #impl_generics #core::ui::UIElementExt for #impl_name #ty_generics #where_clause {
            #as_ui_element
            #measure_override
            #arrange_override
            #try_as_native_control
            #visual_children
            #paint
        }
    }
}
