//! `#[elwindui_macros::class]` — automates the H.2.1a class-hierarchy convention
//! (docs/design/runtime/ui_tree_design.md): `struct ClassName { base: SuperClassName, .. }` implementing
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
//! ## Ancestor resolution: the `__elwindui_inherit_*!` mechanism
//!
//! Every `inherits = Parent` class's ancestor handling — from its immediate parent all the way to
//! the root, any number of hops, regardless of which crate each ancestor lives in — reduces to a
//! single generated line: `<path-to-Parent's-crate>::__elwindui_inherit_Parent!(Self; <overrides>);`
//! (`inherit_macro_prefix`/`build_inherit_invocation`). There is no per-hop special-casing in this
//! module at all: hop-0 and hop-N are handled by the exact same code path.
//!
//! This works because `Parent`'s own `struct`/`trait_only` expansion additionally emits a
//! `#[doc(hidden)] #[macro_export] macro_rules!` trio (`build_inherit_macros`) that:
//! - generates `impl ParentExt for $SubType { fn __dyn_parent(&self) -> &dyn ParentExt {
//!   self.base.__dyn_parent() } }` plus (when `Parent` has a concrete backing type, i.e. isn't
//!   `trait_only`) `impl $SubType { pub fn as_parent(&self) -> &Parent { self.base.as_parent() } }`;
//! - recurses into `Parent`'s own further ancestor's `__elwindui_inherit_*!` (or, for a root class,
//!   a terminal macro) to continue the chain;
//! - accepts a flat list of `#[overrides]`-tagged method items from the caller and, via a tt-muncher
//!   (`__elwindui_inherit_Parent_classify!`), splices in exactly the ones matching one of `Parent`'s
//!   own `#[overridable]`-declared method names, forwarding everything else untouched to the next
//!   layer of recursion. No caller ever has to say *which* ancestor a method belongs to — the
//!   method's own name is the only key needed, since `#[overridable]` is always declared at the one
//!   specific class that owns that method.
//!
//! `self.base.__dyn_parent()`/`self.base.as_parent()` are correct at *any* hop depth, not just
//! hop-0, because `Parent` itself (the declaring class) implements both reflexively (`{ self }`) —
//! so whether `self.base` *is* `Parent` or is some closer ancestor that itself already pulled in
//! `Parent`'s own accessor the same way, the call resolves to the same underlying value either way.
//!
//! A `struct_only` class that composes an `inherits = ..` target implementing the *exact same*
//! trait it itself implements (a "wrapper of wrapper" — a DSL-facing class re-wrapping a same-crate
//! native leaf that already directly implements the trait being wrapped) would otherwise collide
//! with the auto-generated `impl` (E0119); such a class instead calls the target's
//! `__elwindui_inherit_Parent_skip!` entry point, which skips generating that one `impl` but still
//! recurses for anything beyond it. Detecting this narrow, same-crate-only case is the one thing
//! this module still tracks in a process-global map (`same_crate_classes`) — unlike the old
//! per-hop ancestor registry this replaces, it is never consulted to walk a chain, only to decide
//! (a) whether an `inherits = ..` target's macros live in *this* crate (bare/`$crate::` — a
//! macro-expanded `#[macro_export]` macro can't be referred to via an absolute path from within its
//! own defining crate) or must be reached through the target's own stated path prefix
//! (`path_module_prefix`, reusing exactly the module route this class already names the target
//! *type* through — see `inherit_macro_prefix`), and (b) whether that one collision applies. Both
//! answers are always correct from same-crate information alone, so no manually-maintained
//! cross-crate fact table (of the kind this mechanism replaces) is needed for either purpose.
//!
//! `#[overridable]` (declared on a class's own method) and `#[overrides]` (declared on a
//! descendant's override, at any hop depth, no other tag or argument needed) are the only override
//! vocabulary — there is no separate "which ancestor" attribute. An `#[overrides]` method that never
//! matches any ancestor's `#[overridable]` declaration fails loudly at the root of the chain (each
//! "no inherits" class's own local `__elwindui_inherit_{Name}_terminal!` check's `compile_error!`
//! — see `build_inherit_macros`'s own doc comment on why this is generated per-class rather than
//! shared), and a real signature mismatch fails via ordinary trait-impl type checking (E0050/
//! E0053), exactly like any other trait override.
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
//! ancestor) never wants. A root class still participates in the `__elwindui_inherit_*!` mechanism
//! above like any other class — its own methods marked `#[overridable]` work the same way for its
//! descendants — the only difference is that its own generated macro trio recurses into the fixed
//! terminal macro instead of a further ancestor's.
//!
//! Field-driven `Cell`/`RefCell` accessor generation is likewise not implemented — those getters/
//! setters stay hand-written in `impl ClassName { .. }`, exactly as today; they are typically one
//! line each and the real win here is eliminating the `ClassNameExt` trait declaration and the
//! ancestor delegation boilerplate.
//!
//! A `&self`-free method literally named `construct` (any signature, returning `Self`) is
//! **mandatory** on every `#[class]`-tagged class — a class with zero or more than one qualifying
//! `construct` is a compile error, and `#[class]` never synthesizes a default one. `construct`'s own
//! body is hand-written: for an `inherits = Base` class it typically builds its own `base` field by
//! calling `Base::construct(..)`, recursively, all the way to the root.
//!
//! `construct` opts every class into an *auto-generated* `new`, built on `std::rc::Rc::new_cyclic`
//! so `construct`'s own body can reference the special identifier `__self_weak` — a weak reference to
//! the final, most-derived object being constructed — and so any `fn on_constructed(&self)` the class
//! defines runs automatically, exactly once, right after the `Rc` exists, base-class-first across the
//! whole `inherits` chain. Hand-writing `new` is a compile error — any post-construction work a class
//! used to do in a hand-written `new` (parent-pointer rewiring, event wiring, ...) belongs in
//! `on_constructed` instead. `abstract_class` is the only exception: it still requires `construct`
//! (so its own concrete subclasses have something mechanical to call for their own `base` field) but
//! never gets an auto-generated `new` of its own, since it's never meant to be directly instantiated.
//!
//! `__self_weak`'s type, and the hidden parameter `construct` actually gains internally
//! (`__class_self_weak`, after `#[class]` renames the function to `__class_construct`), is always
//! *this class's own* `{ClassName}Ext` trait (or, for `struct_only`, whatever existing trait it
//! implements directly) — never a propagated "hierarchy root" type. An ancestor `construct` call
//! inside the class's own body (rewritten to call `Base::__class_construct(__class_self_weak.clone(),
//! ..)`) relies on the ordinary `{ClassName}Ext: {BaseExt}` supertrait bound already in place at that
//! call site to coerce the weak reference upward — no chain-walking or cross-crate propagation is
//! needed anywhere, even when `inherits` crosses a crate boundary.
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
//! self-contained invocation, unlike every other shape this macro supports. Since a `trait_only`
//! class has no concrete backing type of its own, its generated `__elwindui_inherit_*!` trio omits
//! the `as_<name>()` named-accessor generation (there is no concrete type to return a reference to)
//! and never *consumes* its own `inherits = ..` target's macro reflexively (there is no `self` to
//! generate an `impl` for) — it only *produces* its own macro trio, for whatever concrete class
//! later composes or inherits it.
//!
//! **`struct_only`**: declares `ClassName` a pure struct implementor of an *existing* trait — no new
//! `pub trait ClassNameExt` is generated; the given path (which must itself already be a real trait,
//! typically another class's own `..Ext`) is implemented directly for `ClassName` instead. A
//! `struct_only` class therefore has no "own trait" of its own in the usual sense — its generated
//! `__elwindui_inherit_ClassName!` trio represents the given path instead of a synthesized `..Ext`.

use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use syn::{
    Field, Fields, FnArg, Ident, ImplItem, ImplItemFn, Item, Pat, Path, Token, Type, Visibility,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
};

/// Parsed `#[class(inherits = .., struct_only = .., abstract_class, sealed, no_ancestor_forward)]`
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
    /// Only meaningful alongside `struct_only`: declares that the given trait must *not* be blindly
    /// forwarded (an empty `impl` block) when this class is later named as someone else's `inherits
    /// = ..` target — because, unlike a marker trait with zero required methods (e.g.
    /// `NativeControlExt`), it has real required methods a blind forward can't satisfy (e.g.
    /// `NativeTabView`'s `struct_only = TabView`, where `TabView` is a hand-written trait with
    /// `insert_tab`/`remove_tab`/etc. that doesn't follow this macro's `__dyn_x` convention at all).
    /// Set, this class simply does not generate an `__elwindui_inherit_ClassName!` trio — a future
    /// class naming it as an `inherits = ..` target fails immediately with "macro not found"
    /// (E0433) rather than generating an impossible blind `impl`. A subclass that needs those
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
            "no_ancestor_forward" => Ok(ClassArg::NoAncestorForward),
            other => Err(syn::Error::new(
                ident.span(),
                format!("#[class]: unknown argument `{other}`"),
            )),
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
    no_ancestor_forward: bool,
    /// See `register_same_crate_class`'s own doc comment — `true` unless this class's bare name
    /// collided with an earlier same-crate declaration.
    owns_inherit_macros: bool,
    /// The `struct ClassName`'s own declared visibility, stringified (e.g. `""`/`"pub"`/`"pub(crate)"`)
    /// — `expand_impl`'s `own_ext_alias` (Issue #128 remediation, review finding A2) caps the
    /// `pub use .. as __ElwindUIOwnExt;` re-export of a `struct_only` class's own `existing` target at
    /// this same visibility, since the actual visibility of `existing` itself (declared in a possibly
    /// different crate, unavailable to a proc-macro) is unknown here — by this codebase's own
    /// established convention a `struct_only` struct's declared visibility always matches the
    /// interface it implements (see this module's top doc comment), so this is a safe, accurate proxy
    /// rather than a guess.
    struct_vis: String,
}

/// The identifier of the crate currently being compiled, read fresh from the environment variables
/// cargo (and, critically, rust-analyzer's own proc-macro-srv — the same protocol, same per-request
/// env vars) sets for *this* macro-expansion request. Every process-global store in this module
/// (`class_arg_store`, `same_crate_classes`) is keyed by `(compiling_crate_key(), ..)`, not just
/// `..`, specifically because a real `cargo build` and rust-analyzer differ in one load-bearing way:
/// `cargo build` spawns one OS process per crate compilation, so these `OnceLock`s naturally start
/// empty for each crate; rust-analyzer instead runs *one* persistent proc-macro-srv process for the
/// entire workspace/session. Without this key, one crate's own bare class names (or its struct/impl
/// arg hand-off) leak into a *different* crate's analysis the moment both get processed within the
/// same rust-analyzer session — confirmed empirically via `rust-analyzer diagnostics .` on this
/// workspace: `notepad`'s legitimate `elwindui::ui::Window` (a cross-crate reference) was rejected
/// by `validate_fully_qualified_path` as if `Window` were declared in `notepad`'s own crate, purely
/// because `elwindui-core`'s own analysis (earlier in the same session) had already registered it.
/// `CARGO_PKG_NAME` is a fallback for the rare case `CARGO_CRATE_NAME` isn't set; empty string
/// (never a valid crate name) if neither is — degrading to one shared unscoped bucket on a missing
/// env var would silently reintroduce the exact bug this key exists to prevent.
fn compiling_crate_key() -> String {
    std::env::var("CARGO_CRATE_NAME")
        .or_else(|_| std::env::var("CARGO_PKG_NAME"))
        .unwrap_or_default()
}

/// Keyed by `(compiling_crate_key(), class name)` (e.g. `("elwindui_core", "VerticalLayout")`) —
/// populated by `struct ClassName`'s own `#[class(..)]` invocation, read by the paired
/// `impl ClassName { .. }`'s bare `#[class]` (see `expand`/`expand_impl`). Relies on the struct's
/// attribute expanding before its paired impl's within the same crate compilation — see this
/// module's own doc comment. See `compiling_crate_key`'s own doc comment for why the crate key is
/// part of this map's key at all, not just the class name.
fn class_arg_store() -> &'static Mutex<HashMap<(String, String), StoredClassArgs>> {
    static STORE: OnceLock<Mutex<HashMap<(String, String), StoredClassArgs>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn store_class_args(
    class_name: &str,
    args: &ClassArgs,
    owns_inherit_macros: bool,
    vis: &Visibility,
) {
    let stored = StoredClassArgs {
        inherits: args.inherits.as_ref().map(|t| quote! { #t }.to_string()),
        struct_only: args.struct_only.as_ref().map(|p| quote! { #p }.to_string()),
        abstract_class: args.abstract_class,
        sealed: args.sealed,
        trait_only: args.trait_only,
        no_ancestor_forward: args.no_ancestor_forward,
        owns_inherit_macros,
        struct_vis: quote! { #vis }.to_string(),
    };
    class_arg_store()
        .lock()
        .unwrap()
        .insert((compiling_crate_key(), class_name.to_string()), stored);
}

/// `None` means no `struct ClassName` has been seen yet under this name — the caller (`expand_impl`)
/// turns that into a `compile_error!` pointing at the missing/misordered struct declaration. Removes
/// the entry on success — a bare class name isn't unique crate-wide (e.g. a backend's own
/// `NativeControl` vs. an unrelated same-named class elsewhere), so consuming on read means each
/// struct/impl pair only ever collides with a same-named pair that's still "open" (pushed but not
/// yet consumed), which the struct-immediately-followed-by-its-own-impl convention this whole macro
/// depends on never produces. The second element of the returned pair is `owns_inherit_macros` —
/// see `register_same_crate_class`'s own doc comment.
fn load_class_args(class_name: &str) -> Option<(ClassArgs, bool, Visibility)> {
    let mut store = class_arg_store().lock().unwrap();
    let stored = store.remove(&(compiling_crate_key(), class_name.to_string()))?;
    let parse_type = |s: &String| {
        syn::parse_str::<Type>(s)
            .expect("#[class]: internal: failed to reparse stored `inherits` type")
    };
    let parse_path = |s: &String| {
        syn::parse_str::<Path>(s).expect("#[class]: internal: failed to reparse stored path")
    };
    let struct_vis = syn::parse_str::<Visibility>(&stored.struct_vis)
        .expect("#[class]: internal: failed to reparse stored struct visibility");
    Some((
        ClassArgs {
            inherits: stored.inherits.as_ref().map(parse_type),
            struct_only: stored.struct_only.as_ref().map(parse_path),
            abstract_class: stored.abstract_class,
            sealed: stored.sealed,
            trait_only: stored.trait_only,
            no_ancestor_forward: stored.no_ancestor_forward,
        },
        stored.owns_inherit_macros,
        struct_vis,
    ))
}

/// See `same_crate_classes`'s own doc comment.
#[derive(Clone)]
struct SameCrateClassInfo {
    /// `Some(struct_only path as a token string)` for a `struct_only` class, `None` for an
    /// ordinary/`trait_only` one.
    struct_only: Option<String>,
    /// This class's own `no_ancestor_forward` flag — see `ClassArgs::no_ancestor_forward`'s own
    /// doc comment. Needed by anyone naming this class as their own `inherits = ..` target, to
    /// know whether to skip the hop-0 supertrait bound/entry-macro invocation entirely (there is
    /// nothing to forward if this class's own `struct_only` target is a hand-written, non-`__dyn_x`
    /// trait) — same-crate-always-correct by construction, exactly like `struct_only` above (a
    /// `no_ancestor_forward` class's hand-written trait and whatever composes it are always
    /// declared together in the same backend crate).
    no_ancestor_forward: bool,
    /// Issue #128: `true` only for a `trait_only` registration, `false` for an ordinary/root/
    /// `struct_only` one. Needed because a `struct_only` implementor's own struct declaration
    /// registers *itself* under its own bare name too (e.g. a backend's `struct Window` registers
    /// "Window" in *its own* crate's registry) — the same bare name a `trait_only` interface of the
    /// same name uses, but declared in a *different* crate (e.g. `elwindui-core`'s `trait_only
    /// Window`). `class_interface_bridge_path`/`_self_ref_path` must not mistake "this crate has
    /// *some* class named Window" (true while compiling the backend crate, because of its own
    /// `struct_only Window`) for "this crate has the *trait_only* Window interface" (never true
    /// there) — only this flag distinguishes the two.
    is_trait_only: bool,
}

/// Same-crate-only bookkeeping — the *only* process-global state this module keeps about other
/// classes' declarations, and never consulted to walk an ancestor chain (see this module's own doc
/// comment on the `__elwindui_inherit_*!` mechanism, which needs no such walk at all). Keyed by
/// class bare name. Used for exactly three same-crate-always-correct questions:
/// 1. `inherit_macro_prefix`: is an `inherits = ..` target's macro trio declared in this same
///    crate (bare/`$crate::` reference) or somewhere else (reached via that target's own stated
///    path prefix, `path_module_prefix`)? This is a complete, always-correct binary choice — not a
///    partial/fallback-prone lookup the way the old per-hop registry was — because the "somewhere
///    else" case never needs this module to know *which* crate that is: the class's own
///    `inherits =`/`struct_only =` argument already names a fully-qualified route to the target,
///    and a `#[macro_export]` macro is always reachable through that exact same route (crate root
///    *or* any module that re-exports it, transitively) as the target type itself.
/// 2. `struct_only_collides_with`: does a `struct_only` class's own declared trait happen to be the
///    *exact same* trait its `inherits = ..` target already implements via its own `struct_only`
///    (the "wrapper of wrapper" pattern described in this module's own doc comment)? This only ever
///    needs to be true for same-crate declarations by construction (a DSL-facing wrapper and the
///    raw native leaf it composes are always declared together in the same backend crate).
/// 3. `ancestor_is_forwardable`: is an `inherits = ..` target's own trait even forwardable at all
///    (`SameCrateClassInfo::no_ancestor_forward`, above)?
/// Keyed by `(compiling_crate_key(), bare name)`, not just bare name — see `compiling_crate_key`'s
/// own doc comment for why (rust-analyzer's single persistent proc-macro-srv process, unlike a real
/// `cargo build`'s one-process-per-crate model, would otherwise leak one crate's registrations into
/// every other crate's analysis within the same session).
fn same_crate_classes() -> &'static Mutex<HashMap<(String, String), SameCrateClassInfo>> {
    static REGISTRY: OnceLock<Mutex<HashMap<(String, String), SameCrateClassInfo>>> =
        OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Validates that an `inherits = ..`/`struct_only = ..` argument (`path`, the target's own type
/// path exactly as written) is a fully crate-root-qualified path, never a bare name or a
/// same-crate local alias — required because this path's tokens get embedded verbatim into this
/// class's generated `__elwindui_inherit_*!` macro body (`own_trait_ty`/`concrete_ty` in
/// `expand_impl`), which may later be expanded from a completely different module or crate than
/// the one `path` was written in — a bare or aliased reference only resolves in the writer's own
/// scope, not wherever the macro chain eventually expands (confirmed by testing: `macro_rules!`
/// path resolution for types, same as for macro names, uses call-site scope, not def-site scope).
/// `arg_name` is `"inherits"` or `"struct_only"`, used only for the error message.
fn validate_fully_qualified_path(path: &Path, arg_name: &str) -> syn::Result<()> {
    let Some(last) = path.segments.last() else {
        return Ok(());
    };
    let bare_name = last.ident.to_string();
    if path.segments.len() < 2 {
        let msg = format!(
            "#[class]: `{arg_name} = {bare_name}` must be written as a fully crate-root-qualified path \
             (e.g. `crate::ui::{bare_name}` or `some_crate::ui::{bare_name}`), never a bare name — this \
             class's generated `__elwindui_inherit_*!` macro chain may be expanded from a different \
             module than this one, where a bare name wouldn't resolve"
        );
        return Err(syn::Error::new_spanned(path, msg));
    }
    if same_crate_classes()
        .lock()
        .unwrap()
        .contains_key(&(compiling_crate_key(), bare_name.clone()))
    {
        let first = &path.segments.first().unwrap().ident;
        if first != "crate" {
            let msg = format!(
                "#[class]: `{arg_name} = {first}::..::{bare_name}` names a class declared in this same \
                 crate, but doesn't start with `crate::` — a local alias like `{first}::` (e.g. `use \
                 crate as {first};`) doesn't resolve once embedded in this class's generated \
                 `__elwindui_inherit_*!` macro chain and expanded from a different module. Write the \
                 full path starting with `crate::` instead."
            );
            return Err(syn::Error::new_spanned(path, msg));
        }
    }
    Ok(())
}

/// Returns `true` when this is the *first* same-crate declaration seen under `bare_name` (the
/// "canonical owner" for that bare name's `__elwindui_inherit_*!`/`__elwindui_check_not_sealed_*!`
/// trio — see `owns_inherit_macros`'s own doc comment), `false` for a later, same-crate-name-
/// sharing declaration (e.g. a DSL-facing wrapper sharing its bare name with the raw native leaf
/// it composes, such as `builtins::MenuItem` alongside this same crate's own `MenuItem` — a
/// deliberately supported naming pattern, see this module's own top doc comment on `struct_only`).
fn register_same_crate_class(
    bare_name: &str,
    struct_only: &Option<Path>,
    no_ancestor_forward: bool,
    is_trait_only: bool,
) -> bool {
    let struct_only = struct_only.as_ref().map(|p| quote! { #p }.to_string());
    let key = (compiling_crate_key(), bare_name.to_string());
    let mut registry = same_crate_classes().lock().unwrap();
    if registry.contains_key(&key) {
        false
    } else {
        registry.insert(
            key,
            SameCrateClassInfo {
                struct_only,
                no_ancestor_forward,
                is_trait_only,
            },
        );
        true
    }
}

/// See `same_crate_classes`'s own doc comment (question 3). Assumes forwardable (`false`) on a
/// registry miss — either a genuinely cross-crate target (where `no_ancestor_forward` classes are
/// never used in practice, being an internal implementation detail of the one backend crate that
/// composes them) or a not-yet-registered same-crate one (declaration order puts ancestors first,
/// same as every other same-crate lookup in this module).
fn ancestor_is_no_ancestor_forward(bare_name: &str) -> bool {
    same_crate_classes()
        .lock()
        .unwrap()
        .get(&(compiling_crate_key(), bare_name.to_string()))
        .is_some_and(|info| info.no_ancestor_forward)
}

/// A path type's own leading segments, everything but the final (bare-name) segment, followed by
/// that class's `macro_reexport_mod_ident` wrapper module — e.g. `elwindui::ui::Window` ->
/// `elwindui::ui::__elwindui_macros_of_Window`. That wrapper module (see its own doc comment) is
/// exactly where `expand_impl`/`expand_trait_only` `pub use`-re-export a class's macro trio, right
/// alongside the class itself — so reusing `ty`'s own leading segments here reaches it through
/// whatever chain of `pub use`/`pub mod` re-exports (including a merged/multi-crate one like the
/// `elwindui` facade's `pub mod ui { pub use elwindui_core::ui::*; pub use
/// elwindui_backend_appkit::builtins::*; }`) makes that type path valid in the first place — no
/// separate crate-alias bookkeeping needed. `#[class]` requires every `inherits =`/`struct_only =`
/// argument to be written fully-qualified (`validate_fully_qualified_path`) specifically so this is
/// always available. `None` for a single-segment path (already rejected by validation) or a
/// non-path type.
fn path_module_prefix(ty: &Type) -> Option<TokenStream2> {
    let Type::Path(tp) = ty else { return None };
    if tp.path.segments.len() < 2 {
        return None;
    }
    let leading_colon = &tp.path.leading_colon;
    let prefix_idents = tp
        .path
        .segments
        .iter()
        .rev()
        .skip(1)
        .rev()
        .map(|s| &s.ident);
    let bare_name = tp.path.segments.last()?.ident.to_string();
    let mod_ident = macro_reexport_mod_ident(&bare_name);
    Some(quote! { #leading_colon #(#prefix_idents)::* :: #mod_ident })
}

/// The macro-invocation path for one of a class's `__elwindui_inherit_*!`/
/// `__elwindui_check_not_sealed_*!` trio (see `same_crate_classes`'s own doc comment, question 1)
/// — `None` (bare macro name, no prefix at all) when the target was declared in the crate
/// currently compiling (a genuine same-crate declaration), `Some(path_module_prefix(ty))`
/// otherwise — see that function's own doc comment for why this is always correct.
fn inherit_macro_prefix(bare_name: &str, ty: &Type) -> Option<TokenStream2> {
    if same_crate_classes()
        .lock()
        .unwrap()
        .contains_key(&(compiling_crate_key(), bare_name.to_string()))
    {
        return None;
    }
    path_module_prefix(ty)
}

/// Combines `inherit_macro_prefix` with a macro identifier into the full invocation path — the
/// *direct*-invocation form, for item-position generated code (`expand_impl`'s own `prelude`
/// construction) that calls a target class's macro trio directly. Never use this *inside* another
/// `macro_rules!` body — see `inherit_macro_self_ref_path`. `ty` is this class's own `inherits =`/
/// `struct_only =` argument naming the target (see `inherit_macro_prefix`'s doc comment on why).
///
/// The same-crate case emits `crate::#ident`, not a bare `#ident`. Both compile under rustc, but a
/// bare name resolves purely by textual `macro_rules!` scope — which only reaches from the macro's
/// definition point through the rest of *that file*. The moment a class moves into a submodule
/// (`native_ui/text.rs` inheriting `crate::NativeControl` declared in `native_ui/mod.rs`), rustc
/// still accepts it — `#[macro_use]` on the sibling module makes it work — but **rust-analyzer
/// reports `unresolved macro __elwindui_inherit_NativeControl!`** on every such class, a
/// whole-crate IDE breakage invisible to `cargo build` and `cargo test`. `#[macro_export]` already
/// places the macro at the defining crate's root, so an absolute `crate::` path is reachable from
/// any module in that crate and both engines resolve it identically. (This is what
/// `#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]` in each `#[class]`-using
/// crate's root is for.)
///
/// Note this must *not* be pushed down into `inherit_macro_prefix`: that helper is shared with
/// `inherit_macro_self_ref_path`, where the same-crate case has to stay `$crate::`. A literal
/// `crate::` inside a `macro_rules!` body resolves against the *invoking* crate, so making the
/// prefix itself `crate` breaks every cross-crate consumer (`notepad`'s generated components fail
/// with `cannot find __elwindui_inherit_Control in crate`).
fn inherit_macro_path(bare_name: &str, ty: &Type, ident: Ident) -> TokenStream2 {
    match inherit_macro_prefix(bare_name, ty) {
        Some(prefix) => quote! { #prefix::#ident },
        None => quote! { crate::#ident },
    }
}

/// The *self-reference* form of `inherit_macro_path` — for a macro-to-macro reference written
/// *inside* one of `build_inherit_macros`'s own generated `macro_rules!` bodies (`skip_ident`'s
/// forward, `classify_ident`'s recursive calls and base-case recursion). Unlike direct
/// item-position code, a same-crate bare reference here is wrong: the macro body it's embedded in
/// may itself be invoked from a *different* crate than the one it was defined in (e.g. a backend
/// crate invoking an `elwindui-core`-defined class's inherit-macro chain), and a bare name inside
/// a `macro_rules!` body resolves against the *invoking* crate's scope, not the defining one —
/// `$crate` is `macro_rules!`'s own mechanism for "always resolve against the crate that defined
/// this macro, regardless of who calls it", which is exactly the same-crate case's fix here.
fn inherit_macro_self_ref_path(bare_name: &str, ty: &Type, ident: Ident) -> TokenStream2 {
    match inherit_macro_prefix(bare_name, ty) {
        Some(prefix) => quote! { #prefix::#ident },
        None => quote! { $crate::#ident },
    }
}

/// Issue #128: `<ClassName>Ext` -> `ClassName`, the direction `to_ext_ident`/`ext_trait_type` never
/// go — derives the class name a forwardable `struct_only = some::path::NameExt` argument implements,
/// so `expand_impl`'s struct_only branch can reach that class's own `__elwindui_class_interface_*!`
/// bridge (`class_interface_bridge_path`) without needing a manually-maintained name table. Requires
/// the path's final segment to literally end in `Ext` with a non-empty prefix — a `#[class]`-generated
/// interface always does (`to_ext_ident`); a hand-written trait that doesn't follow this convention
/// must use `struct_only = .., no_ancestor_forward` instead (this function is never called for that
/// case — see `expand_impl`'s own struct_only branch).
fn class_name_from_ext_trait_path(path: &Path) -> syn::Result<Ident> {
    let Some(last) = path.segments.last() else {
        return Err(syn::Error::new_spanned(
            path,
            "#[class]: empty struct_only path",
        ));
    };
    let name = last.ident.to_string();
    match name.strip_suffix("Ext").filter(|s| !s.is_empty()) {
        Some(stripped) => Ok(Ident::new(stripped, last.ident.span())),
        None => {
            let msg = format!(
                "#[class]: `struct_only = ..{name}` does not name a `#[class]`-generated `*Ext` \
                 interface (a class's own derived trait is always `{{ClassName}}Ext`) — normal \
                 `struct_only` forwarding targets exactly that convention so this class's descendants \
                 can use ordinary `#[overrides]` through it; a hand-written trait that doesn't follow \
                 it must add `no_ancestor_forward` instead (`struct_only = ..{name}, \
                 no_ancestor_forward`), which opts out of override-slot forwarding entirely"
            );
            Err(syn::Error::new_spanned(last, msg))
        }
    }
}

/// `Window` -> `__elwindui_class_interface_Window`: the Issue #128 class-interface bridge macro
/// every class-managed interface (ordinary class, root class, `trait_only`) generates alongside its
/// `__elwindui_inherit_*!` trio — see `build_class_interface_bridge_macro`'s own doc comment.
fn class_interface_bridge_ident(bare_name: &str) -> Ident {
    format_ident!("__elwindui_class_interface_{bare_name}")
}

/// PR #164 final remediation round (finding A5): `some::path::RootExt` -> `some::path::Root` — the
/// exact class path (not just the bare name) a `struct_only = ..Ext` argument's own target class
/// would be reached at, reusing `class_name_from_ext_trait_path`'s "strip exactly one trailing
/// `Ext`" rule. Used only to compare against an `inherits = ..` argument's own literal path text
/// (`struct_only_matches_direct_parent`) — never to actually resolve or validate that such a class
/// exists (this proc-macro has no way to check that).
fn struct_only_class_path(existing: &Path) -> syn::Result<TokenStream2> {
    let class_name = class_name_from_ext_trait_path(existing)?;
    let leading_colon = &existing.leading_colon;
    let prefix_idents = existing
        .segments
        .iter()
        .rev()
        .skip(1)
        .rev()
        .map(|s| &s.ident);
    Ok(quote! { #leading_colon #(#prefix_idents)::* :: #class_name })
}

/// PR #164 final remediation round (finding C2, §4.6/§4.8): whether `struct_only`'s own derived
/// class path (`struct_only_class_path`) is *exactly* the same path as `inherits`'s own literal
/// text — the general "this `struct_only` class's own explicit target is precisely its own direct
/// `inherits = ..` parent's own generated interface" condition. Purely syntactic (stringified token
/// comparison of two paths *this class itself* already wrote in its own attribute — no registry, no
/// cross-crate ambiguity of any kind, unlike the generic-arity questions elsewhere in this module).
///
/// Two independent, unrelated consumers:
/// - `expand_impl`'s prelude construction: when true, this class must not *also* pick up a second,
///   separately-generated `impl <parent's own trait> for <this class>` through the ordinary
///   `inherits = ..` forwarding path (that would collide, `E0119`, with the explicit `impl #existing
///   for #impl_name { .. }` this class's own `struct_only` branch already builds) — route through
///   the parent's `_skip!` entry point instead, exactly like `struct_only_collides_with`'s own
///   established (narrower: struct_only-target-equals-struct_only-target) case already does.
/// - The class-interface bridge's own `@impl_struct_only_members` arm (`has_matching_base`
///   parameter): a *root*-mode target's required `as_ui_element` forwarding member can only be
///   generated when this is true (see `build_class_interface_bridge_macro`'s own doc comment) — a
///   friendly compile-time diagnostic fires instead of a raw `E0609`/`E0053` when it's false.
fn struct_only_matches_direct_parent(struct_only: &Option<Path>, inherits: &Option<Type>) -> bool {
    let (Some(existing), Some(inh)) = (struct_only, inherits) else {
        return false;
    };
    let Ok(derived) = struct_only_class_path(existing) else {
        return false;
    };
    derived.to_string() == quote! { #inh }.to_string()
}

/// PR #164 final remediation round (finding A5): the wrapper "bound" trait every non-
/// `no_ancestor_forward`, non-`sealed` class-managed ancestor (ordinary, root, forwardable
/// `struct_only`) generates alongside `__ElwindUIOwnExt`, reached the same way through its own
/// wrapper module (`ancestor_own_trait_bound`). Unlike `__ElwindUIOwnExt` (a bare `pub use` alias
/// whose own generic arity is fixed at generation time — 0 for a `struct_only` target, matching the
/// ancestor struct's own arity for ordinary/root — and therefore ambiguous to a cross-crate caller
/// with no reliable same-crate registry to consult), `__ElwindUIOwnExtBound<..>` is declared with
/// *exactly* the same generic parameter list as the ancestor's own struct declaration
/// (`impl_generics`), with a supertrait bound to the ancestor's real implemented trait (its own
/// generated `{ClassName}Ext` for ordinary/root, or its `struct_only = ..` target for `struct_only`)
/// and a blanket impl for the ancestor struct itself.
///
/// A descendant referencing `<ancestor's wrapper>::__ElwindUIOwnExtBound<exactly the trailing
/// generic arguments its own `inherits = Ancestor<Args>` argument already carries>` therefore never
/// needs to guess whether those arguments apply to the real trait — the arity is always correct by
/// construction (Rust requires `Args` to already match the ancestor struct's own parameter count,
/// regardless of what `Args` turns out to mean), and the *ancestor itself*, at its own compile time
/// (the one place that genuinely knows whether it is ordinary/root or `struct_only`), decides what
/// the supertrait bound actually resolves to. Both the wrapper trait and the blanket impl are
/// trivially satisfiable: the ancestor struct already implements the real target trait itself
/// (either its own generated `impl {ClassName}Ext for AncestorStruct { .. }`, or — for `struct_only`
/// — the explicit `impl #existing for AncestorStruct { .. }` this remediation's own A3 fix already
/// builds), independent of the wrapper.
///
/// This exists specifically because Rust does not permit a macro invocation in supertrait-bound
/// position (confirmed: `trait X: some_macro!() {}` does not parse) — the macro-expansion-time
/// "apply-or-ignore" policy this remediation otherwise uses for *dispatch routing*
/// (`own_ext_policy_ident`, `own_ext_policy_path`) cannot be used for a supertrait bound, which is
/// resolved at this class's own proc-macro time and therefore needs a genuinely proc-macro-time
/// answer — this wrapper trait is that answer, and (unlike the same-crate-registry heuristic it
/// replaces at this one call site, `expand_impl`'s ordinary-class `bound` computation) is correct
/// cross-crate with no registry involved at all.
/// Parameterized by `bare_name` (unlike `own_ext_alias_ident`'s fixed `__ElwindUIOwnExt`): unlike a
/// `pub use .. as X;` alias (scoped to its own, per-class wrapper module, so a fixed name never
/// collides across different classes), this is a genuine `pub trait`/`impl` pair declared directly
/// alongside the ancestor's own struct/trait declaration, in the *same* enclosing module — a fixed
/// name here would collide (`E0428`) the moment two different classes share that module (confirmed
/// empirically against every real class declared in `testsupport.rs`).
fn own_ext_bound_trait_ident(bare_name: &str) -> Ident {
    format_ident!("__ElwindUIOwnExtBound_{bare_name}")
}

/// See `own_ext_bound_trait_ident`'s own doc comment. `fallback`'s own trailing generic arguments
/// are *always* reattached, unconditionally — no same-crate/cross-crate ambiguity at all, since the
/// wrapper bound trait's own arity always matches `fallback`'s by construction.
fn ancestor_own_trait_bound(bare_name: &str, fallback: &Type) -> TokenStream2 {
    let prefix = path_module_prefix(fallback).expect(
        "#[class]: internal: inherits/struct_only argument should already be validated fully-qualified",
    );
    let bound_ident = own_ext_bound_trait_ident(bare_name);
    let generic_args = last_segment_generic_args(fallback);
    quote! { #prefix :: #bound_ident #generic_args }
}

/// Builds the *single* blanket `impl` that makes `own_ext_bound_trait_ident`'s wrapper trait
/// satisfied by every type implementing `target_trait` — not just the ancestor's own declaring
/// struct (`#impl_name`) itself. This is required, not merely convenient: a descendant several hops
/// below the ancestor (e.g. `Shape`/`Control`/`TextBlock`, all implementing `UIElementExt` without
/// literally being `UIElement`) must *also* satisfy `__ElwindUIOwnExtBound_UIElement` the moment
/// *their own* descendants reference it — confirmed empirically (`E0277`, unsatisfied trait bound,
/// against every real multi-hop `UIElement` descendant) when this was first written as a narrow
/// `impl .. for #impl_name` instead. One blanket rule, generated once by the ancestor's own
/// `expand_impl`/`expand_trait_only` run, covers every current *and future* implementor, with the
/// ancestor's own generic parameters (`base_generics`, e.g. `<H>` for `NativeControl<H>`) threaded
/// through as extra, independent generic parameters alongside the new blanket type parameter — e.g.
/// `impl<H, T: NativeControlExt<H>> __ElwindUIOwnExtBound_NativeControl<H> for T {}`.
fn own_ext_bound_blanket_impl(
    base_generics: &syn::Generics,
    bound_trait_ident: &Ident,
    ty_generics: &TokenStream2,
    target_trait: &TokenStream2,
) -> TokenStream2 {
    let mut generics = base_generics.clone();
    let blanket_ty_ident = format_ident!("__ElwindUIOwnExtBoundT");
    generics
        .params
        .push(syn::GenericParam::Type(syn::TypeParam {
            attrs: Vec::new(),
            ident: blanket_ty_ident.clone(),
            colon_token: None,
            bounds: Punctuated::new(),
            eq_token: None,
            default: None,
        }));
    let predicate: syn::WherePredicate = syn::parse_quote! { #blanket_ty_ident : #target_trait };
    generics.make_where_clause().predicates.push(predicate);
    let (impl_generics, _, where_clause) = generics.split_for_impl();
    quote! {
        impl #impl_generics #bound_trait_ident #ty_generics for #blanket_ty_ident #where_clause {}
    }
}

/// PR #164 final remediation round (finding A5): the stable, per-ancestor "own-ext application
/// policy" macro every non-`no_ancestor_forward`, non-`sealed` class-managed ancestor (ordinary,
/// root, forwardable `struct_only`) generates alongside `__ElwindUIOwnExt`/`__ElwindUIOwnExtBound` —
/// decides, at *macro-expansion* time (not proc-macro time, and so correctly cross-crate with no
/// registry lookup of any kind), whether a descendant's own trailing `inherits = Ancestor<Args>`
/// generic arguments should be applied when routing dispatch — the actual `impl <ancestor's real
/// trait> for <descendant>` `build_inherit_macros` generates. `Args` apply for an ordinary/root
/// ancestor (whose own generated `{ClassName}Ext` genuinely shares those parameters); never for a
/// forwardable `struct_only` ancestor (whose own `struct_only = ..` target is always a separately,
/// independently-declared, non-generic interface — see `class_name_from_ext_trait_path`'s own
/// validation, which requires a plain path with no generic arguments of its own).
///
/// Invocation protocol (see `own_ext_policy_path`'s own doc comment for the exact call shape): the
/// caller (a descendant naming this ancestor via `inherits = ..`) first defines a *fresh*,
/// uniquely-named, `#[macro_export]`'d continuation macro at its own call site — one whose single
/// arm is `([$($OwnTrait:tt)*]) => { <splice $($OwnTrait)* into the one place the resolved trait
/// belongs, e.g. the entry-macro invocation this whole thing is routing into> };` — then invokes
/// `<ancestor's wrapper>::__elwindui_apply_own_ext_<bare_name>!($cont; [$($concrete_args:tt)*]);`
/// with that continuation's own path and *only* the trailing generic-argument tokens from its own
/// `inherits = Ancestor<Args>` (never a full path). The ancestor's own policy macro (generated once,
/// baked to exactly one of the two bodies below at the ancestor's own compile time) is:
///
/// - ordinary/root: `$cont!( [ $($args)* ] );` — echoes the caller's own concrete generic arguments
///   straight back, unexamined (this ancestor's own trait genuinely takes them).
/// - forwardable `struct_only`: `$cont!( [] );` — discards them (the real interface never takes
///   them).
///
/// The policy macro itself never needs to know or reproduce the resolved trait's own path prefix —
/// only whether to keep or drop the caller-supplied generic-argument suffix; the caller's own
/// continuation macro already has the correct, always-known-at-proc-macro-time prefix baked in
/// (`<ancestor's wrapper>::__ElwindUIOwnExt` for ordinary/root, or the `struct_only = ..` target's
/// own literal path for `struct_only`).
fn own_ext_policy_ident(bare_name: &str) -> Ident {
    format_ident!("__elwindui_apply_own_ext_{bare_name}")
}

/// Item-position path to `bare_name`'s own `__elwindui_apply_own_ext_*!` policy macro, reached
/// through the exact same already-reliable wrapper module every other cross-crate metadata for this
/// ancestor is reached through (`path_module_prefix`) — mirrors `ancestor_bridge_path` exactly. See
/// `own_ext_policy_ident`'s own doc comment for the full invocation protocol.
fn own_ext_policy_path(bare_name: &str, ty: &Type) -> TokenStream2 {
    let prefix = path_module_prefix(ty).expect(
        "#[class]: internal: inherits/struct_only argument should already be validated fully-qualified",
    );
    let ident = own_ext_policy_ident(bare_name);
    quote! { #prefix::#ident }
}

/// Issue #128 remediation (A2): the stable, class-generated alias name every class-managed
/// ancestor's own `macro_reexport_mod_ident` wrapper module re-exports its *actual* implemented Ext
/// interface under (bare, no generic arguments — a `pub use` target never carries them; the caller
/// reapplies whatever generic arguments apply, once its own `__elwindui_apply_own_ext_*!` policy
/// macro has decided whether they do — see `AncestorRoute`'s own doc comment). Never written by
/// hand in a `struct_only` declaration — see `expand_impl`'s own `macro_reexports` construction, the
/// sole place this is generated.
fn own_ext_alias_ident() -> Ident {
    format_ident!("__ElwindUIOwnExt")
}

/// `ty`'s own final path segment's generic arguments, verbatim (e.g. `<H>` from
/// `crate::NativeControl<H>`), or empty tokens if `ty` has none. Used to re-apply an ancestor's own
/// generic arguments, exactly as written by *this* class's `inherits = ..`, onto a path resolved
/// through that ancestor's wrapper module — a `use`-based alias can only ever name the bare
/// (ungenericized) item, so the instantiation has to happen at the reference site.
fn last_segment_generic_args(ty: &Type) -> TokenStream2 {
    let Type::Path(tp) = ty else {
        return TokenStream2::new();
    };
    let Some(seg) = tp.path.segments.last() else {
        return TokenStream2::new();
    };
    match &seg.arguments {
        syn::PathArguments::AngleBracketed(args) => quote! { #args },
        _ => TokenStream2::new(),
    }
}

/// Item-position path to `bare_name`'s `__elwindui_class_interface_*!` bridge macro — the direct-
/// invocation counterpart to `inherit_macro_path`, but keyed by the *derived* class name
/// (`class_name_from_ext_trait_path`) rather than a `struct_only = ..` argument's own literal last
/// path segment (which is `{ClassName}Ext`, not `ClassName`, so `path_module_prefix` itself can't be
/// reused directly here). `existing` is the fully-qualified `struct_only = ..Ext` argument naming the
/// interface. This is item-position only (used directly by `expand_impl`'s struct_only branch, never
/// spliced into another `macro_rules!` body), so — unlike `inherit_macro_path`/`_self_ref_path` —
/// there is no separate self-reference form to keep in sync.
fn class_interface_bridge_path(bare_name: &str, existing: &Path) -> TokenStream2 {
    let ident = class_interface_bridge_ident(bare_name);
    if trait_only_declared_in_this_crate(bare_name) {
        quote! { crate::#ident }
    } else {
        let leading_colon = &existing.leading_colon;
        let prefix_idents = existing
            .segments
            .iter()
            .rev()
            .skip(1)
            .rev()
            .map(|s| &s.ident);
        let mod_ident = class_interface_wrapper_mod_ident(bare_name);
        quote! { #leading_colon #(#prefix_idents)::* :: #mod_ident :: #ident }
    }
}

/// Whether `bare_name`'s `trait_only` declaration (not merely *some* same-crate-registered class of
/// that name — see `SameCrateClassInfo::is_trait_only`'s own doc comment) lives in the crate
/// currently compiling. The one same-crate question `class_interface_bridge_path` needs that
/// `same_crate_classes`'s other same-crate questions don't: a registry hit alone isn't enough here,
/// since a `struct_only` implementor's own struct declaration registers *itself* under this same
/// bare name too.
fn trait_only_declared_in_this_crate(bare_name: &str) -> bool {
    same_crate_classes()
        .lock()
        .unwrap()
        .get(&(compiling_crate_key(), bare_name.to_string()))
        .is_some_and(|info| info.is_trait_only)
}

/// Issue #128: builds `__elwindui_class_interface_<bare_name>!`, the hidden bridge macro that makes a
/// `struct_only` implementor of this class's interface a transparent link in the ordinary
/// `#[overridable]`/`#[overrides]` override chain — generated alongside the ordinary
/// `__elwindui_inherit_*!` trio by every class-managed interface (ordinary class, root class,
/// `trait_only`; never by `struct_only` itself, which only ever *consumes* one). Two arms:
///
/// - `@impl_struct_only_members $ExtTrait:path, has_matching_base = [..]` — invoked once, directly,
///   by the `struct_only` implementor's own `expand_impl` (item position, spliced *inside* the outer
///   `impl $ExtTrait for $StructTy { .. }` header the caller itself still owns and builds — review
///   finding A3, see this arm's own generated doc comment below). Emits the ordinary shared
///   `__dyn_x` accessor plus one *reflexive* per-method accessor for every `#[overridable]` slot
///   *this* interface declares (baked in here, from `overridable_names`, at this class's own compile
///   time — never re-discovered or duplicated by the `struct_only` side, satisfying the "no copied
///   slot list" invariant); the caller splices the concrete method bodies in separately, alongside
///   this invocation. `$ExtTrait` travels as a macro parameter rather than being baked as a literal
///   path here for the same reason `$OwnTrait` does in `build_inherit_macros`: this class's own
///   fully-qualified path is never something `#[class]` can determine about itself (no
///   `module_path!()`-equivalent) — the caller (`expand_impl`, which already validated `struct_only =
///   ..` fully-qualified) always has it. `has_matching_base = [..]` (review finding C2) is
///   `[$RootConcrete:path]`/`[]`, computed and supplied by the caller alone
///   (`struct_only_matches_direct_parent`) — see `build_class_interface_bridge_macro`'s own
///   `is_root_mode` parameter doc comment for what it's for and why the bracketed form (rather than
///   a bare `true`/`false`) is needed.
/// - `@inherit_through_struct_only $SubType:ty, $OwnTrait:path, $OwnConcrete:path; $($overrides:tt)*`
///   — invoked from inside the `struct_only` implementor's own generated entry-macro body (see
///   `expand_impl`'s `entry_override` construction), once per ordinary descendant reaching this
///   interface through that `struct_only` link. Forwards, unexamined, straight into *this*
///   interface's own real `__elwindui_inherit_<bare_name>!` entry macro (`$crate`-self-referenced,
///   always same-crate relative to *this* bridge's own definition, regardless of which crate the
///   `struct_only` implementor — and so this arm — is ultimately expanded from) — reusing the
///   existing recursive classify muncher exactly as an ordinary `inherits = ..` chain would, so
///   arbitrary further descendant depth (`C -> D -> E -> ..`) falls out of the *existing* recursion
///   with no depth-specific code added here.
/// PR #164 final remediation round (finding C2): `is_root_mode` marks this bridge as belonging to a
/// *root*-mode class-managed interface and extends `@impl_struct_only_members` to also require and
/// consume a `has_matching_base = [..]` argument — `[$RootConcrete:path]` (the root class's own,
/// already-fully-qualified concrete path, supplied fresh by the caller) when the `struct_only`
/// caller's own `inherits = ..` argument is *exactly* this root class itself
/// (`struct_only_matches_direct_parent`, computed by the caller, which alone knows its own attribute
/// text), `[]` otherwise. The concrete path travels as a macro parameter rather than being baked in
/// here as a literal token (tried first, reverted — see `build_class_interface_bridge_macro`'s own
/// doc comment) since this whole macro is `#[macro_export]`'d and invoked from whatever crate/module
/// declares the `struct_only` implementor, essentially never the root class's own defining module.
/// Root mode's own `as_ui_element(&self) -> &Self` is a required method whose return type is
/// hard-pinned to the declaring root struct's own concrete type — a `struct_only` implementor can
/// only ever supply this by composing the exact same root storage via `inherits = <that root
/// class>` and forwarding (`self.base.as_ui_element()`); this is precisely what the
/// `[$RootConcrete:path]` arm emits. The `[]` arm emits a clear, dedicated diagnostic instead of
/// degrading into a raw `E0609 (no field \`base\`)`/`E0053` once the missing method is discovered by
/// the compiler on its own. Every other class-managed interface (`is_root_mode: false` — `trait_only`,
/// ordinary non-root) still requires `has_matching_base = [..]` on the call site for a uniform
/// calling convention (the `struct_only` caller can never know in advance whether the interface it
/// targets happens to be root-mode), but simply ignores its contents.
fn build_class_interface_bridge_macro(
    bare_name: &str,
    dyn_ident: &Ident,
    overridable_names: &[Ident],
    is_root_mode: bool,
) -> TokenStream2 {
    let bridge_ident = class_interface_bridge_ident(bare_name);
    let entry_ident = inherit_macro_ident(bare_name);
    let per_method_accessors: Vec<TokenStream2> = overridable_names
        .iter()
        .map(|name| {
            let accessor = per_method_accessor_ident(name);
            quote! { fn #accessor(&self) -> &dyn $ExtTrait { self } }
        })
        .collect();
    // The root class's own concrete type (`&Self`-equivalent, possibly generic) can *not* be
    // baked in here as a literal token (`#impl_name #ty_generics`, tried first, reverted): this
    // whole macro is `#[macro_export]`'d and invoked from whatever crate/module declares the
    // `struct_only` implementor, which is almost never the root class's own defining module —
    // neither a bare identifier (only resolves when caller and definer coincide, e.g. same-crate
    // `testsupport.rs`) nor `$crate::<name>` (only resolves when the root class happens to live
    // at its defining crate's root, not nested in a submodule) is reliably correct, and a
    // proc-macro has no `module_path!()`-equivalent to compute the real one. Instead, the actual
    // concrete path travels as a macro parameter (`$RootConcrete`), supplied fresh by the
    // `struct_only` caller from its own already-fully-qualified `inherits = ..` argument — the
    // exact same reason `$OwnTrait`/`$OwnConcrete`/`$RootConcrete` all travel as parameters
    // elsewhere in this macro family rather than literal tokens (see `build_inherit_macros`'s own
    // doc comment). Wrapped in `[$RootConcrete:path]`/`[]` (rather than a bare optional
    // `$($RootConcrete:path)?`) so the `true`/`false` arms stay unambiguous to match on.
    let impl_struct_only_members_arms = if is_root_mode {
        quote! {
            (@impl_struct_only_members $ExtTrait:path, has_matching_base = [$RootConcrete:path]) => {
                fn #dyn_ident(&self) -> &dyn $ExtTrait { self }
                #(#per_method_accessors)*
                #[allow(dead_code)]
                fn as_ui_element(&self) -> &$RootConcrete {
                    self.base.as_ui_element()
                }
            };
            (@impl_struct_only_members $ExtTrait:path, has_matching_base = []) => {
                compile_error!(concat!(
                    "#[class]: struct_only implementing a root class interface must also use ",
                    "`inherits = <same root class>` so the required root storage is available"
                ));
            };
        }
    } else {
        quote! {
            (@impl_struct_only_members $ExtTrait:path, has_matching_base = [$($_root:path)?]) => {
                fn #dyn_ident(&self) -> &dyn $ExtTrait { self }
                #(#per_method_accessors)*
            };
        }
    };
    quote! {
        #[doc(hidden)]
        #[macro_export]
        #[allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]
        macro_rules! #bridge_ident {
            // Issue #128 remediation (review finding A3): members only, not the outer `impl`
            // header — the caller (`expand_impl`'s struct_only branch) still owns
            // `impl #impl_generics $ExtTrait for $StructTy #ty_generics #where_clause { .. }`
            // itself and splices this invocation (plus the concrete method bodies) directly
            // inside it, so a generic `struct_only` type's own `impl_generics`/`ty_generics`/
            // `where_clause` are never lost to opaque macro-token round-tripping.
            #impl_struct_only_members_arms
            (@inherit_through_struct_only $SubType:ty, $OwnTrait:path, $OwnConcrete:path, $RootConcrete:path; $($overrides:tt)*) => {
                $crate::#entry_ident!($SubType, $OwnTrait, $OwnConcrete, $RootConcrete, $OwnTrait; $($overrides)*);
            };
        }
    }
}

/// See `same_crate_classes`'s own doc comment (question 2).
fn struct_only_collides_with(own_struct_only: &Option<Path>, parent_bare_name: &str) -> bool {
    let Some(own) = own_struct_only else {
        return false;
    };
    let own_str = quote! { #own }.to_string();
    same_crate_classes()
        .lock()
        .unwrap()
        .get(&(compiling_crate_key(), parent_bare_name.to_string()))
        .and_then(|info| info.struct_only.clone())
        .is_some_and(|v| v == own_str)
}

/// Issue #128: the path to whichever ancestor `bare_name`/`ty` (an `inherits = ..` argument,
/// exactly as *this* class wrote it) names for its own class-interface-bridge macro — reached
/// through the *exact same*, already-reliable wrapper module every other macro of that ancestor is
/// reached through (`inherit_macro_path`, `macro_reexport_mod_ident`), just naming the bridge ident
/// instead of the entry/skip/classify ones. Deliberately never derived from a same-crate registry
/// lookup or naming-convention guess (same-crate declaration order across *different files* in one
/// crate is not reliably "ancestors first", unlike within one file — see `same_crate_classes`'s own
/// doc comment) — computed purely from `ty`'s own already-fully-qualified path text. Requires the
/// *ancestor itself* (e.g. `NativeControl`'s own `struct_only` declaration) to re-export the real
/// bridge macro into this same wrapper module under this same ident — see `expand_impl`'s own
/// `macro_reexports` construction.
/// Unlike `entry_ident`/`skip_ident`/`classify_ident` (always `#[macro_export]`'d directly at crate
/// root, so `inherit_macro_path`'s same-crate branch can use a bare `crate::#ident`), the bridge
/// ident is *never* at crate root — it only ever exists as a `pub use .. as` re-export *inside*
/// `bare_name`'s own wrapper module (`expand_impl`'s `macro_reexports` construction). So this always
/// routes through `path_module_prefix(ty)` (never the bare-`crate::` shortcut), regardless of
/// same-crate/cross-crate — a validated, fully-qualified `inherits = ..`/`struct_only = ..` argument
/// always has >= 2 segments, so this always succeeds. The caller decides whether the result needs
/// `rewrite_crate_segment` afterward (item position: no; self-reference/macro-body position: yes) —
/// exactly like `AncestorRoute`'s own fields already do, so a single function suffices for both.
fn ancestor_bridge_path(bare_name: &str, ty: &Type) -> TokenStream2 {
    let prefix = path_module_prefix(ty)
        .expect("#[class]: internal: inherits/struct_only argument should already be validated fully-qualified");
    let ident = class_interface_bridge_ident(bare_name);
    quote! { #prefix::#ident }
}

/// Rewrites a leading literal `crate` token to the `$crate` macro_rules metavariable. Needed
/// specifically when a type/trait path is embedded as a *literal* token stream into a macro body
/// this class generates for its *own* future descendants (`AncestorRoute`'s own fields, threaded
/// into `build_inherit_macros`) — unlike `$crate` (designed exactly for this), a bare `crate`
/// keyword's hygiene ties it to whatever crate the token was originally authored in, but once
/// spliced into another macro's generated body and reached via a chain of macro-to-macro calls that
/// ultimately gets triggered from a *third* crate, it no longer reliably resolves back to the
/// authoring crate (confirmed empirically: `elwindui-core`'s own `ContentControl`, whose `inherits =
/// crate::ui::Control` is embedded verbatim as its own trio's recursion target for descendants,
/// failed to resolve when that recursion was ultimately triggered from `notepad`, three
/// macro-invocation layers away). `crate` is only ever legally the *first* segment of an
/// already-fully-qualified path (`validate_fully_qualified_path`), so only the leading token needs
/// checking. Operates on tokens (not `syn::Type`) since the caller may already have a
/// `TokenStream2` in hand (e.g. `own_ext_policy_path`'s return value) rather than a parsed type.
fn rewrite_crate_segment(tokens: TokenStream2) -> TokenStream2 {
    let mut iter = tokens.into_iter();
    match iter.next() {
        Some(proc_macro2::TokenTree::Ident(id)) if id == "crate" => {
            let rest: TokenStream2 = iter.collect();
            quote! { $crate #rest }
        }
        Some(first) => {
            let rest: TokenStream2 = iter.collect();
            quote! { #first #rest }
        }
        None => TokenStream2::new(),
    }
}

/// The `dyn TraitExt` element type of a content-collection field's declared type — backs
/// `content_item_dyn_entry`'s `@content_item_dyn` arm. Two shapes, both already used across every
/// real content-collection property (`elwindui-core::ui`'s own `#[prop(..)]` declarations):
/// - `crate::ui::UIElementCollection` (`Layout`'s `children`, a live, heterogeneous collection) —
///   the element type is always the fixed, universal `UIElementExt`, not derived from the
///   collection's own (non-generic) type name at all.
/// - `Vec<std::rc::Rc<dyn SomePath::SomeExt>>` / `ListExt<Rc<dyn ..>>` (`TabView`'s `children`,
///   `Menu`'s `#[content(items)]` `items`) — the element type is whatever trait object the
///   generic argument names, found by walking the type's own generic arguments for a
///   `Type::TraitObject` at any nesting depth (works for either wrapper uniformly, rather than
///   text-matching each spelling separately the way `elwindui-codegen`'s own now-`#[cfg(test)]`-
///   only `dynamic_collection_item_trait` still does for the DSL text path).
///
/// `rewrite_crate_segment` on the found trait object's own bounds (not the whole `dyn Trait`
/// token, which starts with the `dyn` keyword, not `crate`) — same reasoning as
/// `routed_payload_type`'s own doc comment: this type is spliced into a macro arm invoked from
/// whichever crate eventually asks `@content_item_dyn`, so a `crate::`-relative path (as spelled
/// in the declaring class's own `#[prop(..)]`, resolving against `elwindui-core`) needs `$crate::`
/// once embedded here.
fn content_item_dyn_type(ty: &Type) -> TokenStream2 {
    if quote! { #ty }.to_string().contains("UIElementCollection") {
        return quote! { dyn $crate::ui::UIElementExt };
    }
    fn find_trait_object(ty: &Type) -> Option<&syn::TypeTraitObject> {
        match ty {
            Type::TraitObject(to) => Some(to),
            Type::Path(p) => {
                let seg = p.path.segments.last()?;
                let syn::PathArguments::AngleBracketed(args) = &seg.arguments else {
                    return None;
                };
                args.args.iter().find_map(|a| match a {
                    syn::GenericArgument::Type(t) => find_trait_object(t),
                    _ => None,
                })
            }
            _ => None,
        }
    }
    match find_trait_object(ty) {
        Some(to) => {
            let bounds = &to.bounds;
            let rewritten = rewrite_crate_segment(quote! { #bounds });
            quote! { dyn #rewritten }
        }
        // No `dyn`-wrapped element type found (shouldn't normally happen for a real dynamic-
        // child-eligible content field) — fall back to the bare declared type itself, `dyn`-
        // wrapped, so a genuinely wrong declaration still fails to compile with a specific error
        // rather than this function panicking blind.
        None => {
            let rewritten = rewrite_crate_segment(quote! { #ty });
            quote! { dyn #rewritten }
        }
    }
}

/// `elwindui_core::ui::NativeControl<AnyView>` -> `"NativeControl"`; the bare display name used for
/// naming-convention-derived trait/macro identifiers and as an `as_<snake(name)>()` accessor's
/// method name.
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
            let prev_is_upper_and_next_is_lower = i > 0
                && chars[i - 1].is_uppercase()
                && i + 1 < chars.len()
                && chars[i + 1].is_lowercase();
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
/// for referencing an ancestor's derived trait (supertrait bounds) — the rest of the path (module
/// qualifiers) passes through untouched. Used only for `Type`-position references (supertrait
/// bounds, `as_<name>()` return types) — never for the `__elwindui_inherit_*!` macro invocations
/// themselves, which need a *crate-root-only* path instead (`inherit_macro_prefix`), since
/// `#[macro_export]` macros are never reachable via a module-qualified path the way ordinary items
/// are.
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
/// declares: it returns `&dyn {Name}Ext` rather than a concrete struct type specifically so it works
/// uniformly across every class shape this macro supports — ordinary/root classes (one well-known
/// concrete struct), *and* `trait_only` marker/interface traits (no concrete struct at all in this
/// crate; each backend provides its own via `struct_only`, so there's no single type name the trait
/// declaration itself could hard-code). Every other method of `{Name}Ext` becomes a *default* method
/// with body `self.__dyn_x().method(args)` — dispatched dynamically through this accessor — so a
/// descendant only ever has to implement the accessor itself (typically one line, `&self.base` or
/// `self.base.__dyn_x()`) to inherit every one of the ancestor's real methods for free. The
/// *declaring* class itself (`ClassName`'s own `impl ClassNameExt for ClassName`) is the one
/// implementor that must override every method with its real body instead of relying on the default
/// — relying on the default there would recurse forever, since its own `__dyn_x` is reflexive
/// (`{ self }`).
///
/// Requires every `{Name}Ext` method to be dyn-compatible (object-safety rules: no generics, no `Self`
/// by value, ...) — true of every class in this codebase today (plain property-style getters/setters).
fn dyn_accessor_ident(bare_name: &str) -> Ident {
    let bare = bare_name.strip_suffix("Ext").unwrap_or(bare_name);
    format_ident!("__dyn_{}", to_snake_case(bare))
}

/// `compute` -> `__dyn_x_for_compute`: a *per-`#[overridable]`-method* dispatch accessor, one per
/// overridable method rather than one shared per trait (`dyn_accessor_ident`). See
/// `build_inherit_macros`'s own doc comment on `own_impl`/the per-method resolution mechanism for
/// why a single shared accessor is insufficient once an intermediate hop can override *some but
/// not all* of a trait's overridable methods (a real bug, found via a 3-hop test, that this fixes:
/// a default method dispatching through a *shared* accessor always lands on whichever hop is
/// reflexive for the *whole trait*, which is wrong the moment that hop only overrode a *different*
/// overridable method — it needs to land on the closest override *of that one method specifically*,
/// independent of what else that hop did or didn't override).
fn per_method_accessor_ident(name: &Ident) -> Ident {
    format_ident!("__dyn_x_for_{name}")
}

/// `ContentControl` -> `as_content_control`: the named, concretely-typed accessor a class's
/// `__elwindui_inherit_*!` macro generates for whoever inherits it (see this module's own doc
/// comment). Not generated for `trait_only` ancestors — see `build_inherit_macros`.
fn named_accessor_ident(bare_name: &str) -> Ident {
    format_ident!("as_{}", to_snake_case(bare_name))
}

/// `ContentControl` -> `into_content_control_node`: the upcast default method every
/// `{ClassName}Ext` trait declaration gets, one per class, converting `Rc<Self>` to
/// `Rc<dyn {ClassName}Ext>`. Because it lives as a *default method on the trait itself* (not
/// something `build_inherit_macros`'s per-ancestor forwarding `impl` has to emit), a descendant
/// gets one such method per ancestor it actually `impl`s (root, ordinary, and `trait_only` classes
/// alike — `struct_only` implementors inherit it for free from the existing trait they implement)
/// via Rust's own default-method inheritance through the supertrait chain, with no method at all
/// for an ancestor it doesn't have — e.g. `MenuItem`/`Window` never gain `into_ui_element_node`,
/// since neither ever `impl`s `UIElementExt`. `Self: Sized` keeps every one of these out of the
/// trait's vtable, so traits already used as `dyn {ClassName}Ext` elsewhere stay object-safe.
fn into_node_ident(bare_name: &str) -> Ident {
    format_ident!("into_{}_node", to_snake_case(bare_name))
}

fn build_into_node_default(into_ident: &Ident, ext_ty: &TokenStream2) -> TokenStream2 {
    quote! {
        fn #into_ident(self: std::rc::Rc<Self>) -> std::rc::Rc<dyn #ext_ty>
        where
            Self: Sized + 'static,
        {
            self
        }
    }
}

/// `ContentControl` -> `__elwindui_inherit_ContentControl`: the main entry point a subclass invokes
/// (see this module's own doc comment on the `__elwindui_inherit_*!` mechanism).
fn inherit_macro_ident(bare_name: &str) -> Ident {
    format_ident!("__elwindui_inherit_{}", bare_name)
}

/// `ContentControl` -> `__elwindui_inherit_ContentControl_skip`: the alternate entry point a
/// `struct_only` class invokes instead, when it already implements this ancestor's own trait itself
/// (see this module's own doc comment, and `struct_only_collides_with`).
fn inherit_macro_skip_ident(bare_name: &str) -> Ident {
    format_ident!("__elwindui_inherit_{}_skip", bare_name)
}

/// `ContentControl` -> `__elwindui_inherit_ContentControl_classify`: the internal tt-muncher that
/// sorts a flat list of `#[overrides]` methods into "belongs to me" (spliced into the generated
/// `impl`) vs. "belongs further up the chain" (forwarded, unexamined, to the next recursive call).
/// A separate macro name from `inherit_macro_ident`'s (rather than another arm of the same
/// `macro_rules!`) specifically to avoid any arity/shape overlap between the two — safer than
/// relying on `macro_rules!`'s first-match-wins arm ordering to disambiguate them.
fn inherit_macro_classify_ident(bare_name: &str) -> Ident {
    format_ident!("__elwindui_inherit_{}_classify", bare_name)
}

/// `ContentControl` -> `__elwindui_check_not_sealed_ContentControl`: see `#[sealed]`'s own handling
/// in `build_sealed_check_macro`/`expand_impl`.
fn sealed_check_ident(bare_name: &str) -> Ident {
    format_ident!("__elwindui_check_not_sealed_{}", bare_name)
}

/// `Button` -> `__elwindui_props_Button`: the property muncher this class declares for its own
/// `#[prop(..)]` declarations — the property-layer counterpart to `inherit_macro_ident`'s
/// hierarchy-layer trio. See `build_props_macro`.
fn props_macro_ident(bare_name: &str) -> Ident {
    format_ident!("__elwindui_props_{}", bare_name)
}

/// One `#[prop(..)]` declaration: a DSL-visible property of a builtin, declared right on the
/// `#[class]` item that actually implements it rather than restated in a separate table.
///
/// Accepted forms (each argument before the final `name: Type` is a flag):
/// - `#[prop(text: String)]` — a plain settable property
/// - `#[prop(routed, on_click: fn())]` — a routed callback (bubbles via `dispatch_routed`)
/// - `#[prop(onetime, left: Option<f32>)]` — applied once at construction, never re-pushed by a
///   later resync (`Window`'s geometry — the window manager owns the live value)
/// - `#[prop(two_way, text: String)]` — opts into automatic two-way wiring
/// - `#[prop(semantic_brush, background: Option<Brush>)]` — accepts `BrushStyle` and resolves it
///   against the effective Environment before calling the concrete-brush setter
/// - `#[prop(attached, row: i32 = 0)]` — an attached property (`Grid::row`), which a *child* element
///   sets on itself to address its parent. Always needs a default value.
///
/// Class-level markers live in their own attributes next to these: `#[content(children)]` names the
/// property that receives bare nested child elements, and `#[text_style]` marks a text-styling owner.
///
/// Class-level facts deliberately do *not* live here — they are either already `#[class]` arguments
/// (`sealed`, `abstract_class`, `text_style`) or derivable:
/// - **`native`** ≡ `trait_only` with no `inherits` — a base-less interface whose implementation is
///   hand-written per backend. Verified to hold for all 25 builtins at the time this was written; if
///   a future `trait_only` root class is *not* native, this derivation has to become explicit again.
/// - **`embedded`** carried no information once declarations moved onto the implementation itself:
///   every class declaring properties here is, by construction, hand-written Rust.
struct PropDecl {
    name: Ident,
    ty: Type,
    routed: bool,
    attached: bool,
    /// `= expr` — required for `attached`, rejected otherwise (see `Parse`).
    #[allow(dead_code)]
    default: Option<syn::Expr>,
    // `onetime` is accepted and recorded here so a builtin's declaration can already be written in
    // full, but the layer that consumes it (`emit_resync`'s onetime skip) still reads it from
    // `elwindui-codegen`'s own shape table and hasn't moved to this macro yet — see
    // `build_props_macro`. `two_way` *has* moved — see that function's own `@set_on_change` arms.
    #[allow(dead_code)]
    onetime: bool,
    two_way: bool,
    semantic_brush: bool,
}

impl Parse for PropDecl {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut routed = false;
        let mut onetime = false;
        let mut two_way = false;
        let mut attached = false;
        let mut semantic_brush = false;
        // Flags come first and are all bare idents; the property itself is the one `name: Type`
        // pair, so "an ident *not* followed by `:`" is unambiguously a flag.
        loop {
            let ident: Ident = input.parse()?;
            if input.peek(Token![:]) {
                input.parse::<Token![:]>()?;
                let ty: Type = input.parse()?;
                let default = if input.peek(Token![=]) {
                    input.parse::<Token![=]>()?;
                    Some(input.parse::<syn::Expr>()?)
                } else {
                    None
                };
                // Mirrors `validate.rs`'s `rejects_attached_field_without_default_value`: an
                // attached property is read off any child that never mentions it, so it has nothing
                // to fall back on but its declared default.
                if attached && default.is_none() {
                    return Err(syn::Error::new_spanned(
                        &ident,
                        format!(
                            "#[prop]: attached property `{ident}` needs a default value \
                             (e.g. `attached, {ident}: i32 = 0`)"
                        ),
                    ));
                }
                if !attached && default.is_some() {
                    return Err(syn::Error::new_spanned(
                        &ident,
                        format!(
                            "#[prop]: `{ident}` is not `attached`, so it cannot declare a default \
                             value — an ordinary property's default comes from the type's own \
                             constructor"
                        ),
                    ));
                }
                return Ok(PropDecl {
                    name: ident,
                    ty,
                    routed,
                    attached,
                    default,
                    onetime,
                    two_way,
                    semantic_brush,
                });
            }
            match ident.to_string().as_str() {
                "routed" => routed = true,
                "onetime" => onetime = true,
                "two_way" => two_way = true,
                "attached" => attached = true,
                "semantic_brush" => semantic_brush = true,
                other => {
                    return Err(syn::Error::new_spanned(
                        &ident,
                        format!(
                            "#[prop]: unknown flag `{other}` — expected `routed`, `onetime`, \
                             `two_way`, `attached`, `semantic_brush`, or a `name: Type` property \
                             declaration"
                        ),
                    ));
                }
            }
            input.parse::<Token![,]>()?;
        }
    }
}

/// A builtin's DSL declaration on one `#[class]` item: its `#[prop(..)]` properties plus the two
/// class-level markers that are neither `#[class]` arguments nor derivable.
#[derive(Default)]
struct PropDecls {
    props: Vec<PropDecl>,
    /// `#[content(children)]` — which property receives bare nested child elements. Routinely names
    /// a property an *ancestor* declares (`VerticalLayout`'s content is `Layout`'s `children`), so
    /// it is validated by the deferred `@assert_declared` probe rather than locally.
    content_field: Option<Ident>,
    /// `#[text_style]` — this class owns text styling (`TextStyleOwner`) for the font-inheritance
    /// walk. Not derivable: the real `impl TextStyleOwner for ..` lives in a separate block this
    /// macro never sees.
    #[allow(dead_code)]
    text_style: bool,
}

impl PropDecls {
    fn is_declared(&self) -> bool {
        !self.props.is_empty() || self.content_field.is_some() || self.text_style
    }

    fn content_field(&self) -> Option<&Ident> {
        self.content_field.as_ref()
    }
}

/// Splits the DSL declaration attributes — `#[prop(..)]`, `#[content(..)]`, `#[text_style]` — out of
/// an item's attribute list. All three are inert markers consumed entirely here; none may survive
/// into the generated item, since none is a real registered attribute anywhere.
fn take_prop_decls(attrs: &[syn::Attribute]) -> syn::Result<(PropDecls, Vec<syn::Attribute>)> {
    let mut shape = PropDecls::default();
    let mut rest = Vec::new();
    for attr in attrs {
        if attr.path().is_ident("prop") {
            shape.props.push(attr.parse_args::<PropDecl>()?);
        } else if attr.path().is_ident("content") {
            if shape.content_field.is_some() {
                return Err(syn::Error::new_spanned(
                    attr,
                    "#[content]: declared more than once — a class designates exactly one property \
                     to receive nested child elements",
                ));
            }
            shape.content_field = Some(attr.parse_args::<Ident>()?);
        } else if attr.path().is_ident("text_style") {
            shape.text_style = true;
        } else {
            rest.push(attr.clone());
        }
    }

    // Two `#[prop]`s with the same name on one item would generate two identical `@set` arms, and
    // `macro_rules!`'s first-match-wins would silently drop the second — a typo'd duplicate would
    // look like it took effect. Reject it here instead. (This only sees *one* class's own
    // declarations; a descendant shadowing an *ancestor's* property is caught by the deferred
    // `@assert_undeclared` probe — see `build_props_macro`.)
    let mut seen: Vec<String> = Vec::new();
    for prop in &shape.props {
        let name = prop.name.to_string();
        if seen.contains(&name) {
            return Err(syn::Error::new_spanned(
                &prop.name,
                format!("#[prop]: `{name}` is declared twice on this item"),
            ));
        }
        seen.push(name);
    }

    // Note there is deliberately *no* "somebody must designate a content property" check here: a
    // class routinely inherits its ancestor's designation (`VerticalLayout` takes `Layout`'s
    // `children`), and this macro cannot see an ancestor's declarations.

    Ok((shape, rest))
}

/// A call-position-safe form of a (possibly generic) `ExtTrait` path: `FooExt` unchanged when
/// `ty_generics` is empty, `FooExt::<T>` (turbofish) otherwise — a bare `FooExt<T>::method(..)` in
/// expression position parses as a chained comparison ("comparison operators cannot be chained";
/// see `build_dyn_default_methods`'s own doc comment on why `<dyn FooExt<T>>::method(..)` is not
/// used instead). `ext_ty` itself (`#ext_ident #ty_generics`, no `::`) stays correct as-is for
/// every *type*-position use (e.g. `&dyn #ext_ty` return types) — only the call form needs this.
fn turbofish_ext_path(ext_ident: &TokenStream2, ty_generics: &TokenStream2) -> TokenStream2 {
    if ty_generics.is_empty() {
        quote! { #ext_ident }
    } else {
        quote! { #ext_ident::#ty_generics }
    }
}

/// Builds the `fn #dyn_ident(&self) -> &dyn #ext_ty;` (no default) plus one default method per `sig`
/// — shared by `expand_trait_only` and `expand_impl`'s ordinary-class branch, the two places that
/// declare a *new* `{Name}Ext` trait (`struct_only` never does; root mode has its own pre-existing,
/// differently-shaped `as_ui_element` version of this same idea and isn't routed through here).
///
/// `overridable_names` (a subset of `sigs`' own names) get a *dedicated* per-method accessor
/// (`per_method_accessor_ident`) instead of the shared `dyn_ident` — see that function's own doc
/// comment for why a single shared accessor can't correctly resolve "closest override" once a hop
/// overrides only *some* of a trait's overridable methods. Every other (non-overridable) method
/// keeps dispatching through the single shared `dyn_ident`, unaffected — there being only ever one
/// real implementor for those (the declaring class itself), the shared accessor's "reflexive at the
/// declaring class, blind-forward everywhere else" shape has always been sufficient for them.
///
/// Calls through fully-qualified syntax (`#ext_ty::#name(receiver, ..)`), not `receiver.#name(..)` —
/// deliberately: a subclass is free to reuse an ancestor's method name for its own, differently-typed
/// concept (e.g. `ContentControl::padding(&self) -> Option<f32>` alongside `Control::padding(&self)
/// -> f32`), which makes `ContentControlExt: ControlExt` declare two same-named methods. Plain
/// dot-syntax on a `&dyn ContentControlExt` receiver is ambiguous between the two (E0034) regardless
/// of their differing signatures — Rust's method resolution doesn't disambiguate on return type — so
/// the default body must name which trait's method it means.
fn build_dyn_default_methods(
    dyn_ident: &Ident,
    ext_ty: &TokenStream2,
    ext_ty_call: &TokenStream2,
    sigs: &[syn::Signature],
    overridable_names: &[Ident],
) -> Vec<TokenStream2> {
    let mut out: Vec<TokenStream2> = sigs
        .iter()
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
            let receiver = if overridable_names.iter().any(|n| n == name) {
                let per_method = per_method_accessor_ident(name);
                quote! { self.#per_method() }
            } else {
                quote! { self.#dyn_ident() }
            };
            quote! {
                #sig {
                    // `#ext_ty_call` (not `#ext_ty`) here: this function's own doc comment above
                    // explains why the call must stay a plain, fully-qualified path expression
                    // (`ContentControlExt::padding(x)`, never `<dyn ContentControlExt>::padding(x)`
                    // or dot-syntax) — wrapping it as a `<dyn ..>` trait-object type instead
                    // (tried first, reverted) reintroduces exactly the E0034 ambiguity this
                    // fully-qualified form exists to avoid, confirmed against a real fixture
                    // (`examples/inheritance-demo`'s `LoudPanel`/`LabeledPanel`, whose own
                    // `mount`/`__build_view`/`__refresh_dynamic_regions` defaults became ambiguous
                    // between the two Ext traits under `<dyn ..>::`). `#ext_ty_call` differs from
                    // `#ext_ty` (used everywhere else, e.g. `&dyn #ext_ty` return types, which is
                    // a type position and unaffected) only when `#ext_ty` itself carries generic
                    // arguments (a generic root/ordinary class's `ExtTrait<T>`) — there, a bare
                    // `#ext_ty::#name(..)` parses as a chained comparison ("comparison operators
                    // cannot be chained"; `FooExt < T > :: value`), needing an explicit turbofish
                    // `::` before `<T>` instead.
                    #ext_ty_call::#name(#receiver #(, #arg_names)*)
                }
            }
        })
        .collect();
    out.push(quote! { fn #dyn_ident(&self) -> &dyn #ext_ty; });
    for name in overridable_names {
        let per_method = per_method_accessor_ident(name);
        out.push(quote! { fn #per_method(&self) -> &dyn #ext_ty; });
    }
    out
}

/// Builds the `__elwindui_inherit_{Name}!`/`__elwindui_inherit_{Name}_skip!`/
/// `__elwindui_inherit_{Name}_classify!` trio for a class named `bare_name`, with dyn-accessor
/// `dyn_ident`. Neither this class's own trait path nor its own concrete type path is baked into
/// these macros as a literal token — both travel as macro parameters (`$OwnTrait`/`$OwnConcrete`,
/// bound fresh at each `entry`/`classify` invocation) supplied by whoever invokes them, since
/// *this* class's own fully-qualified path is never something `#[class]` can determine on its own
/// (no `module_path!()`-equivalent is available to a proc-macro) — but the *caller* always has it,
/// from their own (now-required-fully-qualified) `inherits = ..`/`struct_only = ..` argument. See
/// this module's own doc comment on `validate_fully_qualified_path` for the full rationale.
///
/// `has_concrete_type` drives `as_<name>()` named-accessor generation (skipped for `trait_only`,
/// which has no concrete type to return a reference to). `overridable_names` are this class's own
/// `#[overridable]`-tagged method names, each becoming one literal-matching arm in the classify
/// muncher. `extra_required_names` are further required methods beyond `dyn_ident` that also need
/// a blind `self.base.#name()` forward returning `&$OwnConcrete` (only ever non-empty for a root
/// class's own `as_ui_element`; see `expand_impl`'s own doc comment on root class mode).
/// `recurse_macro_path` is the fully-qualified macro-path (crate-root-prefixed) to continue the
/// chain with; `recurse_next` is `Some(&AncestorRoute)` — this class's *own* knowledge (from its
/// own `inherits = ..`) of how to reach the *next* hop, deliberately unresolved (see
/// `AncestorRoute`'s own doc comment on why) — or `None` when recursing into the terminal macro (no
/// next hop, so nothing to pass beyond `$SubType` itself).
///
/// `skip_own_impl` (`ClassArgs::no_ancestor_forward`) omits the `impl $OwnTrait for $SubType`
/// block from the classify muncher's base case entirely — this class's own trait is a
/// hand-written one that predates the `__dyn_x` convention (e.g. `NativeTabView`'s `struct_only =
/// TabView`), so it has no `#dyn_ident` method to implement (adding one would be E0407, "method
/// not a member of trait"). The named accessor and recursion into whatever *this* class itself
/// further inherits both still happen normally — only the direct `impl` of this one hop is
/// skipped, exactly mirroring the old (pre-unification) behavior where an unrelated, always-
/// unconditional blind forward to `UIElementExt` meant a `no_ancestor_forward` hop never blocked
/// anything beyond itself.
/// Builds `__elwindui_shape_{bare_name}!` — the DSL **shape** layer's counterpart to
/// `build_inherit_macros`'s hierarchy layer, and the reason a builtin no longer needs a separate
/// entry in a compiler-side shape table.
///
/// The problem it solves: `#[elwindui::component]` expands in the *consumer's* crate, and a
/// proc-macro can never read another crate's macro-expansion results (separate compilation units).
/// So a builtin declared in `elwindui-core` cannot hand its DSL surface to the consumer's
/// `#[component]` expansion directly. The same deferred, token-level composition
/// `__elwindui_inherit_*!` already uses for the class hierarchy works here too: the *generated code*
/// invokes this macro, and rustc expands it later, in the consumer's crate, against the definition
/// that lives here.
///
/// Shape:
/// - `(@set $recv, $name, $value)` — the entry callers use. It only seeds `$origin` (this class's
///   own name) and hands off to `@set_from`.
/// - `(@set_from $origin, $recv, <prop>, $value)` — one arm per declared `#[prop]`, expanding to
///   that property's real setter call.
/// - `(@set_from $origin, $recv, $name:ident, $value)` — the catch-all, forwarding anything this
///   class doesn't declare to its own ancestor's shape macro, `$origin` unchanged. Same "forward
///   what isn't mine up the chain" muncher shape `build_inherit_macros`'s `classify` uses for
///   `#[overrides]`.
/// - at the end of the chain (a class with no `inherits`) that catch-all becomes a `compile_error!`
///   instead. It reports `$origin` — the type the *use site* actually named — not this terminal
///   class, which the user may never have heard of (`Button { titel: .. }` must blame `Button`, not
///   `UIElement`).
///
/// Setter calls are emitted in *method* syntax (`$recv.set_text(..)`) rather than a fully-qualified
/// trait path: the `{Name}Ext` trait's own module path isn't knowable from inside this macro, and
/// the generated view code already brings the right `..Ext` traits into scope at the call site.
///
/// **Ancestor shadowing is rejected, not silently allowed.** A class's own literal arms are matched
/// before the forwarding catch-all, so a descendant redeclaring a property name an *ancestor* also
/// declares would win silently and make the ancestor's setter unreachable — diverging from the rule
/// the table-based path enforces (`validate::validate_field_overrides`: a same-named inherited field
/// is an error unless the redeclaration is marked `#[override]`). A proc-macro cannot see the
/// ancestor's property list at declaration time — that is precisely what isn't readable across
/// crates, and the reason this macro exists at all. So the check is deferred the same way everything
/// else here is: each class emits one `@assert_undeclared` probe per declared property against its
/// parent's shape macro, and rustc resolves it later. An ancestor that declares the name expands the
/// probe to a `compile_error!`; one that doesn't forwards it further up; the chain's terminal
/// expands it to nothing. Cost is one item-position macro call per declared property.
fn build_props_macro(
    bare_name: &str,
    shape: &PropDecls,
    parent: Option<(&str, &Type)>,
) -> TokenStream2 {
    let macro_ident = props_macro_ident(bare_name);
    let bare_ident = format_ident!("{bare_name}");

    // Only plain scalar properties and `#[routed]` callbacks take part in the `@set` protocol. The
    // rest each need their own emission shape and are deliberately left without an arm here rather
    // than given a wrong one:
    // - `attached` properties are set by a *child* onto its parent, through
    //   `UIElement::set_attached::<T>`, so they never reach this receiver at all — `@attached_set`;
    // - a `children`-named collection and a *collection-shaped* `#[prop(content, ..)]` property
    //   receive nested *elements* whose emission differs by declared type (`Vec<T>` bulk-set vs
    //   `ListExt<T>` add-loop) — `@children`, via `attach_kind`'s `Collection`/`VecProperty` cases.
    // Reaching one of these through `@set` falls through to the catch-all and is reported as an
    // unknown property, which is honest: `@set` genuinely cannot express it.
    //
    // A *single-slot* (`AttachKind::SingleSlot`, `Rc<dyn ..>`) `#[prop(content, ..)]` property is
    // deliberately *not* excluded here, even though `@children` can reach it too (its
    // `@children_into`'s single-slot arm) — the DSL only ever addresses a single-slot content field
    // by a *named* element-valued attribute (`content: Grid { .. }`, one value; the `@children`,
    // bare-nested-child spelling is how a `Vec`/`Collection`-shaped field is filled instead, never a
    // single slot), and a named attribute's value always reaches this receiver through `@set` like
    // any other named property — `elwindui-codegen`'s `emit_external_construction` has no shape
    // table telling it which field is content-designated, so it needs `@set` to already accept it.
    // Both paths emit the exact same `$recv.#setter($value)` call for this case, so there is nothing
    // to reconcile between them.
    let takes_set_arm = |p: &PropDecl| {
        let name = p.name.to_string();
        let is_collection_shaped_content = shape.content_field().map(|c| c.to_string()).as_deref()
            == Some(name.as_str())
            && !matches!(attach_kind(&p.ty), AttachKind::SingleSlot);
        !p.attached && name != "children" && !is_collection_shaped_content
    };

    // A `#[routed]` property's `@set_from` arm has a fundamentally different job than a plain one:
    // `elwindui-codegen` hands it a *bare* closure/callable matching the DSL's own written arity (the
    // same value it would build for an ordinary callback property — see `wrap_prop_value`'s doc
    // comment on why this, and not `dyn UIElementExt`-typed values, is the one case that still needs
    // this macro's help beyond a plain pass-through), never a pre-built `Box<dyn Fn(&T,
    // &RoutedEventArgs)>`. The arm itself builds the routed adapter and calls
    // `register_routed_handler` — `elwindui-codegen` never has to know a property is `#[routed]` at
    // all, only that it wrote a callback-shaped value; whether that dispatches to a setter or a
    // routed registration is entirely this declaration's own business. `@routed`/`@routed_from`
    // (below) stay as the lower-level primitive this delegates to, for a caller that already has a
    // fully-built handler in hand.
    // `@clear` resets a property to its platform default (`clear_<name>()`, no arguments) — the
    // other half of a clearable property's dispatch, alongside `@set`. Same forwarding shape as
    // `@set`, minus a value to carry. Routed properties are excluded (unlike `@set`, which handles
    // them too) — an event registration has no "reset to platform default" concept.
    let clear_arms: Vec<TokenStream2> = shape
        .props
        .iter()
        .filter(|p| takes_set_arm(p) && !p.routed)
        .map(|p| {
            let name = &p.name;
            let clear = format_ident!("clear_{}", name);
            quote! {
                (@clear_from $origin:ident, $recv:expr, #name) => {
                    $recv.#clear();
                };
            }
        })
        .collect();

    // `@field_type` answers "what Rust type does this class (or an ancestor) declare property
    // `$name` as", expanded in *type position* rather than value/expr position — the missing half
    // of `elwindui-codegen::codegen::resolve_effective_fields`'s external-base fallback (Refs #90):
    // a consumer crate's `#[elwindui::component(inherits Control)]` that bare-forwards an inherited
    // attribute (`padding: padding`) has no local `TypeInfo` for `Control` to read a field type
    // from (real builtins no longer have one at all, see `elwindui_codegen::TEST_BUILTIN_SHAPE_SOURCE`'s
    // own doc comment) — so the derived component's own synthesized struct field/constructor
    // parameter for `padding` is given `elwindui::core::__elwindui_props_Control!(@field_type
    // padding)` as its literal type text, deferring to *this* macro (which does know the real type,
    // declared right here) at the consumer's own expansion time. Same two-hop `@field_type`/
    // `@field_type_from` shape as `@clear`/`@clear_from` (`$origin` seeded at the entry so a
    // terminal-chain `compile_error!` can still name the type the use site actually wrote), same
    // `takes_set_arm`-filtered property set (a routed callback/attached/children-shaped property
    // has no single settable "value type" a struct field could hold). `rewrite_crate_segment` is
    // required here (unlike `@set_from`'s `wrap_prop_value` calls) because this type is spliced
    // literally into the *generated* type position rather than only consulted to decide how to
    // wrap a value — a bare `crate::`-relative type spelled in the declaring crate (`elwindui-core`)
    // must resolve there, not in whichever consumer crate eventually expands this arm, exactly like
    // `attached_arms`'s identical `#[attached]` type handling above.
    let field_type_arms: Vec<TokenStream2> = shape
        .props
        .iter()
        .filter(|p| takes_set_arm(p) && !p.routed)
        .map(|p| {
            let name = &p.name;
            let prop_ty = &p.ty;
            let ty = rewrite_crate_segment(quote! { #prop_ty });
            quote! {
                (@field_type_from $origin:ident, #name) => { #ty };
            }
        })
        .collect();

    let set_arms: Vec<TokenStream2> = shape
        .props
        .iter()
        .filter(|p| takes_set_arm(p))
        .map(|p| {
            let name = &p.name;
            if p.routed {
                let handler = routed_handler_from_bare_callable(&p.ty);
                quote! {
                    (@set_from $origin:ident, $recv:expr, #name, $value:expr) => {
                        $crate::#macro_ident!(@routed_from $origin, $recv, #name, #handler);
                    };
                }
            } else {
                let setter = format_ident!("set_{}", name);
                let value = wrap_prop_value(&p.ty, quote! { $value });
                quote! {
                    (@set_from $origin:ident, $recv:expr, #name, $value:expr) => {
                        $recv.#setter(#value);
                    };
                }
            }
        })
        .collect();

    // Semantic-brush capability belongs to the property declaration, not to its spelling in
    // elwindui-codegen. `@set_with_environment` lets cross-crate codegen defer that decision until
    // this macro is expanded beside the real `#[prop]`: marked properties resolve `BrushStyle`,
    // ordinary properties delegate to the existing `@set` conversion path unchanged.
    let set_with_environment_arms: Vec<TokenStream2> = shape
        .props
        .iter()
        .filter(|p| takes_set_arm(p))
        .map(|p| {
            let name = &p.name;
            if p.semantic_brush {
                quote! {
                    (@set_with_environment_from $origin:ident, $recv:expr, #name, $value:expr, $environment:expr) => {
                        match ::core::convert::Into::<$crate::theme::BrushStyle>::into($value)
                            .resolve($environment)
                        {
                            $crate::theme::ResolvedValue::Value(__elwindui_semantic_brush) => {
                                $crate::#macro_ident!(
                                    @set_from $origin, $recv, #name, __elwindui_semantic_brush
                                );
                            }
                            $crate::theme::ResolvedValue::PlatformDefault => {
                                $crate::#macro_ident!(@clear_from $origin, $recv, #name);
                            }
                        }
                    };
                }
            } else {
                quote! {
                    (@set_with_environment_from $origin:ident, $recv:expr, #name, $value:expr, $environment:expr) => {
                        $crate::#macro_ident!(@set_from $origin, $recv, #name, $value);
                    };
                }
            }
        })
        .collect();

    let semantic_brush_arms: Vec<TokenStream2> = shape
        .props
        .iter()
        .map(|p| {
            let name = &p.name;
            let value = p.semantic_brush;
            quote! { (@is_semantic_brush_from $origin:ident, #name) => { #value }; }
        })
        .collect();

    // `#[text_style]` injects these properties as one declared capability rather than seven
    // handwritten `#[prop]`s. Generate the same deferred setter metadata here so codegen never has
    // to infer text-style ownership from a field name. The shared declaration metadata decides
    // which injected fields are semantic brushes; the others forward concrete values directly.
    let text_style_set_with_environment_arms = shape.text_style.then(|| {
        let arms = elwindui_codegen::TEXT_STYLE_FIELDS
            .iter()
            .map(|(name, _, semantic_brush)| {
            let name = format_ident!("{name}");
            let setter = format_ident!("set_{name}");
            let clear = format_ident!("clear_{name}");
            if *semantic_brush {
                quote! {
                    (@set_with_environment_from $origin:ident, $recv:expr, #name, $value:expr, $environment:expr) => {
                        match ::core::convert::Into::<$crate::theme::BrushStyle>::into($value)
                            .resolve($environment)
                        {
                            $crate::theme::ResolvedValue::Value(__elwindui_semantic_brush) => {
                                $crate::ui::UIElementExt::as_text_style_owner(&*($recv))
                                    .expect(concat!(
                                        "`", stringify!(#name), "` is declared by #[text_style], \
                                         but the resolved node has no TextStyleOwner"
                                    ))
                                    .#setter(::core::option::Option::Some(__elwindui_semantic_brush));
                            }
                            $crate::theme::ResolvedValue::PlatformDefault => {
                                $crate::ui::UIElementExt::as_text_style_owner(&*($recv))
                                    .expect(concat!(
                                        "`", stringify!(#name), "` is declared by #[text_style], \
                                         but the resolved node has no TextStyleOwner"
                                    ))
                                    .#clear();
                            }
                        }
                    };
                    (@is_semantic_brush_from $origin:ident, #name) => { true };
                }
            } else {
                quote! {
                    (@set_with_environment_from $origin:ident, $recv:expr, #name, $value:expr, $environment:expr) => {
                        $crate::ui::UIElementExt::as_text_style_owner(&*($recv))
                            .expect(concat!(
                                "`", stringify!(#name), "` is declared by #[text_style], but the \
                                 resolved node has no TextStyleOwner"
                            ))
                            .#setter($value);
                    };
                    (@is_semantic_brush_from $origin:ident, #name) => { false };
                }
            }
        });
        quote! {
            #(#arms)*
        }
    });

    // `@routed` registers a `#[routed]` callback (`dispatch_routed`-style bubbling) instead of
    // assigning it — the same two-hop entry/forward shape as `@set`, but dispatching to
    // `UIElementExt::register_routed_handler::<Payload>(name, handler)` instead of a setter. The
    // payload type comes from the property's own declared `fn(..)`/`fn()` signature, known here at
    // declaration time — `elwindui-codegen` no longer needs to derive it from a use site's resolved
    // `TypeInfo`. `$value` is expected to already be a fully-built `Box<dyn Fn(&Payload,
    // &RoutedEventArgs)>`; its inner closure may leave `Payload` unannotated and let this call's own
    // turbofish infer it (verified: rustc resolves an untyped closure parameter through a generic
    // function's explicit turbofish even across a `Box<dyn Fn>` coercion).
    let routed_arms: Vec<TokenStream2> = shape
        .props
        .iter()
        .filter(|p| p.routed)
        .map(|p| {
            let name = &p.name;
            let name_str = name.to_string();
            let payload_ty = routed_payload_type(&p.ty);
            quote! {
                (@routed_from $origin:ident, $recv:expr, #name, $value:expr) => {
                    $recv.register_routed_handler::<#payload_ty>(#name_str, $value);
                };
            }
        })
        .collect();

    // `@attached_set` needs no forwarding chain, unlike `@set`/`@routed`/`@children`: the DSL's
    // `Owner::field: value` syntax always names its owning class explicitly (`Grid::row`, never
    // inherited/shadowed), so `elwindui-codegen` calls straight into `Owner`'s own props macro — no
    // ambiguity to resolve by walking ancestors. The turbofish on `set_attached::<T>` is exactly why
    // this needs to be a macro at all: `T` is `#[prop(attached, ..)]`'s own declared type, known
    // here and nowhere `elwindui-codegen` can still see once a builtin's shape lives only here.
    let attached_arms: Vec<TokenStream2> = shape
        .props
        .iter()
        .filter(|p| p.attached)
        .map(|p| {
            let name = &p.name;
            let name_str = name.to_string();
            // Same reasoning as `routed_payload_type`: this type is spliced as an explicit
            // turbofish into generated code that may be invoked from any crate, so a leading
            // `crate::` in how the attached type was spelled needs `$crate::` here.
            let prop_ty = &p.ty;
            let ty = rewrite_crate_segment(quote! { #prop_ty });
            quote! {
                (@attached_set #name, $binding:expr, $value:expr) => {
                    $binding.as_ui_element().set_attached::<#ty>(#bare_name, #name_str, $value);
                };
            }
        })
        .collect();
    let attached_fallback = quote! {
        (@attached_set $name:ident, $binding:expr, $value:expr) => {
            compile_error!(concat!(
                "`",
                #bare_name,
                "` has no `#[attached]` property named `",
                stringify!($name),
                "`"
            ));
        };
    };

    // `@set_on_change` wires a two-way property's own native "value changed" callback back into a
    // DSL use site's bound path (`elwindui-codegen`'s `emit_wiring`, the counterpart to `@set`'s
    // forward direction) — one arm per `#[prop(two_way, ..)]` field, calling that field's own
    // typed `set_on_<property>_change` setter. Since codegen only invokes this arm for explicit
    // `<=>`, reaching the fallback is a real capability error rather than an ignorable candidate.
    let two_way_arms: Vec<TokenStream2> = shape
        .props
        .iter()
        .filter(|p| p.two_way)
        .map(|p| {
            let name = &p.name;
            let change_setter = format_ident!("set_on_{}_change", name);
            quote! {
                (@set_on_change #name, $widget:expr, $on_change:expr) => {
                    $widget.#change_setter($on_change);
                };
            }
        })
        .collect();
    let two_way_fallback = quote! {
        (@set_on_change $name:ident, $widget:expr, $on_change:expr) => {
            compile_error!(concat!(
                "property `", stringify!($name), "` does not support two-way binding"
            ));
        };
    };

    // One arm per declared property — *every* property, not just the `@set`-able ones: a name
    // collision is a collision whether the ancestor's version is a setter, a routed callback, or an
    // attached property.
    let assert_arms: Vec<TokenStream2> = shape
        .props
        .iter()
        .map(|p| {
            let name = &p.name;
            let msg = format!("` redeclares `{name}`, already declared by its ancestor `{bare_name}` — pick a different name, or extend the ancestor's declaration instead");
            quote! {
                (@assert_undeclared $origin:ident, #name) => {
                    compile_error!(concat!("`", stringify!($origin), #msg));
                };
            }
        })
        .collect();

    // `@children` attaches bare nested child elements to whichever property `#[content(..)]` names.
    // It takes two hops on purpose: the *designation* is local (`VerticalLayout` says
    // `#[content(children)]`) but the designated property's declared *type* — which decides how
    // children are attached — may live several classes up (`children` is `Layout`'s). So `@children`
    // resolves the name here and hands off to `@children_into`, which walks the chain until it finds
    // the class that actually declares that property.
    let children_into_arms: Vec<TokenStream2> = shape
        .props
        .iter()
        .map(|p| {
            let name = &p.name;
            let setter = format_ident!("set_{}", name);
            let body = match attach_kind(&p.ty) {
                // A live collection (`UIElementCollection`, `ListExt<T>`): append through the
                // accessor, mirroring the `.children().add(..)` convention virtual builtins already
                // use, rather than replacing the whole collection.
                AttachKind::Collection => quote! {
                    $( $recv.#name().add($child); )*
                },
                // A `Vec<T>` property is replaced wholesale by its setter (`TabView`'s `children`).
                AttachKind::VecProperty => quote! {
                    $recv.#setter(::std::vec![$($child),*]);
                },
                // A single slot (`Rc<dyn ..>`): exactly one child, enforced by the arity of the
                // dedicated arm below rather than by a runtime check.
                AttachKind::SingleSlot => quote! {
                    compile_error!(concat!(
                        "`",
                        stringify!($origin),
                        "` takes exactly one child element, but got several"
                    ));
                },
            };
            let single_slot_arm = matches!(attach_kind(&p.ty), AttachKind::SingleSlot).then(|| {
                let child = single_slot_child_value(&p.ty, quote! { $child });
                quote! {
                    (@children_into $origin:ident, #name, $recv:expr, [$child:expr]) => {
                        $recv.#setter(#child);
                    };
                }
            });
            quote! {
                #single_slot_arm
                (@children_into $origin:ident, #name, $recv:expr, [$($child:expr),* $(,)?]) => {
                    #body
                };
            }
        })
        .collect();

    // A class that designates content resolves the name here; one that doesn't lets the catch-all
    // below carry the whole `@children` call up to an ancestor that does.
    let children_entry = shape.content_field().map(|content| {
        quote! {
            (@children $recv:expr, [$($child:expr),* $(,)?]) => {
                $crate::#macro_ident!(@children_into #bare_ident, #content, $recv, [$($child),*]);
            };
        }
    });

    // `@content_item_dyn` expands to `dyn TraitExt` for the content-collection's element type
    // (`TabView`'s `children: Vec<Rc<dyn TabViewItemExt>>` -> `dyn TabViewItemExt`, `Layout`'s
    // `children: UIElementCollection` -> `dyn UIElementExt`) — used in *type* position
    // (`DynamicChildSlot<$crate::#macro_ident!(@content_item_dyn)>`), the one piece of a dynamic
    // `for`/`if`/`match` region's own generated struct field `elwindui-codegen` can no longer name
    // without a shape table once a builtin's shape lives only here. Exactly `@children`'s own two-
    // hop shape (this fn's own doc comment on `children_entry`/`@children_into`), for the identical
    // reason: the class that *designates* the content field (`VerticalLayout`'s `#[content(children)]`)
    // is routinely not the one that *declares* it (`Layout`'s own `#[prop(children: ..)]`) — so the
    // entry only ever resolves the *name* here, and `@content_item_dyn_into` (mirroring
    // `@children_into`) is what actually walks the chain to whichever class's `shape.props` really
    // has it.
    let content_item_dyn_entry = shape.content_field().map(|content| {
        quote! {
            (@content_item_dyn) => {
                $crate::#macro_ident!(@content_item_dyn_into #bare_ident, #content)
            };
        }
    });

    // `@content_field_get $recv:expr` calls whichever getter `#[content(..)]` names directly on
    // `$recv` (`Dropdown`'s `items()`, `TabView`'s `children()`, ...) — the shape-macro counterpart
    // `elwindui-codegen`'s `dynamic_region_refresh_method` needs for a *cross-crate* builtin's list-
    // content field, where it has no local `TypeInfo.content_field` to read directly (the same-crate
    // case just reads that field; this query exists only for the case it can't). One hop only, not
    // `@content_item_dyn`'s two: the *name* is all that's needed here (no property type to resolve),
    // and Rust's own trait method resolution already finds the getter on `$recv` however far up the
    // ancestor chain it's actually declared, once `dynamic_region_refresh_method`'s own `use
    // elwindui::core::ui::#parent_ext as _;` brings the right trait into scope — so unlike
    // `@children`/`@content_item_dyn`, this class's own `#[content(..)]` designation is the entire
    // answer.
    let content_field_get_entry = shape.content_field().map(|content| {
        quote! {
            (@content_field_get $recv:expr) => {
                $recv.#content()
            };
        }
    });
    let content_item_dyn_into_arms: Vec<TokenStream2> = shape
        .props
        .iter()
        .map(|p| {
            let name = &p.name;
            let dyn_ty = content_item_dyn_type(&p.ty);
            quote! {
                (@content_item_dyn_into $origin:ident, #name) => { #dyn_ty };
            }
        })
        .collect();

    // `@assert_declared` is `@assert_undeclared`'s mirror: it succeeds where the other fails. It
    // backs `#[prop(content, ..)]`, whose designation is routinely *inherited*
    // (`VerticalLayout`'s `content = children` names `Layout`'s `children`), so it cannot be checked
    // locally any more than a collision can.
    let declared_arms: Vec<TokenStream2> = shape
        .props
        .iter()
        .map(|p| {
            let name = &p.name;
            quote! { (@assert_declared $origin:ident, #name) => {}; }
        })
        .collect();

    let (
        fallback,
        set_with_environment_fallback,
        semantic_brush_fallback,
        clear_fallback,
        field_type_fallback,
        routed_fallback,
        assert_fallback,
        declared_fallback,
        children_fallback,
    ) = match parent {
        Some((parent_bare, parent_ty)) => {
            let parent_macro =
                inherit_macro_self_ref_path(parent_bare, parent_ty, props_macro_ident(parent_bare));
            (
                quote! {
                    (@set_from $origin:ident, $recv:expr, $name:ident, $value:expr) => {
                        #parent_macro!(@set_from $origin, $recv, $name, $value);
                    };
                },
                quote! {
                    (@set_with_environment_from $origin:ident, $recv:expr, $name:ident, $value:expr, $environment:expr) => {
                        #parent_macro!(@set_with_environment_from $origin, $recv, $name, $value, $environment);
                    };
                },
                quote! {
                    (@is_semantic_brush_from $origin:ident, $name:ident) => {
                        #parent_macro!(@is_semantic_brush_from $origin, $name)
                    };
                },
                quote! {
                    (@clear_from $origin:ident, $recv:expr, $name:ident) => {
                        #parent_macro!(@clear_from $origin, $recv, $name);
                    };
                },
                quote! {
                    (@field_type_from $origin:ident, $name:ident) => {
                        #parent_macro!(@field_type_from $origin, $name)
                    };
                },
                quote! {
                    (@routed_from $origin:ident, $recv:expr, $name:ident, $value:expr) => {
                        #parent_macro!(@routed_from $origin, $recv, $name, $value);
                    };
                },
                quote! {
                    (@assert_undeclared $origin:ident, $name:ident) => {
                        #parent_macro!(@assert_undeclared $origin, $name);
                    };
                },
                quote! {
                    (@assert_declared $origin:ident, $name:ident) => {
                        #parent_macro!(@assert_declared $origin, $name);
                    };
                },
                quote! {
                    (@children $recv:expr, [$($child:expr),* $(,)?]) => {
                        #parent_macro!(@children $recv, [$($child),*]);
                    };
                    (@children_into $origin:ident, $name:ident, $recv:expr, [$($child:expr),* $(,)?]) => {
                        #parent_macro!(@children_into $origin, $name, $recv, [$($child),*]);
                    };
                },
            )
        }
        None => (
            quote! {
                (@set_from $origin:ident, $recv:expr, $name:ident, $value:expr) => {
                    compile_error!(concat!(
                        "`",
                        stringify!($origin),
                        "` (or any of its ancestors) has no such property: ",
                        stringify!($name)
                    ));
                };
            },
            quote! {
                (@set_with_environment_from $origin:ident, $recv:expr, $name:ident, $value:expr, $environment:expr) => {
                    compile_error!(concat!(
                        "`",
                        stringify!($origin),
                        "` (or any of its ancestors) has no such property: ",
                        stringify!($name)
                    ));
                };
            },
            quote! {
                (@is_semantic_brush_from $origin:ident, $name:ident) => {
                    compile_error!(concat!(
                        "`",
                        stringify!($origin),
                        "` (or any of its ancestors) has no such property: ",
                        stringify!($name)
                    ))
                };
            },
            quote! {
                (@clear_from $origin:ident, $recv:expr, $name:ident) => {
                    compile_error!(concat!(
                        "`",
                        stringify!($origin),
                        "` (or any of its ancestors) has no such property: ",
                        stringify!($name)
                    ));
                };
            },
            quote! {
                (@field_type_from $origin:ident, $name:ident) => {
                    compile_error!(concat!(
                        "`",
                        stringify!($origin),
                        "` (or any of its ancestors) has no such property: ",
                        stringify!($name)
                    ))
                };
            },
            quote! {
                (@routed_from $origin:ident, $recv:expr, $name:ident, $value:expr) => {
                    compile_error!(concat!(
                        "`",
                        stringify!($origin),
                        "` (or any of its ancestors) has no such #[routed] event: ",
                        stringify!($name)
                    ));
                };
            },
            // Reached the top of the chain without a collision: the name is free.
            quote! {
                (@assert_undeclared $origin:ident, $name:ident) => {};
            },
            quote! {
                (@assert_declared $origin:ident, $name:ident) => {
                    compile_error!(concat!(
                        "#[prop(content, ",
                        stringify!($name),
                        ")] on `",
                        stringify!($origin),
                        "`: neither it nor any ancestor declares a `",
                        stringify!($name),
                        "` property"
                    ));
                };
            },
            quote! {
                (@children $recv:expr, [$($child:expr),* $(,)?]) => {
                    compile_error!(
                        "this element takes no nested child elements — no `#[content(..)]` is \
                         declared on it or any ancestor"
                    );
                };
                (@children_into $origin:ident, $name:ident, $recv:expr, [$($child:expr),* $(,)?]) => {
                    compile_error!(concat!(
                        "#[content(",
                        stringify!($name),
                        ")] on `",
                        stringify!($origin),
                        "`: neither it nor any ancestor declares a `",
                        stringify!($name),
                        "` property"
                    ));
                };
            },
        ),
    };

    // `@content_item_dyn`'s own fallback — same two-way split as `@set_from`/`@children` above: an
    // ancestor forwards the query further up (`content_item_dyn_entry`'s own doc comment on why an
    // unmatched name here isn't an error); the terminal class turns an unresolved query into a
    // `compile_error!` naming the type that actually asked (`$origin`), same spirit as the others.
    let content_item_dyn_fallback = match parent {
        Some((parent_bare, parent_ty)) => {
            let parent_macro =
                inherit_macro_self_ref_path(parent_bare, parent_ty, props_macro_ident(parent_bare));
            quote! {
                (@content_item_dyn) => {
                    #parent_macro!(@content_item_dyn)
                };
                (@content_item_dyn_into $origin:ident, $name:ident) => {
                    #parent_macro!(@content_item_dyn_into $origin, $name)
                };
            }
        }
        None => quote! {
            (@content_item_dyn) => {
                compile_error!(concat!(
                    "`",
                    #bare_name,
                    "` (or any of its ancestors) declares no content-collection property to hold \
                     a dynamic `for`/`if`/`match` region's children"
                ))
            };
            (@content_item_dyn_into $origin:ident, $name:ident) => {
                compile_error!(concat!(
                    "`",
                    stringify!($origin),
                    "`: neither it nor any ancestor declares a `",
                    stringify!($name),
                    "` property to hold a dynamic `for`/`if`/`match` region's children"
                ))
            };
        },
    };

    // `@content_field_get`'s own fallback — same two-way split as `@content_item_dyn` above: an
    // ancestor forwards the query further up; the terminal class turns an unresolved query into a
    // `compile_error!`.
    let content_field_get_fallback = match parent {
        Some((parent_bare, parent_ty)) => {
            let parent_macro =
                inherit_macro_self_ref_path(parent_bare, parent_ty, props_macro_ident(parent_bare));
            quote! {
                (@content_field_get $recv:expr) => {
                    #parent_macro!(@content_field_get $recv)
                };
            }
        }
        None => quote! {
            (@content_field_get $recv:expr) => {
                compile_error!(concat!(
                    "`",
                    #bare_name,
                    "` (or any of its ancestors) declares no content-collection property to hold \
                     a dynamic `for`/`if`/`match` region's children"
                ))
            };
        },
    };

    // The probes themselves, emitted in item position next to the class.
    let mut probes: Vec<TokenStream2> = Vec::new();
    // Collision probe: only a class that *has* an ancestor emits any — a root class has nothing to
    // collide with.
    if let Some((parent_bare, parent_ty)) = parent {
        let parent_macro =
            inherit_macro_path(parent_bare, parent_ty, props_macro_ident(parent_bare));
        probes.extend(shape.props.iter().map(|p| {
            let name = &p.name;
            quote! { #parent_macro!(@assert_undeclared #bare_ident, #name); }
        }));
    }
    // Content probe: aimed at this class's *own* macro, so a content field declared right here
    // matches a literal arm and a merely inherited one forwards up the chain from there.
    if let Some(content) = shape.content_field() {
        let own_macro = inherit_macro_path_for_self(bare_name);
        probes.push(quote! { #own_macro!(@assert_declared #bare_ident, #content); });
    }

    quote! {
        #[doc(hidden)]
        #[macro_export]
        #[allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]
        macro_rules! #macro_ident {
            (@set $recv:expr, $name:ident, $value:expr) => {
                $crate::#macro_ident!(@set_from #bare_ident, $recv, $name, $value);
            };
            #(#set_arms)*
            #fallback
            (@set_with_environment $recv:expr, $name:ident, $value:expr, $environment:expr) => {
                $crate::#macro_ident!(
                    @set_with_environment_from #bare_ident, $recv, $name, $value, $environment
                );
            };
            #text_style_set_with_environment_arms
            #(#set_with_environment_arms)*
            #set_with_environment_fallback
            (@is_semantic_brush $name:ident) => {
                $crate::#macro_ident!(@is_semantic_brush_from #bare_ident, $name)
            };
            #(#semantic_brush_arms)*
            #semantic_brush_fallback
            (@clear $recv:expr, $name:ident) => {
                $crate::#macro_ident!(@clear_from #bare_ident, $recv, $name);
            };
            #(#clear_arms)*
            #clear_fallback
            (@field_type $name:ident) => {
                $crate::#macro_ident!(@field_type_from #bare_ident, $name)
            };
            #(#field_type_arms)*
            #field_type_fallback
            (@routed $recv:expr, $name:ident, $value:expr) => {
                $crate::#macro_ident!(@routed_from #bare_ident, $recv, $name, $value);
            };
            #(#routed_arms)*
            #routed_fallback
            #(#attached_arms)*
            #attached_fallback
            #(#two_way_arms)*
            #two_way_fallback
            #(#assert_arms)*
            #assert_fallback
            #(#declared_arms)*
            #declared_fallback
            #children_entry
            #(#children_into_arms)*
            #children_fallback
            #content_item_dyn_entry
            #(#content_item_dyn_into_arms)*
            #content_item_dyn_fallback
            #content_field_get_entry
            #content_field_get_fallback
        }
        #(#probes)*
    }
}

/// A `#[prop(routed, on_x: fn(T0, ...))]`'s payload type — mirrors
/// `elwindui_codegen::codegen::callback_param_types`, operating directly on the already-parsed
/// `syn::Type` this macro has instead of a re-parsed type string. `fn()` (no parameters) has payload
/// `()`, matching `register_routed_handler::<()>`'s existing zero-parameter convention
/// (`elwindui_codegen::codegen::emit_generic_on_click_routing`). Only 0-or-1 parameters are
/// supported today, the same limit `emit_routed_registration` already enforces.
///
/// `rewrite_crate_segment` matters here in a way it doesn't for `wrap_prop_value`: this type is
/// spliced as an explicit turbofish (`register_routed_handler::<#payload_ty>`) into the *generated*
/// `@routed_from` arm, so it must resolve from whichever crate eventually invokes that arm — e.g.
/// `crate::input::KeyEventArgs` (as spelled in `UIElement`'s own `#[prop]` declaration, resolving
/// against `elwindui-core`) needs `$crate::` once embedded here. `wrap_prop_value`'s conversions
/// never have this problem: `.into()` needs no explicit target-type annotation of its own — Rust
/// infers it from the real setter's own parameter type at the call site.
/// Wraps a bare closure/callable `$value` (matching the DSL's own written arity, `elwindui-codegen`'s
/// ordinary callback-value convention — see `wrap_prop_value`'s doc comment) into the fixed
/// `Box<dyn Fn(&Payload, &RoutedEventArgs)>` shape `register_routed_handler` needs, ready to hand to
/// `@routed_from`. Arity comes from `ty`'s own `fn(..)` declaration, exactly like
/// `routed_payload_type` — a 0-parameter routed property (`on_click: fn()`) ignores the payload
/// entirely; a 1-parameter one (`on_key_down: fn(KeyEventArgs)`) calls `$value` with it dereferenced.
/// Both leave the payload parameter's own type unannotated, relying on `@routed_from`'s own
/// `register_routed_handler::<Payload>` turbofish to pin it down (verified: this inference reaches
/// through an unannotated closure parameter even across the `Box<dyn Fn>` coercion).
fn routed_handler_from_bare_callable(ty: &Type) -> TokenStream2 {
    let one_param = matches!(ty, Type::BareFn(f) if f.inputs.len() == 1);
    // `$value` is evaluated exactly once, into `__handler`, *before* the outer `move` closure --
    // not re-spliced as source text directly into the outer closure's body. Splicing `$value`
    // there would re-evaluate (re-construct) it on every invocation of the outer `Fn` closure,
    // which for a `move`-capturing DSL closure means re-*moving* its own captured state out of an
    // already-captured (by-reference-called) environment on the second call — exactly the "cannot
    // move out of a captured variable in an `Fn` closure" error this construction avoids.
    if one_param {
        quote! {
            {
                let __handler = $value;
                Box::new(move |__payload, _args: &$crate::input::RoutedEventArgs| {
                    (__handler)(*__payload);
                })
            }
        }
    } else {
        quote! {
            {
                let __handler = $value;
                Box::new(move |_payload, _args: &$crate::input::RoutedEventArgs| {
                    (__handler)();
                })
            }
        }
    }
}

fn routed_payload_type(ty: &Type) -> TokenStream2 {
    let Type::BareFn(f) = ty else {
        return quote! { () };
    };
    match f.inputs.len() {
        1 => {
            let arg_ty = &f.inputs[0].ty;
            rewrite_crate_segment(quote! { #arg_ty })
        }
        _ => quote! { () },
    }
}

/// The item-position path to a class's *own* shape macro. Always same-crate by construction, so it
/// is always `crate::` — the absolute form `inherit_macro_path` uses for the same reason (a bare
/// name resolves only within the defining file, which breaks the moment a class moves into a
/// submodule; see that function's own doc comment).
fn inherit_macro_path_for_self(bare_name: &str) -> TokenStream2 {
    let ident = props_macro_ident(bare_name);
    quote! { crate::#ident }
}

/// How a content property receives its children, decided by the property's declared type — the same
/// distinction `elwindui_codegen::codegen::build_component_setters` draws, moved to the declaration
/// that actually knows the type.
enum AttachKind {
    /// A live collection appended through its accessor (`UIElementCollection`, `ListExt<T>`).
    Collection,
    /// A `Vec<T>` property replaced wholesale by its setter.
    VecProperty,
    /// A single slot (`Rc<dyn ..>`) holding exactly one child.
    SingleSlot,
}

fn attach_kind(ty: &Type) -> AttachKind {
    let spelled = quote! { #ty }.to_string();
    if spelled.contains("UIElementCollection") || spelled.contains("ListExt") {
        AttachKind::Collection
    } else if spelled.trim_start().starts_with("Vec <") {
        AttachKind::VecProperty
    } else {
        AttachKind::SingleSlot
    }
}

/// Whether a `#[prop]`'s declared type is exactly `String` — see `build_props_macro`'s setter
/// argument convention.
fn is_string_type(ty: &Type) -> bool {
    matches!(ty, Type::Path(p) if p.qself.is_none() && p.path.is_ident("String"))
}

/// `ty`'s last path segment, ignoring any qualified-self (`<T as Trait>::X`) form — `#[prop]`
/// types are always plain paths (`crate::graphics::Brush`, `Option<T>`, ...), never that.
fn last_type_segment(ty: &Type) -> Option<&syn::PathSegment> {
    match ty {
        Type::Path(p) if p.qself.is_none() => p.path.segments.last(),
        _ => None,
    }
}

/// If `ty` is `Option<Inner>`, `Inner`'s own type; otherwise `None`.
fn option_inner(ty: &Type) -> Option<&Type> {
    let seg = last_type_segment(ty)?;
    if seg.ident != "Option" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(args) = &seg.arguments else {
        return None;
    };
    args.args.iter().find_map(|a| match a {
        syn::GenericArgument::Type(t) => Some(t),
        _ => None,
    })
}

/// Whether `ty`'s last path segment is literally `name` — a coarse, name-only check (not a real
/// path comparison), matching a `#[prop]` type regardless of which of its several equivalent
/// spellings a builtin happens to use (`crate::graphics::Brush` from within `elwindui-core`,
/// `elwindui::core::graphics::Brush` from a consumer, or a future bare `Brush` behind a `use`).
fn is_named(ty: &Type, name: &str) -> bool {
    last_type_segment(ty).is_some_and(|s| s.ident == name)
}

/// How a `#[prop]`'s DSL-authored value must be transformed before it reaches the real setter —
/// decided once, here, from the property's own declared `Type` (known at `#[class]` macro-expansion
/// time, i.e. once per *declaration*), instead of by `elwindui-codegen` once per *use site* (which,
/// once a builtin's shape lives only here, no longer has this type information available at all).
///
/// - `String`, bare or `Option`-wrapped — real hand-written-native/virtual-builtin setters take
///   `&str` by convention, never `Option<&str>`/`Option<String>` (an empty string is the "clear"
///   convention for these, e.g. `NativeControl::set_tooltip`/`TextBox::set_placeholder` — see their
///   own doc comments); `&(..)` derefs an owned `String` down to `&str`, and is a harmless reborrow
///   when the value is already a `&str` literal. Unlike `Brush`/`Color` below, the `Option`-wrapped
///   case is *not* additionally `Some(..)`-wrapped, since the real setter never takes `Option`.
/// - `Brush`/`Color`, bare or `Option`-wrapped — `.into()` lets a hex-string DSL literal
///   (`fill: "#3a3a3c"`) reach the setter via `Color`/`Brush`'s own `From<&str>` (elwindui-core),
///   and is a no-op (std's reflexive `From<T> for T`) when the value already is the target type.
///   `Option`-wrapped targets additionally need `Some(..)` — the DSL convention (matching
///   `elwindui-codegen`'s pre-existing `foreground`/`background` special-casing) is that a DSL
///   author always writes the *inner* value, never `Option` themselves.
/// - `Vec<T>` — a DSL array literal (`rows: [GridLength::Auto, ..]`) parses as a real Rust array
///   expression, not a `Vec`; `.to_vec()` converts (a no-op when the value is already a `Vec`,
///   e.g. an expression rather than a literal).
/// - anything else (`dyn UIElementExt`-typed content/attached values included) — passed through
///   unchanged. A concrete `Rc<Concrete: UIElementExt>` value coerces to `Rc<dyn UIElementExt>`
///   automatically at this call-argument position (ordinary Rust unsized coercion), so no explicit
///   conversion is needed here, unlike `elwindui-codegen`'s own `into_node_if_needed`.
fn wrap_prop_value(ty: &Type, value: TokenStream2) -> TokenStream2 {
    if is_string_type(ty) {
        return quote! { &(#value) };
    }
    if is_named(ty, "Vec") {
        return quote! { (#value).to_vec() };
    }
    // A plain (non-`#[routed]`) `fn(..)`-sugared callback prop's real setter takes a
    // `Box<dyn Fn(..)>` (`TabView.on_select`/`on_new_tab`, etc.) — `elwindui-codegen`'s
    // `emit_wiring`/`emit_external_construction` (both `#[routed]` and plain paths alike) hand this
    // arm a bare closure/callable value, matching what it already builds for a `#[routed]`
    // property's own `@routed_from`-bound handler (see this fn's own module-level context — the
    // `@set_from` doc comment on `p.routed`'s two branches). `Box::new(..)` coerces any concrete
    // closure type to the declared `Box<dyn Fn(..)>` at this call site, the same unsized-coercion
    // trick `into_node_if_needed`'s callers already rely on for trait-object arguments elsewhere.
    if matches!(ty, Type::BareFn(_)) {
        return quote! { Box::new(#value) };
    }
    let (target, wrap_in_some) = match option_inner(ty) {
        Some(inner) => (inner, true),
        None => (ty, false),
    };
    if is_string_type(target) {
        return quote! { &(#value) };
    }
    if is_named(target, "Brush") || is_named(target, "Color") {
        let converted = quote! { (#value).into() };
        return if wrap_in_some {
            quote! { Some(#converted) }
        } else {
            converted
        };
    }
    if let Some(bound) = rc_dyn_trait_bound(target) {
        if let Some(seg) = bound.bounds.iter().find_map(|b| match b {
            syn::TypeParamBound::Trait(t) => t.path.segments.last(),
            _ => None,
        }) {
            let name = seg.ident.to_string();
            let method = into_node_ident(name.strip_suffix("Ext").unwrap_or(&name));
            let converted = quote! { (#value).#method() };
            return if wrap_in_some {
                quote! { Some(#converted) }
            } else {
                converted
            };
        }
    }
    value
}

/// A single-slot content field's real setter (`build_props_macro`'s `@children_into` single-slot
/// arm — `content`/`submenu`/... — WPF/WinUI3's `ContentPropertyAttribute` pattern) takes `Rc<dyn
/// SomeTraitExt>` directly. Plain unsized coercion at that setter call already handles a real
/// builtin's own concrete value (`#[class]` gives it a direct `impl SomeTraitExt`), but not an
/// `#[elwindui::component]`-frontend user struct (e.g. `content: MyPanel { .. }`), which never
/// `impl`s that trait directly — only its own generated `into_node()` erases it. Calling
/// `.into_<bare(SomeTraitExt)>_node()` unconditionally works for both without this macro needing
/// to know which one it's looking at: every class this workspace generates — real builtins via
/// `#[class]`'s own `into_<name>_node` default methods (this file's `into_node_ident`/
/// `build_into_node_default`), and user components via their own, independently generated
/// `{Name}Ext` ancestor-forwarding chain — implements every ancestor trait it has directly, so it
/// always has that ancestor's own `into_<name>_node` default method (a trivial `{ self }` for
/// anything that already *is* the target type, the same identity result unsized coercion would
/// have produced). Deliberately **not** folded into `wrap_prop_value` (shared far more broadly by
/// `@set`/`@set_from`, including resync/rebind call sites where the value may already be an
/// erased trait object — `Self: Sized` on `into_<name>_node` excludes it from ever being callable
/// through a `dyn` receiver, so forcing this there breaks those) — this helper's one caller
/// (`children_into_arms`'s single-slot arm) only ever sees a freshly, concretely constructed
/// nested element, never an already-erased handle, so the call is always safe here. Found via a
/// real notepad screenshot regression (Refs #14): a `TabViewItem { DocumentView { .. } }` bare
/// child compiled fine but the `DocumentView` never actually reached the visual tree.
fn single_slot_child_value(ty: &Type, value: TokenStream2) -> TokenStream2 {
    let Some(bound) = rc_dyn_trait_bound(ty) else {
        return value;
    };
    let Some(method) = bound
        .bounds
        .iter()
        .find_map(|b| match b {
            syn::TypeParamBound::Trait(t) => t.path.segments.last(),
            _ => None,
        })
        .map(|seg| {
            let name = seg.ident.to_string();
            into_node_ident(name.strip_suffix("Ext").unwrap_or(&name))
        })
    else {
        return value;
    };
    quote! { (#value).#method() }
}

/// If `ty` is `std::rc::Rc<dyn SomeTrait>`, returns that `dyn SomeTrait` bound — see
/// `single_slot_child_value`'s own doc comment on why this matters. Deliberately narrower than
/// `content_item_dyn_type`'s own recursive trait-object search (which also matches list-shaped
/// wrappers like `ListExt<dyn X>`, a fundamentally different, per-item construction path this has
/// no business touching): only a bare `Rc<dyn X>` denotes a single embedded node here.
fn rc_dyn_trait_bound(ty: &Type) -> Option<&syn::TypeTraitObject> {
    let Type::Path(p) = ty else { return None };
    let seg = p.path.segments.last()?;
    if seg.ident != "Rc" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(args) = &seg.arguments else {
        return None;
    };
    args.args.iter().find_map(|a| match a {
        syn::GenericArgument::Type(Type::TraitObject(to)) => Some(to),
        _ => None,
    })
}

/// PR #164 final remediation round (finding A6): unresolved routing metadata to the *next* ancestor
/// named by a class's own `inherits = Parent<Args>` — deliberately carries no semantic Ext-trait
/// path at all. `build_inherit_macros`'s generated classify/skip macros are reused for *every*
/// future descendant depth, not just the immediate `inherits = ..` edge — whether `Parent`'s own
/// trailing generic arguments (`concrete_args`) apply to `Parent`'s real interface trait is a
/// decision only `Parent`'s own `__elwindui_apply_own_ext_*!` policy macro can make correctly, at
/// real macro-expansion time, once whatever future descendant actually reaches this hop is known —
/// never pre-baked here via a same-crate registry (`same_crate_classes`), which is empty across real
/// crate boundaries and so cannot answer this correctly beyond the immediate hop. This replaces the
/// former `ancestor_own_trait`/`struct_only_ancestor_in_this_crate` same-crate-registry heuristic,
/// which only ever got the immediate `inherits = ..` edge right (fixed separately, by
/// `expand_impl`'s own prelude, PR #164's earlier A5 fix) — every *recursive* hop beyond that one
/// still baked a same-crate-registry-derived guess as a literal token, silently wrong across real
/// crate boundaries (confirmed: a 3+ hop generic `struct_only` chain reattached `<Args>` onto a
/// non-generic real interface once the ancestor stopped being in the same crate as the reattaching
/// hop).
struct AncestorRoute {
    /// `path_module_prefix(inh)`, rewritten — the wrapper module `Parent`'s own `__ElwindUIOwnExt`
    /// alias, entry/skip/classify trio, bridge, and policy macro are all reached through.
    own_ext_prefix: TokenStream2,
    /// `last_segment_generic_args(inh)`, rewritten — this class's own `inherits = ..` argument's
    /// trailing generic arguments exactly as written (e.g. `<ConsumerSource>`), handed to `Parent`'s
    /// own policy macro to keep or discard.
    concrete_args: TokenStream2,
    /// `#inh`, rewritten — `Parent`'s own concrete type, exactly as this class named it.
    concrete: TokenStream2,
    /// `ancestor_bridge_path(parent_bare, inh)`, rewritten.
    bridge_path: TokenStream2,
    /// `own_ext_policy_path(parent_bare, inh)`, rewritten.
    policy_path: TokenStream2,
}

/// Builds `parent_bare`/`inh`'s (an `inherits = ..` target) `AncestorRoute` — shared by `expand_impl`
/// and `expand_trait_only`, the two producers of `build_inherit_macros`'s own `recurse_next`
/// metadata (PR #164 final remediation round, finding A6). Every field is
/// `rewrite_crate_segment`-treated: all of them end up embedded as literal tokens inside a
/// `#[macro_export]`'d macro body (the continuation macro `build_inherit_macros` generates), later
/// re-expanded from whatever crate an eventual descendant lives in — not necessarily the crate that
/// generated this route.
fn build_ancestor_route(parent_bare: &str, inh: &Type) -> AncestorRoute {
    let own_ext_prefix = rewrite_crate_segment(path_module_prefix(inh).expect(
        "#[class]: internal: inherits argument should already be validated fully-qualified",
    ));
    let concrete_args = rewrite_crate_segment(last_segment_generic_args(inh));
    let concrete = rewrite_crate_segment(quote! { #inh });
    let bridge_path = rewrite_crate_segment(ancestor_bridge_path(parent_bare, inh));
    let policy_path = rewrite_crate_segment(own_ext_policy_path(parent_bare, inh));
    AncestorRoute {
        own_ext_prefix,
        concrete_args,
        concrete,
        bridge_path,
        policy_path,
    }
}

fn build_inherit_macros(
    bare_name: &str,
    dyn_ident: &Ident,
    has_concrete_type: bool,
    skip_own_impl: bool,
    overridable_names: &[Ident],
    // PR #164 final remediation round (finding C2): unlike `named_accessor` (whose own
    // `$OwnConcrete` genuinely varies per hop/caller, always correctly, matching whatever `self.base`
    // actually is at that hop), `extra_required_names` exists *only* for root mode's own
    // `as_ui_element`, which always means the same thing regardless of hop depth: "reach the one true
    // root instance", not "reach my own direct base". A *normal* (non-bridged) chain never surfaces
    // any difference — every intermediate hop's own `next_concrete` recomputation transitively still
    // points at the same, single root type by the time this class's own classify chain runs — but a
    // `struct_only` bridge forward (`entry_override`) does NOT recompute `$OwnConcrete` the same way
    // (`named_accessor` specifically needs it left alone, matching whatever the struct_only class's
    // own reflexive accessor returns) — so `extra_forwards` needs its own, separately-threaded
    // parameter (`$RootConcrete`, added to every entry/classify macro this function generates)
    // instead of reusing `$OwnConcrete` — confirmed empirically (`E0308`, return-type mismatch)
    // against a real multi-hop root-mode `struct_only` bridge fixture (`BridgeRootDerived`,
    // `testsupport.rs`) before this fix.
    extra_required_names: &[Ident],
    // The path to the *next* ancestor's own entry macro (self-ref position — this function always
    // embeds it inside a `macro_rules!` body it generates, whether used directly, `recurse_next:
    // None`, or indirectly through the continuation macro `recurse_next: Some` generates). Ignored
    // when `recurse_next` is `None` (the terminal case recurses into its own local check instead).
    recurse_macro_path: &TokenStream2,
    // PR #164 final remediation round (finding A6): `AncestorRoute`, not a pre-resolved
    // `(next_trait, next_concrete, next_bridge_path)` tuple — see `AncestorRoute`'s own doc comment
    // for why the semantic trait can never be baked in here.
    recurse_next: Option<&AncestorRoute>,
    // Issue #128: overrides the entry macro's own body (`$SubType`/`$OwnTrait`/`$OwnConcrete`/
    // `$BridgePath`/`$overrides` all still in scope) instead of the default "classify against my own
    // locally-known `overridable_names`" behavior — used exactly once, by `expand_impl`'s struct_only
    // branch, to redirect a forwardable `struct_only` implementor's entry macro into its interface's
    // `__elwindui_class_interface_*!` bridge (`@inherit_through_struct_only`) instead, since a
    // `struct_only` class's own locally-known `overridable_names` is never the right classification
    // source for the interface it implements. `$BridgePath` itself is never baked as a literal token
    // here — it travels as a macro parameter for exactly the same reason `$OwnTrait`/`$OwnConcrete`
    // do (see `ancestor_bridge_path`'s own doc comment): a path baked once, inside a
    // macro body invoked from arbitrary third crates, resolves against whichever crate re-invokes
    // it, not the crate that originally generated the body. `None` (every other caller) preserves
    // this function's original, unconditional behavior exactly.
    entry_override: Option<TokenStream2>,
) -> TokenStream2 {
    let entry_ident = inherit_macro_ident(bare_name);
    let skip_ident = inherit_macro_skip_ident(bare_name);
    let classify_ident = inherit_macro_classify_ident(bare_name);

    let named_accessor = has_concrete_type.then(|| {
        let accessor_ident = named_accessor_ident(bare_name);
        // A root-mode ancestor never generates its own reflexive `as_<bare_name>` inherent
        // method (`expand_impl`'s own `reflexive_named_accessor`, `!is_root_mode`-gated — root's
        // one true reflexive accessor is the hardcoded `as_ui_element` trait method instead). The
        // outward-facing name this macro exposes to descendants still needs to stay
        // `as_<bare_name>` uniformly (so a descendant can always call `as_<ancestor>()` regardless
        // of whether that ancestor happens to be root or ordinary) — only the *call target* into
        // `self.base` needs to redirect to whatever reflexive method actually exists. Root mode is
        // the only caller of this function that ever passes a non-empty `extra_required_names`
        // (always exactly `[as_ui_element]`, see that parameter's own doc comment above), so
        // reusing its first element here (instead of a further dedicated parameter) is safe and
        // exact — confirmed by a real fixture (`BridgeFixtureGenericOrdinaryBase`, a
        // non-`UIElement`-named root class with a *direct* ordinary descendant) that only ever
        // exercised this path once its own bare name stopped snake-casing to coincidentally match
        // `as_ui_element` (as `UIElement` itself always has, masking this bug until now).
        //
        // The *return type* must switch along with it, from `$OwnConcrete` to `$RootConcrete`:
        // for a normal (non-bridged) direct root descendant the two are identical (`extra_forwards`'
        // own doc comment), so this is a no-op there, but across a `struct_only` bridge they
        // genuinely diverge (`$OwnConcrete` stays the bridge's own concrete type, e.g.
        // `BridgeRootConcrete`; `self.base.as_ui_element()` returns the actual root type, e.g.
        // `&BridgeRootBase`) — confirmed by `E0308` against the real `BridgeRootBase` ->
        // `BridgeRootConcrete` -> `BridgeRootDerived` fixture before this fix.
        let is_root_ancestor = !extra_required_names.is_empty();
        let call_target = extra_required_names.first().unwrap_or(&accessor_ident);
        let return_ty = if is_root_ancestor {
            quote! { $RootConcrete }
        } else {
            quote! { $OwnConcrete }
        };
        quote! {
            impl $SubType {
                #[allow(dead_code)]
                pub fn #accessor_ident(&self) -> &#return_ty { self.base.#call_target() }
            }
        }
    });

    let extra_forwards: Vec<TokenStream2> = extra_required_names
        .iter()
        .map(|name| quote! { fn #name(&self) -> &$RootConcrete { self.base.#name() } })
        .collect();
    let extra_forwards = &extra_forwards;

    // PR #164 final remediation round (finding A6): recursing to the next layer no longer bakes a
    // pre-resolved ancestor trait as a literal token here (`next_trait`, formerly computed by the
    // caller via `ancestor_own_trait`'s same-crate-registry heuristic — wrong across real crate
    // boundaries, since this class's own generated classify/skip macros are reused for *every*
    // future descendant depth, not just the immediate one a registry lookup might happen to know
    // about). Instead, a fixed, once-per-class continuation macro (`continuation_ident`, generated
    // below when `recurse_next: Some`, never duplicated per future invocation — unlike a
    // per-invocation name, which multiple real future descendants reaching this same hop would
    // collide on) asks the next ancestor's own `__elwindui_apply_own_ext_*!` policy macro whether its
    // own generic arguments apply, at the point some real descendant chain actually reaches this
    // hop — the exact same two-mechanism protocol `expand_impl`'s own prelude already uses for the
    // immediate hop (`own_ext_policy_path`/`own_ext_bound_trait_ident`'s own doc comments); this is
    // that same protocol applied recursively, not a new one.
    let continuation_ident = format_ident!("__elwindui_continue_recursive_inherits_of_{bare_name}");
    // No further ancestor: recurse into a *local* (same-crate, `$crate::`-self-referenced) leftover-
    // `#[overrides]` check instead of the caller-supplied `recurse_macro_path` (ignored in this
    // case) — every "no inherits" class (true root mode's `UIElement`, or any other struct_only/
    // ordinary class with no `inherits` at all, e.g. `Window`) generates its own copy of this
    // rather than all sharing one fixed, `elwindui-core`-only macro: a single shared terminal macro
    // would need every *other* crate's own classify macro to reference it by a path valid from
    // whatever *third* crate eventually invokes that classify macro (e.g. `notepad` invoking
    // `elwindui-backend-appkit`'s `Window` chain, which used to recurse into a macro path baked in
    // at `elwindui-backend-appkit`'s own compile time, using *its* view of how to reach
    // `elwindui-core` — valid there, but not necessarily from `notepad`, which only depends on the
    // `elwindui` facade). A same-crate `$crate::` self-reference has no such problem, since it
    // always resolves against the crate that generated *this* classify macro, regardless of caller.
    let (recurse_macro_path, terminal_check, continuation_macro) = match recurse_next {
        None => {
            let terminal_ident = format_ident!("__elwindui_inherit_{bare_name}_terminal");
            let path = quote! { $crate::#terminal_ident };
            let check = quote! {
                #[doc(hidden)]
                #[macro_export]
                #[allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]
                macro_rules! #terminal_ident {
                    ($SubType:ty;) => {};
                    ($SubType:ty; $($leftover:tt)+) => {
                        compile_error!(concat!(
                            "#[overrides]: no ancestor declared these methods #[overridable]: ",
                            stringify!($($leftover)+)
                        ));
                    };
                }
            };
            (path, Some(check), None)
        }
        Some(route) => {
            // `own_ext_prefix`/`concrete`/`bridge_path` are baked here (proc-macro-time-known,
            // reliably the same every time this hop is reached) — only the resolved trait tokens
            // themselves (`$($OwnTraitTail)*`) come from the policy macro, fresh, per invocation.
            let own_ext_prefix = &route.own_ext_prefix;
            let concrete = &route.concrete;
            let bridge_path = &route.bridge_path;
            let alias_ident = own_ext_alias_ident();
            let path = recurse_macro_path.clone();
            let continuation = quote! {
                #[doc(hidden)]
                #[macro_export]
                #[allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]
                macro_rules! #continuation_ident {
                    ($SubType:ty, [$($OwnTraitTail:tt)*]; $($unmatched:tt)*) => {
                        #path!($SubType, #own_ext_prefix::#alias_ident $($OwnTraitTail)*, #concrete, #concrete, #bridge_path; $($unmatched)*);
                    };
                }
            };
            (path, None, Some(continuation))
        }
    };
    let recurse_macro_path = &recurse_macro_path;

    // The actual "recurse past me" call embedded in `skip_ident`'s and classify's base-case bodies
    // below — routed through the next ancestor's own policy macro (`recurse_next: Some`) or straight
    // to the local terminal check (`recurse_next: None`). `trailing` is the differing tail-token
    // capture each of the two call sites below supplies (`$($overrides)*`/`$($unmatched)*`).
    let recurse_call = |trailing: TokenStream2| -> TokenStream2 {
        match recurse_next {
            None => quote! { #recurse_macro_path!($SubType; #trailing); },
            Some(route) => {
                let policy_path = &route.policy_path;
                let concrete_args = &route.concrete_args;
                quote! {
                    #policy_path!($crate::#continuation_ident, $SubType; [#concrete_args]; #trailing);
                }
            }
        }
    };

    // One independent "slot" per `#[overridable]` name, each capturing 0-or-1 `item` (the matching
    // `#[overrides]` body, if this hop provided one) — replaces a single flat `[$($matched:item)*]`
    // bag (see `own_impl`'s own comment below for why one shared slot/accessor per trait is
    // insufficient once a hop can override *some but not all* of a trait's overridable methods). A
    // class with no `#[overridable]` methods of its own (the overwhelming majority) gets zero
    // slots, making every pattern/expansion below identical to before this mechanism existed.
    let slot_idents: Vec<Ident> = (0..overridable_names.len())
        .map(|i| format_ident!("__slot_{i}"))
        .collect();
    // Pattern position: `[$( $__slot_i:item )?]` — captures this slot generically, whether or not
    // this particular arm is the one that fills it.
    let slot_patterns: Vec<TokenStream2> = slot_idents
        .iter()
        .map(|s| quote! { [$( $#s:item )?] })
        .collect();
    // Expansion position: `[$( $__slot_i )?]` — re-embeds whatever this slot already held,
    // unchanged (used by every arm that doesn't touch this particular slot).
    let slot_passthroughs: Vec<TokenStream2> =
        slot_idents.iter().map(|s| quote! { [$( $#s )?] }).collect();
    // The entry macro's own initial call: one literal empty `[]` per slot (no `$`/metavariable
    // involved here — this is the concrete starting value classify's own patterns above then match
    // against).
    let empty_slots: Vec<TokenStream2> = slot_idents.iter().map(|_| quote! { [] }).collect();

    // `$crate::#classify_ident!` (not a bare `#classify_ident!`) in every self-reference below --
    // required whenever the entry/classify macros defined *here* are invoked from a *different*
    // crate than this one: a bare reference inside a `macro_rules!` body only resolves within the
    // crate it's textually invoked from, but `$crate` always resolves back to the crate that
    // *defined* the macro, regardless of who calls it (ordinary `macro_rules!` hygiene).
    let mut classify_arms = Vec::new();
    for (i, name) in overridable_names.iter().enumerate() {
        let mut expansion_slots = slot_passthroughs.clone();
        expansion_slots[i] = quote! { [$($body)*] };
        classify_arms.push(quote! {
            ($SubType:ty, $OwnTrait:path, $OwnConcrete:path, $RootConcrete:path; #(#slot_patterns)*; [$($unmatched:tt)*]; #name => { $($body:item)* } $(, $($rest:tt)*)?) => {
                $crate::#classify_ident!($SubType, $OwnTrait, $OwnConcrete, $RootConcrete; #(#expansion_slots)*; [$($unmatched)*]; $($($rest)*)?);
            };
        });
    }

    // `skip_own_impl` (`no_ancestor_forward`): this hop's own trait is hand-written and predates
    // the `__dyn_x` convention, so it has no `#dyn_ident` method to implement -- omit the `impl`
    // entirely, but still recurse into whatever this hop itself further inherits (below).
    //
    // Each `#[overridable]` method gets its *own* dedicated accessor (`per_method_accessor_ident`),
    // resolved here via `$crate::#resolve_ident!` — one shared accessor per *trait* (the original
    // design) can't correctly represent "closest override" independently per method: a hop that
    // overrides method A but not method B needs *other*, non-overriding descendants' default
    // dispatch for A to stop at this hop, while their dispatch for B must still continue past it —
    // a single boolean-shaped accessor (reflexive vs. forwarding) can't satisfy both at once (found
    // via a real 3-hop test; see `docs/specs/macro_class_spec.md` §14's note on this). Any
    // method *not* declared `#[overridable]` has only ever had one real implementor (the declaring
    // class), so `#dyn_ident`'s original "reflexive there, forward everywhere else" shape remains
    // correct and unchanged for it.
    let resolve_ident = format_ident!("__elwindui_inherit_{bare_name}_resolve");
    let resolve_calls: Vec<TokenStream2> = slot_idents
        .iter()
        .zip(overridable_names.iter())
        .map(|(slot, name)| {
            let accessor = per_method_accessor_ident(name);
            quote! { $crate::#resolve_ident!($OwnTrait, #accessor; $( $#slot )?); }
        })
        .collect();
    let resolve_macro = (!overridable_names.is_empty()).then(|| {
        quote! {
            #[doc(hidden)]
            #[macro_export]
            #[allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]
            macro_rules! #resolve_ident {
                // No override at this hop for this one method -- forward to whatever `self.base`
                // itself resolves to (another override further up, or the original declaring
                // class's own reflexive one).
                ($OwnTrait:path, $accessor:ident;) => {
                    fn $accessor(&self) -> &dyn $OwnTrait { self.base.$accessor() }
                };
                // Overridden at this hop -- reflexive (this hop *is* the closest override for this
                // one method), plus the real override body itself.
                ($OwnTrait:path, $accessor:ident; $item:item) => {
                    fn $accessor(&self) -> &dyn $OwnTrait { self }
                    $item
                };
            }
        }
    });
    let own_impl = (!skip_own_impl).then(|| {
        quote! {
            impl $OwnTrait for $SubType {
                fn #dyn_ident(&self) -> &dyn $OwnTrait { self.base.#dyn_ident() }
                #(#extra_forwards)*
                #(#resolve_calls)*
            }
        }
    });

    // `#[allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]`: this whole mechanism
    // depends on a `$crate::`-qualified macro-to-macro reference (needed for the cross-crate case,
    // see the comment above) still working when the reference is actually same-crate too (where
    // `$crate` expands to the same "absolute path" form this lint flags) — currently accepted with
    // a warning, "planned to become a hard error in a future rustc release" per the lint's own
    // message (rust-lang/rust#52234). If a future rustc version removes this allowance entirely,
    // every macro-to-macro self-reference in this function needs a different mechanism.
    // Same-module `pub use` self-re-exports for these three macros (plus the sealed-check macro)
    // are built separately, by the caller, into one shared wrapper module — see
    // `macro_reexport_mod_ident`'s own doc comment for why. `#resolve_ident` doesn't need this
    // treatment: it's only ever `$crate::`-self-referenced from *this* class's own classify macro,
    // never named directly by another class's own generated code the way `entry`/`skip` are.
    // `$BridgePath` is accepted by every entry macro unconditionally (whether or not `entry_override`
    // is `Some`) so that every caller can compute and pass it the same way, with no need to know in
    // advance whether the ancestor they're naming happens to be a forwardable `struct_only`
    // implementor — see `ancestor_bridge_path`'s own doc comment. The default body
    // (classify invocation) simply never references it.
    let entry_body = entry_override.unwrap_or_else(|| {
        quote! {
            $crate::#classify_ident!($SubType, $OwnTrait, $OwnConcrete, $RootConcrete; #(#empty_slots)*; []; $($overrides)*);
        }
    });

    let skip_recurse_call = recurse_call(quote! { $($overrides)* });
    let classify_recurse_call = recurse_call(quote! { $($unmatched)* });

    quote! {
        #[doc(hidden)]
        #[macro_export]
        #[allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]
        macro_rules! #entry_ident {
            ($SubType:ty, $OwnTrait:path, $OwnConcrete:path, $RootConcrete:path, $BridgePath:path; $($overrides:tt)*) => {
                #entry_body
            };
        }

        #[doc(hidden)]
        #[macro_export]
        #[allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]
        macro_rules! #skip_ident {
            ($SubType:ty; $($overrides:tt)*) => {
                #skip_recurse_call
            };
        }

        #[doc(hidden)]
        #[macro_export]
        #[allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]
        macro_rules! #classify_ident {
            #(#classify_arms)*
            // Head belongs to some other ancestor -- keep it for the next layer of recursion.
            ($SubType:ty, $OwnTrait:path, $OwnConcrete:path, $RootConcrete:path; #(#slot_patterns)*; [$($unmatched:tt)*]; $name:ident => { $($body:item)* } $(, $($rest:tt)*)?) => {
                $crate::#classify_ident!($SubType, $OwnTrait, $OwnConcrete, $RootConcrete; #(#slot_passthroughs)*; [$($unmatched)* $name => { $($body)* },]; $($($rest)*)?);
            };
            // Base case: emit the impl (required accessor + one resolved accessor per overridable
            // method), the named accessor (if any), then recurse with whatever nobody claimed yet.
            ($SubType:ty, $OwnTrait:path, $OwnConcrete:path, $RootConcrete:path; #(#slot_patterns)*; [$($unmatched:tt)*];) => {
                #own_impl
                #named_accessor
                #classify_recurse_call
            };
        }

        #continuation_macro
        #resolve_macro
        #terminal_check
    }
}

/// The per-class module `path_module_prefix` routes a cross-module macro reference through —
/// e.g. `Window` -> `__elwindui_macros_of_Window`. `#[macro_export]` unavoidably places a macro at
/// *this* crate's root (there is no way to opt out) — sufficient for a `$crate::`-qualified self-
/// reference or a caller with this crate as a *direct* dependency, but NOT, by itself, reachable
/// through any module path re-exporting the surrounding module's contents (confirmed empirically:
/// referencing the macro via its own textual module path, with no explicit `pub use` there, fails
/// to resolve even from within the defining crate). An explicit same-module `pub use` fixes that —
/// but writing it at the *exact* same scope as the `macro_rules!` itself collides (E0255) whenever
/// that scope happens to already be this crate's own root (`#[macro_export]`'s forced placement and
/// the explicit `pub use` would then both be binding the identical name in the identical scope) —
/// which is exactly the case for this codebase's backend crates' raw leaf types, declared directly
/// at their crate's `lib.rs` top level. Routing the `pub use` through a small per-class wrapper
/// module — always a distinct scope, whether the class itself sits at crate root or three modules
/// deep — sidesteps that collision unconditionally, with no need to detect (impossible, from within
/// a proc-macro) whether a given expansion site is the crate root.
fn macro_reexport_mod_ident(bare_name: &str) -> Ident {
    format_ident!("__elwindui_macros_of_{bare_name}")
}

/// Issue #128: the wrapper module a `trait_only` class's own `__elwindui_class_interface_*!` bridge
/// macro is `pub use`-re-exported through — deliberately a *different* name from
/// `macro_reexport_mod_ident`'s own `__elwindui_macros_of_<bare_name>` (which a `struct_only`
/// implementor of *the same bare name*, in a completely different crate, also generates for
/// itself — e.g. `elwindui-core`'s `trait_only Window` and each backend's own `struct_only Window`
/// deliberately share the bare name "Window", see this module's own top doc comment). Facade crates
/// that glob-merge a backend crate's own top-level items together with `elwindui-core::ui`'s (e.g.
/// `elwindui`'s own `pub mod ui { pub use elwindui_backend_appkit::*; pub use elwindui_core::ui::*;
/// }`) would otherwise hit an ambiguous-glob-reexport collision between the two same-named wrapper
/// modules — confirmed empirically. Only ever needs to carry the bridge macro itself (unlike
/// `macro_reexport_mod_ident`'s wrapper, nothing outside this class's own generated code ever
/// references its entry/skip/classify macros by an external module path — every reference to those
/// from the bridge macro's own body is a same-crate `$crate::` self-reference instead).
fn class_interface_wrapper_mod_ident(bare_name: &str) -> Ident {
    format_ident!("__elwindui_class_interface_macros_of_{bare_name}")
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
            if let Some(Type::Path(tp)) = &args.inherits {
                if let Err(e) = validate_fully_qualified_path(&tp.path, "inherits") {
                    return e.to_compile_error();
                }
            }
            if let Some(p) = &args.struct_only {
                if let Err(e) = validate_fully_qualified_path(p, "struct_only") {
                    return e.to_compile_error();
                }
            }
            let owns_inherit_macros = register_same_crate_class(
                &item_struct.ident.to_string(),
                &args.struct_only,
                args.no_ancestor_forward,
                false,
            );
            store_class_args(
                &item_struct.ident.to_string(),
                &args,
                owns_inherit_macros,
                &item_struct.vis,
            );
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

/// The path prefix that reaches `elwindui-core` from whichever crate `#[class]` is currently
/// expanding within, determined generically via `proc-macro-crate` (inspecting the *currently
/// compiling* crate's own `Cargo.toml`) rather than hardcoding crate names — any crate that ends
/// up depending on `elwindui-core`, directly or only transitively through the `elwindui` facade,
/// resolves correctly with no changes to this function.
fn core_path() -> TokenStream2 {
    use proc_macro_crate::{FoundCrate, crate_name};
    match crate_name("elwindui-core") {
        Ok(FoundCrate::Itself) => quote! { crate },
        Ok(FoundCrate::Name(name)) => {
            let ident = format_ident!("{name}");
            quote! { #ident }
        }
        Err(_) => match crate_name("elwindui").expect("#[class]: this crate depends on neither `elwindui-core` nor `elwindui` — cannot resolve a path to elwindui-core") {
            FoundCrate::Itself => quote! { crate::core },
            FoundCrate::Name(name) => {
                let ident = format_ident!("{name}");
                quote! { #ident::core }
            }
        },
    }
}

fn expand_struct(args: &ClassArgs, item: syn::ItemStruct) -> TokenStream2 {
    let class_name = &item.ident;
    let vis = &item.vis;
    // Same inert-marker split `expand_trait_only` does — a builtin whose real implementation is an
    // ordinary `struct` in this crate (`UIElement`/`Layout`/`Grid`/...) declares its DSL surface
    // here rather than on a `trait_only` interface. See `build_props_macro`.
    let (prop_decls, attrs) = match take_prop_decls(&item.attrs) {
        Ok(split) => split,
        Err(e) => return e.to_compile_error(),
    };
    let attrs = &attrs;
    let generics = &item.generics;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let existing_fields: Vec<Field> = match &item.fields {
        Fields::Named(named) => named.named.iter().cloned().collect(),
        Fields::Unit => Vec::new(),
        Fields::Unnamed(_) => {
            return syn::Error::new_spanned(
                &item,
                "#[class]: struct must use named fields, not a tuple struct",
            )
            .to_compile_error();
        }
    };

    // `pub` (matching H.2.1a's convention): plenty of call sites reach through `.base` across
    // module/crate boundaries (`self.base.handle`, `self.base.as_ui_element()`, ...). The field's
    // type is the `inherits = ..` target exactly as written — struct names are never rewritten, so
    // no suffix transform is needed here.
    let base_field = args.inherits.as_ref().map(|ty| quote! { pub base: #ty, });

    // rust-analyzer-only: gives its autoderef-based method resolution a way to walk the whole
    // ancestor chain as plain inherent methods (each ancestor class emits this same `Deref` for its
    // own `base` field in turn), entirely independent of the `{ClassName}Ext` traits/
    // `__elwindui_inherit_*!` macro chain real builds use for the same purpose — see
    // `build_rust_analyzer_shadow`'s own doc comment and docs/specs/macro_class_spec.md §15 for
    // why. Unlike `expand_impl`, this function has no cross-invocation (`class_arg_store`)
    // dependency of its own — it always succeeds from its own single invocation's args alone,
    // regardless of rust-analyzer's expansion order, so nothing extra is needed to make this part
    // reliable.
    let shape_macro = prop_decls.is_declared().then(|| {
        let parent = args
            .inherits
            .as_ref()
            .and_then(|ty| last_segment_name(ty).map(|name| (name, ty)));
        build_props_macro(
            &class_name.to_string(),
            &prop_decls,
            parent.as_ref().map(|(name, ty)| (name.as_str(), *ty)),
        )
    });

    let deref_shadow = args.inherits.as_ref().map(|ty| {
        quote! {
            #[cfg(rust_analyzer)]
            #[allow(unexpected_cfgs)]
            impl #impl_generics std::ops::Deref for #class_name #ty_generics #where_clause {
                type Target = #ty;
                fn deref(&self) -> &Self::Target { &self.base }
            }
        }
    });

    quote! {
        #(#attrs)*
        #vis struct #class_name #generics {
            #base_field
            #(#existing_fields,)*
        }
        #deref_shadow
        #shape_macro
    }
}

/// `trait_only` (see this module's own doc comment): a single, self-contained invocation on a bare
/// `trait ClassName { .. }` item — the user's own trait items (already ordinary, bodyless trait
/// methods) pass through unchanged; the macro renames the trait to `ClassNameExt` and adds the
/// supertrait bound (`inherits = ..` if given, rewritten to `..Ext`, or `base::AsAny` automatically
/// otherwise, mirroring root class mode's own "no ancestor -> `AsAny`" rule in `expand_impl`). No
/// paired `struct`/`impl` is expected — there's no `store_class_args`/`load_class_args` round-trip
/// here at all. Also emits this class's own `__elwindui_inherit_*!` trio, same as `expand_impl`
/// does for an ordinary class — see `build_inherit_macros`.
fn expand_trait_only(args: &ClassArgs, item: syn::ItemTrait) -> TokenStream2 {
    if !args.trait_only {
        let msg = "#[class]: a bare `trait ClassName { .. }` item is only valid with `trait_only` — declare a \
                   `struct ClassName { .. }` for an ordinary class, or add `trait_only` here for a pure \
                   marker/interface trait";
        return syn::Error::new_spanned(&item, msg).to_compile_error();
    }
    let class_name = &item.ident;
    let bare_name = class_name.to_string();
    let ext_name = to_ext_ident(&bare_name, class_name.span());
    let vis = &item.vis;
    // `#[prop(..)]` are inert markers consumed here — they declare this builtin's
    // DSL surface (see `build_props_macro`) and must not survive onto the generated trait.
    let (prop_decls, attrs) = match take_prop_decls(&item.attrs) {
        Ok(split) => split,
        Err(e) => return e.to_compile_error(),
    };
    let attrs = &attrs;
    let generics = &item.generics;
    let where_clause = &item.generics.where_clause;
    let items = &item.items;
    let user_supertraits = &item.supertraits;
    let core = core_path();
    let bound_ty: Type = match &args.inherits {
        Some(t) => ext_trait_type(t),
        None => syn::parse2(quote! { #core::base::AsAny })
            .expect("#[class]: internal: failed to build AsAny bound"),
    };
    let bound = quote! { #bound_ty };
    let colon_bound = if user_supertraits.is_empty() {
        quote! { : #bound }
    } else {
        quote! { : #user_supertraits + #bound }
    };
    let owns_inherit_macros = register_same_crate_class(&bare_name, &None, false, true);
    let sigs: Vec<syn::Signature> = items
        .iter()
        .filter_map(|item| match item {
            syn::TraitItem::Fn(f) => Some(f.sig.clone()),
            _ => None,
        })
        .collect();
    // Issue #128: `#[overridable]` *is* meaningful on a `trait_only` method (unlike `sigs` above,
    // which only ever carries bodyless signatures with no attributes at all) — collected separately
    // here, from `items`' own `f.attrs`, since previously this was the exact point that silently
    // discarded it (root cause of #128: nothing downstream, same-crate or cross-crate, could ever
    // see that a `trait_only` method was declared overridable). Never spliced back into `sigs`/the
    // generated trait declaration itself — `dyn_methods`/`build_class_interface_bridge_macro` below
    // are the only consumers.
    let overridable_names: Vec<Ident> = items
        .iter()
        .filter_map(|item| match item {
            syn::TraitItem::Fn(f) if f.attrs.iter().any(|a| a.path().is_ident("overridable")) => {
                Some(f.sig.ident.clone())
            }
            _ => None,
        })
        .collect();
    let dyn_ident = dyn_accessor_ident(&bare_name);
    let ext_ty = quote! { #ext_name };
    // `ext_ty` never carries generics here (no `#generics`/`ty_generics` splice above, even when
    // the `trait_only` trait itself declares its own) — call-position and type-position are
    // already identical, so no separate turbofish form is needed.
    let dyn_methods =
        build_dyn_default_methods(&dyn_ident, &ext_ty, &ext_ty, &sigs, &overridable_names);
    let into_node_ident = into_node_ident(&bare_name);
    let into_node_default = build_into_node_default(&into_node_ident, &ext_ty);

    // Issue #128: unlike before, `trait_only` now *does* generate its own `__elwindui_inherit_*!`
    // trio, plus the matching `macro_reexport_mod_ident` wrapper module and a
    // `__elwindui_class_interface_*!` bridge macro (`build_class_interface_bridge_macro`) — the
    // cross-crate transport a forwardable `struct_only` implementor's own entry macro redirects
    // into (`expand_impl`'s `entry_override`), so an `#[overridable]` method declared here is
    // reachable by an ordinary descendant reached through that `struct_only` link, exactly like an
    // ordinary `inherits = ..` chain. Nothing outside this bridge mechanism ever names a `trait_only`
    // class's own bare (pre-rename) name as an `inherits = ..` target directly — every backend still
    // implements the shared interface via `struct_only = ..Ext` instead — so this trio's own *entry*
    // macro is, in production, only ever invoked from inside a `struct_only` implementor's own
    // `entry_override` (via the bridge), never named directly by a further `inherits = ..`. Gated on
    // `owns_inherit_macros` (a `trait_only` bare name can, like any other, lose the same-crate
    // ownership race — see `register_same_crate_class`'s own doc comment) exactly as `expand_impl`
    // gates its own trio.
    let inherit_macros_and_bridge = owns_inherit_macros.then(|| {
        let (recurse_macro_path, recurse_next) = match &args.inherits {
            Some(inh) => {
                let parent_bare = last_segment_name(inh).unwrap_or_default();
                let path = inherit_macro_self_ref_path(
                    &parent_bare,
                    inh,
                    inherit_macro_ident(&parent_bare),
                );
                (path, Some(build_ancestor_route(&parent_bare, inh)))
            }
            None => (TokenStream2::new(), None),
        };
        let recurse_next_ref = recurse_next.as_ref();
        let inherit_macros = build_inherit_macros(
            &bare_name,
            &dyn_ident,
            /* has_concrete_type */ false,
            /* skip_own_impl */ false,
            &overridable_names,
            &[],
            &recurse_macro_path,
            recurse_next_ref,
            None,
        );
        let bridge_macro =
            build_class_interface_bridge_macro(&bare_name, &dyn_ident, &overridable_names, false);
        let bridge_ident = class_interface_bridge_ident(&bare_name);
        // A *different* wrapper module name from `macro_reexport_mod_ident`'s own
        // `__elwindui_macros_of_<bare_name>` — see `class_interface_wrapper_mod_ident`'s own doc
        // comment on the same-bare-name collision this avoids. Only the bridge macro needs a
        // module-addressable home here: nothing outside this class's own generated code ever
        // references its entry/skip/classify macros by path (see `inherit_macros`, above, whose
        // own recursion/self-references stay `$crate`-qualified regardless).
        let mod_ident = class_interface_wrapper_mod_ident(&bare_name);
        let macro_reexports = quote! {
            #[doc(hidden)]
            #[allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]
            pub mod #mod_ident {
                pub use crate::#bridge_ident;
            }
        };
        quote! {
            #inherit_macros
            #bridge_macro
            #macro_reexports
        }
    });

    // The shape layer *is* generated for `trait_only` (unlike the inherit trio above): the
    // backend-agnostic DSL surface of every native leaf is declared here in `elwindui-core`, and
    // each backend's own `struct_only` implementor deliberately declares none — so there is exactly
    // one `__elwindui_shape_{Name}!` per builtin workspace-wide, with no cross-crate bare-name
    // collision for `#[macro_export]` to trip over.
    let shape_macro = prop_decls.is_declared().then(|| {
        let parent = args
            .inherits
            .as_ref()
            .and_then(|ty| last_segment_name(ty).map(|name| (name, ty)));
        build_props_macro(
            &bare_name,
            &prop_decls,
            parent.as_ref().map(|(name, ty)| (name.as_str(), *ty)),
        )
    });

    quote! {
        #(#attrs)*
        #vis trait #ext_name #generics #colon_bound #where_clause {
            #(#dyn_methods)*
            #into_node_default
        }
        #inherit_macros_and_bridge
        #shape_macro
    }
}

/// Builds a `#[cfg(rust_analyzer)]`-only `impl ClassName { .. }` exposing this class's own
/// methods/constructor as plain inherent methods — entirely self-contained from this single `impl`
/// invocation's own `item.items` (`own_methods`/`override_methods`/`ctor_methods`, already
/// classified by `expand_impl`'s own loop, which never consults `args`/`class_arg_store`), so it
/// always succeeds regardless of whether/when the paired `struct ClassName`'s attribute has been
/// expanded.
///
/// Only ever called from `expand_impl`'s `class_arg_store` lookup-failure branch — see this
/// module's own doc comment on that store and docs/specs/macro_class_spec.md §15: rust-analyzer's
/// demand-driven macro expansion doesn't guarantee rustc's "same-crate attribute macros expand in
/// source order," so this lookup can fail spuriously there even when nothing is really wrong. The
/// ordinary success path needs no shadow of its own — `trait_decl`/`trait_impl`/`ctor_block` (built
/// straight from `args`, exactly as before this mechanism existed) already give rust-analyzer full,
/// accurate information, including the `pub trait {ClassName}Ext` declaration itself; hiding that
/// declaration behind this cfg too (an earlier version of this function did exactly that, gating the
/// *entire* real generation unconditionally) was tried and reverted — plenty of hand-written code in
/// this codebase names `{ClassName}Ext` directly as a *type* (`Rc<dyn UIElementExt>`, `T:
/// UIElementExt`, ...), not just through method-call completion, and none of that resolves once the
/// trait itself doesn't exist under `cfg(rust_analyzer)`.
///
/// Real (`cfg(not(rust_analyzer))`) builds never see this — it plays no role in actual compiled
/// behavior, and is never exercised by `cargo build`/`cargo test` (verify with `RUSTFLAGS="--cfg
/// rust_analyzer" cargo check`, since that cfg is otherwise only ever set by rust-analyzer itself).
///
/// `own_methods`/`override_methods` are re-emitted with `pub` visibility (overriding whatever
/// `expand_impl`'s own classification loop already set on the clones it hands here) — in the real
/// build they're reachable through the `pub` `{ClassName}Ext` trait, and the shadow's plain
/// inherent `impl` needs its own explicit `pub` to match that same effective visibility. An
/// `#[overrides]` method is included alongside `own_methods` here — even though the real build
/// routes it through the ancestor's own `impl` via the classify muncher — since a class's own
/// override is exactly the method body that wins at this class's own concrete type, matching the
/// real dispatch's own "closest override" semantics. `ctor_methods` keep whatever visibility
/// `expand_impl` already resolved (including deliberately-private `#[inherent]` helpers) — that one
/// mirrors the real `ctor_block` exactly, so no visibility override happens here.
///
/// `new`-synthesis here is a simplified, best-effort version of the real rule in `expand_impl`
/// (always synthesize from a `construct` method when one exists and no hand-written `new` does) —
/// `args.abstract_class` (which suppresses this on a real `abstract_class`) isn't available on this
/// lookup-failure path, and a rare `abstract_class`'s completion surface being slightly over-
/// approximated is an IDE-only inaccuracy, never observable from a real build.
fn build_rust_analyzer_shadow(
    impl_name: &Ident,
    impl_generics: &syn::ImplGenerics,
    ty_generics: &syn::TypeGenerics,
    where_clause: Option<&syn::WhereClause>,
    own_methods: &[ImplItemFn],
    override_methods: &[ImplItemFn],
    ctor_methods: &[(ImplItemFn, bool)],
) -> TokenStream2 {
    let has_hand_written_new = ctor_methods.iter().any(|(f, _)| f.sig.ident == "new");
    let construct_fn = if has_hand_written_new {
        None
    } else {
        ctor_methods
            .iter()
            .find(|(f, _)| f.sig.ident == "construct")
            .map(|(f, _)| f)
    };
    let auto_new = construct_fn.map(|f| {
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
    let own_bodies = own_methods.iter().chain(override_methods.iter()).map(|f| {
        let mut f = f.clone();
        f.vis = Visibility::Public(Default::default());
        quote! { #f }
    });
    let ctor_bodies = ctor_methods.iter().map(|(f, force_pub)| {
        let mut f = f.clone();
        if *force_pub {
            f.vis = Visibility::Public(Default::default());
        }
        quote! { #f }
    });
    quote! {
        #[cfg(rust_analyzer)]
        #[allow(unexpected_cfgs)]
        impl #impl_generics #impl_name #ty_generics #where_clause {
            #(#ctor_bodies)*
            #(#own_bodies)*
            #auto_new
        }
    }
}

/// Validates `fn on_constructed(&self)`'s signature (guide §4.2): exactly `&self`, no other params,
/// no generics, not `async`, returns `()` (explicit `-> ()` or no `-> ..` at all). Returns the guide's
/// own suggested diagnostic text on any violation.
fn validate_on_constructed_sig(sig: &syn::Signature) -> syn::Result<()> {
    let msg = "`on_constructed` must have the signature `fn on_constructed(&self)`";
    if sig.asyncness.is_some() {
        return Err(syn::Error::new_spanned(sig, msg));
    }
    if !sig.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(&sig.generics, msg));
    }
    let mut inputs = sig.inputs.iter();
    match inputs.next() {
        Some(FnArg::Receiver(r)) if r.reference.is_some() && r.mutability.is_none() => {}
        _ => return Err(syn::Error::new_spanned(&sig.inputs, msg)),
    }
    if inputs.next().is_some() {
        return Err(syn::Error::new_spanned(&sig.inputs, msg));
    }
    match &sig.output {
        syn::ReturnType::Default => {}
        syn::ReturnType::Type(_, ty) => {
            let is_unit = matches!(&**ty, Type::Tuple(t) if t.elems.is_empty());
            if !is_unit {
                return Err(syn::Error::new_spanned(ty, msg));
            }
        }
    }
    Ok(())
}

/// `true` if `ty` is written literally as the bare `Self` path type — used to validate `construct`'s
/// own declared return type (guide §10.1/test 12.18).
fn is_self_type(ty: &Type) -> bool {
    matches!(ty, Type::Path(tp) if tp.qself.is_none() && tp.path.is_ident("Self"))
}

/// Rewrites `construct`'s own function body in place (guide §3.2/§3.3/§6): every bare reference to
/// the reserved identifier `__self_weak` becomes `__class_self_weak` (the real hidden parameter this
/// function is about to gain), and every call whose callee is exactly `<parent_bare_name>::construct`
/// — this class's own declared `inherits = ..` target, never an unrelated `Type::construct()` call —
/// becomes `<parent_bare_name>::__class_construct(__class_self_weak.clone(), ..)`. Pure `syn`
/// `VisitMut` AST rewriting, never string replacement (guide §6.3). Also rejects a user-declared
/// `let __self_weak = ..;` binding as a reserved-identifier collision (guide §5.3/§5.4) — a
/// same-named *parameter* is checked separately by the caller, before this visitor ever runs, since
/// `syn::Signature`'s own params aren't part of the `Block` this visitor walks.
struct SelfWeakRewriter<'a> {
    hidden_ident: &'a Ident,
    parent_bare_name: Option<&'a str>,
    error: Option<syn::Error>,
}

impl<'a> syn::visit_mut::VisitMut for SelfWeakRewriter<'a> {
    fn visit_local_mut(&mut self, local: &mut syn::Local) {
        if let syn::Pat::Ident(pi) = &local.pat {
            if pi.ident == "__self_weak" && self.error.is_none() {
                self.error = Some(syn::Error::new_spanned(
                    pi,
                    "`__self_weak` is reserved by the #[class] macro",
                ));
            }
        }
        syn::visit_mut::visit_local_mut(self, local);
    }

    fn visit_expr_mut(&mut self, expr: &mut syn::Expr) {
        // Post-order: recurse first, so a nested `__self_weak` reference used as an argument to an
        // ancestor `construct(..)` call gets rewritten before the enclosing call itself is handled.
        syn::visit_mut::visit_expr_mut(self, expr);
        match expr {
            syn::Expr::Path(p) if p.path.is_ident("__self_weak") => {
                if let Some(seg) = p.path.segments.first_mut() {
                    seg.ident = self.hidden_ident.clone();
                }
            }
            syn::Expr::Call(call) => {
                let Some(parent) = self.parent_bare_name else {
                    return;
                };
                let syn::Expr::Path(fp) = &mut *call.func else {
                    return;
                };
                let segs = &fp.path.segments;
                if segs.len() < 2 {
                    return;
                }
                let last_is_construct = segs.last().is_some_and(|s| s.ident == "construct");
                let second_last_matches = segs[segs.len() - 2].ident == parent;
                if !(last_is_construct && second_last_matches) {
                    return;
                }
                fp.path.segments.last_mut().unwrap().ident = format_ident!("__class_construct");
                let hidden = self.hidden_ident.clone();
                let new_arg: syn::Expr = syn::parse_quote! { #hidden.clone() };
                call.args.insert(0, new_arg);
            }
            _ => {}
        }
    }
}

/// Transforms a user-written `construct` function in place into the real, hidden
/// `__class_construct` (guide §3.1/§3.2): inserts the leading hidden parameter
/// `__class_self_weak: std::rc::Weak<dyn {own_ext_ty}>` (this class's own Ext trait — see
/// `expand_impl`'s own `own_ext_ty` computation, §1a/§1b of the plan: no root-type propagation is
/// needed since every ancestor `construct` call coerces via the ordinary `{Child}Ext: {Parent}Ext`
/// supertrait bound already in place at the call site), rewrites `__self_weak`/ancestor-`construct`
/// references in the body (`SelfWeakRewriter`), and renames the function to `__class_construct`.
fn transform_construct_fn(
    f: &mut ImplItemFn,
    own_ext_ty: &TokenStream2,
    parent_bare_name: Option<&str>,
) -> syn::Result<()> {
    for arg in &f.sig.inputs {
        if let FnArg::Typed(pt) = arg {
            if let Pat::Ident(pi) = &*pt.pat {
                if pi.ident == "__self_weak" {
                    return Err(syn::Error::new_spanned(
                        pi,
                        "`__self_weak` is reserved by the #[class] macro",
                    ));
                }
            }
        }
    }
    let hidden_ident = format_ident!("__class_self_weak");
    let mut rewriter = SelfWeakRewriter {
        hidden_ident: &hidden_ident,
        parent_bare_name,
        error: None,
    };
    syn::visit_mut::VisitMut::visit_block_mut(&mut rewriter, &mut f.block);
    if let Some(e) = rewriter.error {
        return Err(e);
    }
    f.sig.ident = format_ident!("__class_construct");
    f.attrs.push(syn::parse_quote!(#[doc(hidden)]));
    let hidden_param: FnArg = syn::parse_quote! { #hidden_ident: std::rc::Weak<dyn #own_ext_ty> };
    f.sig.inputs.insert(0, hidden_param);
    Ok(())
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
            None => {
                return syn::Error::new_spanned(&item.self_ty, "#[class]: empty type path")
                    .to_compile_error();
            }
        },
        other => {
            return syn::Error::new_spanned(other, "#[class]: unsupported `impl` self type")
                .to_compile_error();
        }
    };
    let bare_name = class_name.to_string();
    // The compiled struct is always exactly `class_name` — no suffix transform.
    let impl_name = class_name.clone();
    // `<H>`/`<H: 'static>`/where-clause, threaded through every generated block below so a generic
    // class (e.g. `NativeControl<H>`) works the same as a non-generic one.
    let (impl_generics, ty_generics, where_clause) = item.generics.split_for_impl();

    // `bool` = force this entry to `pub` regardless of what the user wrote — true constructors
    // (no `self`) always did in the hand-written original, but an `#[inherent]` `&self` helper
    // keeps whatever visibility the user actually gave it (some, like `rebuild`/
    // `sync_dynamic_entries` in the appkit `TabView` facade, are deliberately private).
    let mut ctor_methods: Vec<(ImplItemFn, bool)> = Vec::new();
    let mut instance_methods: Vec<ImplItemFn> = Vec::new();
    // `#[overridable]`-tagged own method names — the descendant-facing vocabulary this class
    // exposes for the `__elwindui_inherit_*!` classify muncher (`build_inherit_macros`).
    let mut overridable_names: Vec<Ident> = Vec::new();
    // `#[overrides]`-tagged methods (any hop distance) — collected as a flat, unordered list of
    // real `fn` items and handed to this class's own `inherits = ..` target's `__elwindui_inherit_*!`
    // invocation; routing to the right ancestor's `impl` is entirely the classify muncher's job
    // (keyed by each method's own name), not something resolved here.
    let mut override_methods: Vec<ImplItemFn> = Vec::new();
    // The optional `fn on_constructed(&self)` lifecycle hook (§1d/§4 of the plan) — stashed here
    // rather than flowing through `instance_methods` since it's a purely internal lifecycle hook,
    // never part of `{ClassName}Ext`. `None` if this class doesn't define one.
    let mut on_constructed_fn: Option<ImplItemFn> = None;
    // This classification pass reads only `item.items`, never `args`/`class_arg_store` — moved
    // ahead of the store lookup below specifically so `build_rust_analyzer_shadow` (called right
    // after it) never depends on that lookup's success. See that function's own doc comment.
    for impl_item in item.items {
        match impl_item {
            ImplItem::Fn(f) if f.sig.ident == "on_constructed" => {
                if let Err(e) = validate_on_constructed_sig(&f.sig) {
                    return e.to_compile_error();
                }
                on_constructed_fn = Some(f);
            }
            ImplItem::Fn(f) if f.sig.ident == "new" => {
                let msg = "#[class]: do not define `new` by hand — `#[class]` generates `new()` \
                           automatically from `construct` and runs `on_constructed` automatically once \
                           `Rc::new_cyclic` completes. Move any post-construction work (parent-pointer \
                           wiring, event wiring, ...) into `on_constructed` instead.";
                return syn::Error::new_spanned(&f.sig, msg).to_compile_error();
            }
            ImplItem::Fn(f) if f.sig.ident == "__new_unmounted" => {
                let msg = "#[class]: `__new_unmounted` is a reserved, auto-generated name (CI-7 of \
                           #80, docs/design/runtime/component_lifecycle_design.md §4f) — do not \
                           define it by hand.";
                return syn::Error::new_spanned(&f.sig, msg).to_compile_error();
            }
            ImplItem::Fn(mut f) => {
                // `#[inherent]` opts a `&self` method *out* of trait-impl routing entirely — for
                // helpers that aren't part of any trait (ancestor's or `ClassNameExt`'s own), e.g. the
                // backend facade layer's `into_any_view`/`set_on_text_change`. It lands as a plain
                // `impl ClassName { .. }` method, alongside constructors.
                let is_inherent = f.attrs.iter().any(|a| a.path().is_ident("inherent"));
                let is_overridable = f.attrs.iter().any(|a| a.path().is_ident("overridable"));
                let is_overrides = f.attrs.iter().any(|a| a.path().is_ident("overrides"));
                if is_overridable {
                    overridable_names.push(f.sig.ident.clone());
                }
                f.attrs.retain(|a| {
                    !(a.path().is_ident("overridable")
                        || a.path().is_ident("overrides")
                        || a.path().is_ident("inherent"))
                });
                if is_overrides {
                    let mut f = f.clone();
                    f.vis = Visibility::Inherited;
                    override_methods.push(f);
                }
                if is_inherent {
                    ctor_methods.push((f, false));
                    continue;
                }
                let has_self = matches!(f.sig.inputs.first(), Some(FnArg::Receiver(_)));
                if has_self {
                    if is_overrides {
                        // Already collected above (with its real body) — an `#[overrides]` method
                        // is exclusively routed through the ancestor's own `impl` via the classify
                        // muncher, never through `ClassNameExt`'s own trait impl too.
                        continue;
                    }
                    // Every remaining instance method lands in `ClassNameExt`'s own trait impl —
                    // trait impl items always inherit the trait's own visibility and reject an
                    // explicit qualifier (E0449).
                    f.vis = Visibility::Inherited;
                    instance_methods.push(f);
                } else {
                    ctor_methods.push((f, true));
                }
            }
            other => {
                return syn::Error::new_spanned(
                    other,
                    "#[class]: only `fn` items are supported inside `impl ClassName { .. }`",
                )
                .to_compile_error();
            }
        }
    }
    let own_methods = instance_methods;

    // `construct` is mandatory for every `#[class]`-tagged class (guide §10.1, confirmed in scope
    // with no carve-out for hand-written-`new()`-only classes) — find it and verify exactly one
    // qualifying candidate exists. Checked here, ahead of `args` resolution/`core_path()` (both
    // needed only once a real `construct` is confirmed to exist), so a class with a missing/
    // duplicate/malformed `construct` fails fast without requiring anything else about this
    // class's environment (e.g. a resolvable path to `elwindui-core`) to be in place yet.
    let construct_positions: Vec<usize> = ctor_methods
        .iter()
        .enumerate()
        .filter(|(_, (f, _))| f.sig.ident == "construct")
        .map(|(i, _)| i)
        .collect();
    match construct_positions.len() {
        0 => {
            let msg = format!("class `{class_name}` does not define `construct(...) -> Self`");
            return syn::Error::new_spanned(&item.self_ty, msg).to_compile_error();
        }
        1 => {}
        _ => {
            let msg = format!(
                "#[class]: class `{class_name}` defines multiple `construct` functions — exactly \
                 one `construct(...) -> Self` is required"
            );
            return syn::Error::new_spanned(&item.self_ty, msg).to_compile_error();
        }
    }
    let construct_index = construct_positions[0];
    let construct_returns_self = matches!(
        &ctor_methods[construct_index].0.sig.output,
        syn::ReturnType::Type(_, ty) if is_self_type(ty)
    );
    if !construct_returns_self {
        let msg = "#[class]: `construct` must return `Self`";
        return syn::Error::new_spanned(&ctor_methods[construct_index].0.sig, msg)
            .to_compile_error();
    }

    // A bare `#[class]` on the impl means "use whatever `#[class(..)]` args the paired `struct
    // ClassName` declared" (see `store_class_args`/`load_class_args`) — explicit args on the impl
    // (the old, still-supported form) always win over anything stored.
    // `owns_inherit_macros` is `true` unless this bare name collided with an earlier same-crate
    // declaration (see `register_same_crate_class`'s own doc comment) — explicit args on the impl
    // (bypassing the struct-args store entirely) have no such information available, so they
    // default to `true` (assume no collision), matching this rare, pre-existing form's existing
    // behavior before this check existed.
    let (args, owns_inherit_macros, struct_vis) = if attr_is_empty {
        match load_class_args(&bare_name) {
            Some(triple) => triple,
            None => {
                // A genuine ordering mistake under rustc (struct declared after its impl, or not at
                // all) fails here for real — but this lookup can *also* fail spuriously under
                // rust-analyzer, which never guaranteed this impl's paired struct was expanded
                // first (see `build_rust_analyzer_shadow`'s own doc comment and
                // docs/specs/macro_class_spec.md §15). So: always emit the self-contained shadow
                // (built from `item.items` alone, just classified above, no store dependency) for
                // rust-analyzer's benefit, and keep the `compile_error!` real but gate it to actual
                // `cargo build`/`cargo check` only, so a spurious lookup failure under
                // rust-analyzer never blanks out this class's own completion.
                let shadow = build_rust_analyzer_shadow(
                    &impl_name,
                    &impl_generics,
                    &ty_generics,
                    where_clause,
                    &own_methods,
                    &override_methods,
                    &ctor_methods,
                );
                let msg = format!(
                    "#[class]: no matching `struct {class_name}` with #[elwindui_macros::class(..)] found earlier in \
                     this file — declare the struct (with any inherits/struct_only/... args) before this \
                     impl block, or pass args explicitly here"
                );
                let err = syn::Error::new_spanned(&item.self_ty, msg).to_compile_error();
                return quote! {
                    #shadow
                    #[cfg(not(rust_analyzer))]
                    #[allow(unexpected_cfgs)]
                    #err
                };
            }
        }
    } else {
        // The rare explicit-args-on-impl form bypasses the struct-args store entirely, so no stored
        // struct visibility is available either — default to `pub`, matching this form's existing
        // permissive behavior (Issue #128 remediation's `own_ext_alias` is the only consumer of this
        // value; a too-narrow guess here would only make the `pub(crate)`-vs-`pub` privacy check on
        // that alias unnecessarily strict, never silently wrong).
        (attr_args, true, Visibility::Public(Default::default()))
    };
    let args = &args;

    // Root-class mode (`inherits` omitted *and* not `struct_only`, i.e. the one class with no
    // ancestor of its own at all — `UIElement`): there's no ancestor to route anything to, so
    // every method the user wrote is an "own" method. A `struct_only` class with no `inherits`
    // (e.g. `Window`/`MenuBar`, composing nothing) is *not* root mode — it just has no further
    // chain beyond itself, which only matters for `inherit_macros`'s own `recurse` computation
    // below (its own struct_only branch, checked first, already handles its trait_decl/trait_impl
    // either way).
    let is_root_mode = args.inherits.is_none() && args.struct_only.is_none();

    let core = core_path();

    // `#[sealed]`(§3 of the plan)/inheritance handling: unconditionally, whenever this class itself
    // `inherits = Parent`, check `Parent` isn't `#[sealed]`, then invoke `Parent`'s own
    // `__elwindui_inherit_*!` (or `..._skip!`, if this class is `struct_only` for the identical
    // trait `Parent` already implements) trio to pull in everything from `Parent` on down —
    // hop-0 and hop-N are handled by this exact same, single code path; see this module's own doc
    // comment.
    let mut prelude = Vec::new();
    if let Some(inh) = &args.inherits {
        if let Some(parent_bare) = last_segment_name(inh) {
            let sealed_path =
                inherit_macro_path(&parent_bare, inh, sealed_check_ident(&parent_bare));
            prelude.push(quote! { #sealed_path!(); });

            // `Parent`'s own entry macro is always invoked here, regardless of *this* class's own
            // `no_ancestor_forward` -- that flag only concerns *this* class's own hand-written
            // trait (via `struct_only`'s `dyn_accessor` gate below, and `build_inherit_macros`'s
            // `skip_own_impl` for what *this* class's own generated trio does for descendants);
            // it says nothing about whether *this* class itself still needs to implement
            // `Parent`'s (perfectly ordinary, `__dyn_x`-compatible) trait. Skipping this call for
            // a `no_ancestor_forward` class (e.g. `NativeTabView`) would leave it without an
            // `impl NativeControlExt for NativeTabView` at all.
            //
            // Each override is passed as a `name => { <the fn item> }` keyed group — the
            // classify muncher (`build_inherit_macros`) matches on the method's own name to
            // decide which ancestor's `impl` it belongs in; nothing here needs to know that.
            let overrides = override_methods.iter().map(|f| {
                let name = &f.sig.ident;
                quote! { #name => { #f }, }
            });
            // PR #164 final remediation round (finding C2, §4.6/§4.8): a `struct_only` class whose
            // own explicit target is *exactly* its direct `inherits = ..` parent's own generated
            // interface must not *also* pick up a second, independently-generated `impl <parent's
            // own trait> for <this class>` through this ordinary forwarding path — that would
            // collide (`E0119`) with the explicit `impl #existing for #impl_name { .. }` this
            // class's own `struct_only` branch already builds — so it routes through the parent's
            // `_skip!` entry point too, exactly like the narrower, pre-existing
            // `struct_only_collides_with` case (struct_only-target-equals-struct_only-target).
            let use_skip = struct_only_collides_with(&args.struct_only, &parent_bare)
                || struct_only_matches_direct_parent(&args.struct_only, &args.inherits);
            if use_skip {
                let invoke_path =
                    inherit_macro_path(&parent_bare, inh, inherit_macro_skip_ident(&parent_bare));
                prelude.push(quote! {
                    #invoke_path!(#impl_name #ty_generics; #(#overrides)*);
                });
            } else {
                let parent_bridge_path = ancestor_bridge_path(&parent_bare, inh);
                // PR #164 final remediation round (finding A5): `Parent`'s entry macro's own
                // `$OwnTrait` argument is resolved at *macro-expansion* time now, not baked here at
                // this class's own proc-macro time (`ancestor_own_trait`'s same-crate-registry
                // heuristic, which this call site used to use, cannot reliably distinguish
                // "reattach `inh`'s own trailing generic arguments" from "don't" for a genuinely
                // cross-crate `Parent` — see `own_ext_policy_ident`'s own doc comment for the full
                // protocol and reasoning). This fresh, uniquely-named (per this class, which has at
                // most one direct `inherits = ..`) continuation macro captures everything else this
                // invocation needs; `Parent`'s own policy macro supplies only the resolved trait
                // tokens.
                let policy_path = own_ext_policy_path(&parent_bare, inh);
                let continuation_ident =
                    format_ident!("__elwindui_continue_inherits_of_{bare_name}");
                let concrete_args = last_segment_generic_args(inh);
                // The policy macro's own output is *only* the resolved trailing generic-argument
                // tokens (`[]` or the echoed `[$($args)*]`) — never a full path (see
                // `own_ext_policy_ident`'s own doc comment) — so the always-correct, proc-macro-time-
                // known prefix (`<Parent's wrapper>::__ElwindUIOwnExt`) has to be supplied here, by
                // this continuation macro itself, not by the policy macro.
                //
                // Every one of these four is embedded into a `macro_rules!` *body* below (this
                // continuation macro), not item position — despite being defined and first invoked
                // in this same crate, a bare `crate` token inside a macro body still doesn't
                // reliably resolve once it's forwarded on into `Parent`'s own (different-crate, e.g.
                // `elwindui-core`) recursive classify chain (see `rewrite_crate_segment`'s own doc
                // comment — the exact same "bake once, re-expand elsewhere" hazard, confirmed
                // empirically here too: `cannot find type UIElement in this scope` against a real
                // backend-appkit build before this fix). `inherit_macro_self_ref_path` (not
                // `inherit_macro_path`, item-position-only per its own doc comment) for the same
                // reason.
                let invoke_path_self_ref = inherit_macro_self_ref_path(
                    &parent_bare,
                    inh,
                    inherit_macro_ident(&parent_bare),
                );
                let own_ext_prefix = rewrite_crate_segment(path_module_prefix(inh).expect(
                    "#[class]: internal: inherits argument should already be validated fully-qualified",
                ));
                let own_ext_alias_ident_val = own_ext_alias_ident();
                let inh_rewritten = rewrite_crate_segment(quote! { #inh });
                let parent_bridge_path_rewritten =
                    rewrite_crate_segment(parent_bridge_path.clone());
                prelude.push(quote! {
                    #[doc(hidden)]
                    #[macro_export]
                    #[allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]
                    macro_rules! #continuation_ident {
                        ([$($OwnTrait:tt)*]) => {
                            #invoke_path_self_ref!(#impl_name #ty_generics, #own_ext_prefix :: #own_ext_alias_ident_val $($OwnTrait)*, #inh_rewritten, #inh_rewritten, #parent_bridge_path_rewritten; #(#overrides)*);
                        };
                    }
                });
                prelude.push(quote! {
                    #policy_path!(crate::#continuation_ident; [ #concrete_args ]);
                });
            }
        }
    } else if !override_methods.is_empty() {
        let names: Vec<String> = override_methods
            .iter()
            .map(|f| f.sig.ident.to_string())
            .collect();
        let msg = format!(
            "#[class]: #[overrides] methods {names:?} require `inherits = ..` (root class mode has no ancestor)"
        );
        return syn::Error::new_spanned(&item.self_ty, msg).to_compile_error();
    }

    let ext_ident = to_ext_ident(&bare_name, class_name.span());

    // This class's own `{ClassName}Ext` trait (or, for `struct_only`, the existing trait it
    // implements directly) — the type `__class_self_weak` is declared as. No root-type propagation
    // is needed: an ancestor `construct` call coerces via the ordinary `{Child}Ext: {Parent}Ext`
    // supertrait bound already in place at the call site (`Weak<dyn Sub> -> Weak<dyn Super>` coerces
    // at argument position whenever `Sub: Super`, verified against this workspace's own toolchain).
    let own_ext_ty: TokenStream2 = match &args.struct_only {
        Some(p) => quote! { #p },
        None => quote! { #ext_ident #ty_generics },
    };
    let parent_bare_name_owned = args.inherits.as_ref().and_then(last_segment_name);
    if let Err(e) = transform_construct_fn(
        &mut ctor_methods[construct_index].0,
        &own_ext_ty,
        parent_bare_name_owned.as_deref(),
    ) {
        return e.to_compile_error();
    }

    let (trait_decl, trait_impl) = if let Some(existing) = &args.struct_only {
        // The concrete, hand-written implementor of someone else's `{Name}Ext` trait — this is the
        // one shape besides root mode that provides *real* bodies directly rather than relying on
        // `{Name}Ext`'s own default methods, so it must also supply that trait's required `__dyn_x`
        // accessor (plus, per Issue #128, one reflexive per-`#[overridable]`-slot accessor) itself
        // (reflexively — `self` already *is* the concrete implementor). Skipped for
        // `no_ancestor_forward` (e.g. `NativeTabView`'s `struct_only = TabView`): that target is a
        // hand-written trait predating this macro's `__dyn_x` convention, with no such method
        // declared on it at all — adding one here would be E0407 (method not a member of trait); a
        // `no_ancestor_forward` target also never has a `__elwindui_class_interface_*!` bridge to
        // route through (see this module's own doc comment on `no_ancestor_forward`), so this
        // branch keeps building the `impl` directly, exactly as before #128.
        let bodies = own_methods.iter().map(|f| quote! { #f });
        if args.no_ancestor_forward {
            (
                TokenStream2::new(),
                quote! {
                    impl #impl_generics #existing for #impl_name #ty_generics #where_clause {
                        #(#bodies)*
                    }
                },
            )
        } else {
            // Issue #128: `existing` (the `struct_only = ..Ext` argument) names a `#[class]`-generated
            // interface — its own class name, and so its `__elwindui_class_interface_*!` bridge, is
            // derivable from that `Ext`-suffixed path alone (`class_name_from_ext_trait_path`), with
            // no manually-maintained table. The bridge (`@impl_struct_only_members`, review finding
            // A3) only supplies the shared `__dyn_x` accessor and one reflexive per-`#[overridable]`-
            // slot accessor per member — this branch keeps building and owning the outer `impl`
            // header itself (`impl_generics`/`ty_generics`/`where_clause` all preserved, exactly as
            // the `no_ancestor_forward` branch above does), so a generic `struct_only` type does not
            // lose its own generic parameters/bounds to the bridge's opaque macro tokens.
            let class_name = match class_name_from_ext_trait_path(existing) {
                Ok(name) => name,
                Err(e) => return e.to_compile_error(),
            };
            let bridge_path = class_interface_bridge_path(&class_name.to_string(), existing);
            // PR #164 final remediation round (finding C2): computed purely from this class's own
            // `struct_only =`/`inherits =` attribute text (no cross-crate ambiguity at all — see
            // `struct_only_matches_direct_parent`'s own doc comment) and passed through so the
            // *target* interface's own bridge (which alone knows whether it's root-mode) can decide
            // whether to emit the required `as_ui_element` forwarding member or a diagnostic.
            let has_matching_base =
                struct_only_matches_direct_parent(&args.struct_only, &args.inherits);
            // `[#inh]`/`[]`: when this class's own `inherits = ..` genuinely names the matching
            // root class (`has_matching_base`), that already-fully-qualified path (`args.inherits`)
            // is threaded straight through as the bridge's `$RootConcrete` macro parameter — see
            // `build_class_interface_bridge_macro`'s own doc comment on why the target bridge can
            // never bake this path in on its own.
            let root_concrete_arg = if has_matching_base {
                let inh = args.inherits.as_ref().expect(
                    "#[class]: internal: has_matching_base implies inherits is Some \
                     (struct_only_matches_direct_parent requires both to be Some)",
                );
                quote! { [#inh] }
            } else {
                quote! { [] }
            };
            (
                TokenStream2::new(),
                quote! {
                    impl #impl_generics #existing for #impl_name #ty_generics #where_clause {
                        #bridge_path!(@impl_struct_only_members #existing, has_matching_base = #root_concrete_arg);
                        #(#bodies)*
                    }
                },
            )
        }
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
        // `as_ui_element` returns a *concrete* type (used pervasively for direct field access on
        // the root's own shared state, e.g. `self.as_ui_element().min_width`) — it cannot double
        // as a `__dyn_x`-style dispatch accessor for `#[overridable]` methods the way an ordinary
        // class's own accessor does, because dispatching through it would always reach the root's
        // own concrete value, never an intermediate override. So a root class additionally gets
        // its own `__dyn_x`-style accessor (`dyn_ident`, `&dyn ClassNameExt`-returning) used
        // *only* to give its `#[overridable]` methods the same "correctly reaches the closest
        // override at any hop depth" default-method dispatch every ordinary class's own methods
        // already get (`build_dyn_default_methods`) — every other own method keeps today's
        // existing shape (its literal body embedded directly as the trait's own default, shared
        // verbatim by every descendant, since nothing ever overrides it).
        let dyn_ident = dyn_accessor_ident(&bare_name);
        let (overridable_methods, plain_methods): (Vec<ImplItemFn>, Vec<ImplItemFn>) = own_methods
            .into_iter()
            .partition(|f| overridable_names.iter().any(|n| *n == f.sig.ident));
        let ext_ty = quote! { #ext_ident #ty_generics };
        let ext_ty_call = turbofish_ext_path(&quote! { #ext_ident }, &quote! { #ty_generics });
        let overridable_sigs: Vec<syn::Signature> =
            overridable_methods.iter().map(|f| f.sig.clone()).collect();
        let overridable_defaults = build_dyn_default_methods(
            &dyn_ident,
            &ext_ty,
            &ext_ty_call,
            &overridable_sigs,
            &overridable_names,
        );
        let plain_defaults = plain_methods.iter().map(|f| quote! { #f });
        let overridable_bodies = overridable_methods.iter().map(|f| quote! { #f });
        // Each overridable method's own *dedicated* accessor (`per_method_accessor_ident` — see
        // `build_dyn_default_methods`'s own doc comment on why one shared accessor isn't enough)
        // is, for the *declaring* class, reflexive — same reasoning as `dyn_ident` itself.
        let overridable_accessor_impls = overridable_names.iter().map(|name| {
            let accessor = per_method_accessor_ident(name);
            quote! { fn #accessor(&self) -> &dyn #ext_ty { self } }
        });
        let into_node_ident = into_node_ident(&bare_name);
        let into_node_default = build_into_node_default(&into_node_ident, &ext_ty);
        (
            quote! {
                pub trait #ext_ident #impl_generics #bound #where_clause {
                    // PR #164 final remediation round: `#ty_generics`, not bare `#impl_name` — a
                    // genuinely generic root class (never exercised in production before this
                    // remediation's own T8 fixture, `elwindui-bridge-fixture-iface`'s
                    // `BridgeFixtureGenericOrdinaryBase<T>`) needs its own required accessor to
                    // return `&Self<T>`, not an incomplete `&Self` missing its own type parameters
                    // — confirmed as a genuine, pre-existing gap (present before this remediation
                    // round entirely, verified against this branch's own base commit) via a real
                    // `E0107`/parse-confusion failure otherwise.
                    fn as_ui_element(&self) -> &#impl_name #ty_generics;
                    #(#overridable_defaults)*
                    #(#plain_defaults)*
                    #into_node_default
                }
            },
            // `ClassName` is a genuine `ClassNameExt` implementor itself — trivially for both
            // required accessors (`{ self }`) — but must restate its `#[overridable]` methods'
            // real bodies explicitly here rather than relying on the trait's own (dispatch-based)
            // defaults, exactly like any other declaring class: relying on the default would
            // recurse forever, since its own `dyn_ident` is reflexive.
            quote! {
                impl #impl_generics #ext_ident #ty_generics for #impl_name #ty_generics #where_clause {
                    #[allow(dead_code)]
                    fn as_ui_element(&self) -> &#impl_name #ty_generics { self }
                    fn #dyn_ident(&self) -> &dyn #ext_ty { self }
                    #(#overridable_accessor_impls)*
                    #(#overridable_bodies)*
                }
            },
        )
    } else {
        // Ordinary class: always declares its own `{ClassName}Ext` trait, even when `own_methods` is
        // empty (a class composed purely of `#[inherent]` helpers over an `inherits = ..` base, e.g.
        // a thin DSL-facing wrapper) — an empty trait/impl pair is harmless.
        let bound = args.inherits.as_ref().and_then(|t| {
            let parent_bare = last_segment_name(t)?;
            // No supertrait bound at all when the ancestor isn't forwardable (see
            // `ancestor_is_no_ancestor_forward`'s own doc comment) — there is nothing to bound
            // against, since that ancestor's own trait was never meant to be blindly implemented
            // (its hand-written trait has real required methods this class doesn't supply).
            if ancestor_is_no_ancestor_forward(&parent_bare) {
                return None;
            }
            // PR #164 final remediation round (finding A5): `ancestor_own_trait_bound` (the wrapper
            // `__ElwindUIOwnExtBound` trait) — a supertrait bound is resolved at *this* class's own
            // proc-macro time, and Rust does not permit a macro invocation there, so the
            // macro-expansion-time policy protocol used for dispatch routing (`own_ext_policy_ident`)
            // cannot apply to this position — see `own_ext_bound_trait_ident`'s own doc comment for
            // the full reasoning.
            let own_trait_ty = ancestor_own_trait_bound(&parent_bare, t);
            Some(quote! { : #own_trait_ty })
        });
        // Every own method becomes a *default* trait method, dispatching through the required
        // `__dyn_x` accessor (`build_dyn_default_methods`) — mirroring root mode's own
        // `as_ui_element`-based default-method pattern, generalized so any ordinary class's
        // descendants inherit its methods for free without needing to know their names/signatures.
        // `ClassName` itself, the declaring class, still overrides every one of them with its real
        // body below — relying on the default there would recurse forever (its own `__dyn_x` is
        // reflexive).
        let dyn_ident = dyn_accessor_ident(&bare_name);
        let own_sigs: Vec<syn::Signature> = own_methods.iter().map(|f| f.sig.clone()).collect();
        let ext_ty = quote! { #ext_ident #ty_generics };
        let ext_ty_call = turbofish_ext_path(&quote! { #ext_ident }, &quote! { #ty_generics });
        let dyn_methods = build_dyn_default_methods(
            &dyn_ident,
            &ext_ty,
            &ext_ty_call,
            &own_sigs,
            &overridable_names,
        );
        let bodies = own_methods.iter().map(|f| quote! { #f });
        // See root mode's own identical treatment (above) for why each of *this* class's own
        // `#[overridable]` methods (if it declares any of its own, beyond just re-overriding an
        // ancestor's) needs its own dedicated, reflexive accessor here too.
        let overridable_accessor_impls = overridable_names.iter().map(|name| {
            let accessor = per_method_accessor_ident(name);
            quote! { fn #accessor(&self) -> &dyn #ext_ty { self } }
        });
        let into_node_ident = into_node_ident(&bare_name);
        let into_node_default = build_into_node_default(&into_node_ident, &ext_ty);
        (
            quote! {
                pub trait #ext_ident #impl_generics #bound #where_clause {
                    #(#dyn_methods)*
                    #into_node_default
                }
            },
            quote! {
                impl #impl_generics #ext_ident #ty_generics for #impl_name #ty_generics #where_clause {
                    fn #dyn_ident(&self) -> &dyn #ext_ty { self }
                    #(#overridable_accessor_impls)*
                    #(#bodies)*
                }
            },
        )
    };

    // `construct` (mandatory, checked above) always drives an *auto-generated* `new` built on
    // `Rc::new_cyclic`, running the `on_constructed` hook chain right after — hand-writing `new` by
    // hand is rejected earlier, in the classification loop. `abstract_class` (e.g. `Layout`) never
    // gets an auto-generated `new` — an abstract class is never meant to be directly, publicly
    // instantiated as its own `Rc<Self>` — but it still defines `construct` (mandatory) purely so
    // its own concrete subclasses (`VerticalLayout`/`HorizontalLayout`) have something mechanical to
    // call for their own `base` field, instead of re-building `Layout { .. }` by hand in each one.
    let auto_new = (!args.abstract_class).then(|| {
        // `params` already includes the leading hidden `__class_self_weak` parameter
        // `transform_construct_fn` inserted — `new`'s own public signature/forwarded call skips it
        // (index 0) and restates only the user's original parameters.
        let params = &ctor_methods[construct_index].0.sig.inputs;
        // Materialized (not a bare iterator) because `quote!` below consumes it twice — once for
        // `new`'s signature, once for `__new_unmounted`'s identical one (CI-7 of #80).
        let public_params: Vec<_> = params.iter().skip(1).collect();
        let arg_names: Vec<TokenStream2> = params
            .iter()
            .skip(1)
            .filter_map(|arg| match arg {
                FnArg::Typed(pat_type) => {
                    let pat = &pat_type.pat;
                    Some(quote! { #pat })
                }
                FnArg::Receiver(_) => None,
            })
            .collect();
        quote! {
            pub fn new(#(#public_params),*) -> std::rc::Rc<Self> {
                let __obj = std::rc::Rc::new_cyclic(|__weak_self: &std::rc::Weak<Self>| {
                    Self::__class_construct(__weak_self.clone(), #(#arg_names),*)
                });
                __obj.__elwindui_run_on_constructed();
                __obj
            }

            // CI-7 of #80 (docs/design/runtime/component_lifecycle_design.md §4f): the same
            // construction step as `new()` above, but WITHOUT the trailing
            // `__elwindui_run_on_constructed()` call — the caller (only ever `EnvironmentScope`'s
            // own generated code today) is responsible for calling `.mount(environment)` on the
            // returned `Rc<Self>` itself, explicitly, afterward. Unconditionally emitted alongside
            // `new` (not gated by any class-level flag) because the choice between the two is a
            // per-call-site decision, not a per-type one — the same reusable class may be
            // constructed ordinarily in one place and inside an `EnvironmentScope` in another.
            #[doc(hidden)]
            pub fn __new_unmounted(#(#public_params),*) -> std::rc::Rc<Self> {
                std::rc::Rc::new_cyclic(|__weak_self: &std::rc::Weak<Self>| {
                    Self::__class_construct(__weak_self.clone(), #(#arg_names),*)
                })
            }
        }
    });

    // The hidden lifecycle hook-chain method (guide §4.5): base-class-first, exactly once per
    // object, never part of `{ClassName}Ext` (no dynamic dispatch needed — it's only ever called
    // once, concretely, right after `Rc::new_cyclic`, on the exact type being constructed).
    let run_on_constructed_ident = format_ident!("__elwindui_run_on_constructed");
    let base_hook_call = args
        .inherits
        .as_ref()
        .map(|_| quote! { self.base.#run_on_constructed_ident(); });
    let own_hook_call = on_constructed_fn
        .as_ref()
        .map(|_| quote! { self.on_constructed(); });
    let run_on_constructed_method = quote! {
        #[doc(hidden)]
        pub fn #run_on_constructed_ident(&self) {
            #base_hook_call
            #own_hook_call
        }
    };
    let on_constructed_method_emit = on_constructed_fn.as_ref().map(|f| quote! { #f });

    let mut ctor_block = TokenStream2::new();
    if !ctor_methods.is_empty() || auto_new.is_some() {
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
                #on_constructed_method_emit
                #run_on_constructed_method
                #auto_new
            }
        };
    }

    // A non-root class's own reflexive named accessor (`pub fn as_<name>(&self) -> &Self { self
    // }`) — the base case every `__elwindui_inherit_*!` chain built on top of `self.base.as_x()`
    // calls (`build_inherit_macros`'s `named_accessor`) ultimately bottoms out on. A root class
    // doesn't need this generated separately: `as_ui_element` already plays this exact role, via
    // its own trait impl above (root mode is the one shape whose "own accessor" is a trait method
    // rather than a plain inherent one).
    let reflexive_named_accessor = (!is_root_mode).then(|| {
        let accessor_ident = named_accessor_ident(&bare_name);
        quote! {
            impl #impl_generics #impl_name #ty_generics #where_clause {
                #[allow(dead_code)]
                pub fn #accessor_ident(&self) -> &Self { self }
            }
        }
    });

    // This class's own `__elwindui_inherit_*!` trio, for whoever later `inherits = ClassName` (or,
    // for a root class, whoever eventually reaches it via the chain) — see this module's own doc
    // comment. Not generated for `no_ancestor_forward` classes (see that flag's own doc comment),
    // for `#[sealed]` ones (nothing can ever legally name a sealed class as an `inherits = ..`
    // target, so there is no future consumer to generate this for), or when this bare name lost
    // the same-crate ownership race (`owns_inherit_macros`, see `register_same_crate_class`'s own
    // doc comment) — together these are what let a same-bare-named pair (a `#[sealed]` DSL-facing
    // wrapper and the unsealed native leaf it composes, e.g. `builtins::TextArea`/`appkit::TextArea`;
    // or two unrelated leaves nothing inherits from either, e.g. this crate's two `MenuItem`s)
    // coexist without a macro-name collision (`macro_rules!` items share one flat, crate-wide
    // namespace, unlike ordinary Rust items, which stay disambiguated by module).
    let inherit_macros = if args.sealed || !owns_inherit_macros {
        None
    } else {
        let dyn_ident = match &args.struct_only {
            Some(p) => dyn_accessor_ident(
                &p.segments
                    .last()
                    .map(|s| s.ident.to_string())
                    .unwrap_or_default(),
            ),
            None => dyn_accessor_ident(&bare_name),
        };
        // No further ancestor to recurse to (true root mode, e.g. `UIElement` -- or a
        // `struct_only` class with no `inherits` at all, e.g. `Window`/`MenuBar`, which simply has
        // no chain beyond itself) — `build_inherit_macros` generates its own local terminal check
        // in that case and ignores `recurse_macro_path` entirely, so `TokenStream2::new()` here is
        // just a placeholder. Otherwise, this class's *own* knowledge of its `inherits = ..`
        // target's fully-qualified trait/concrete path is what the next layer needs — see
        // `build_inherit_macros`'s own doc comment on why this can't be baked in by the next
        // layer itself. This class's *own* `no_ancestor_forward` doesn't change any of this: it
        // only controls whether *this* layer's classify macro builds `impl $OwnTrait for
        // $SubType` for itself (`skip_own_impl`, below) — whatever lies beyond it is still fully
        // reachable through it.
        let (recurse_macro_path, recurse_next) = match &args.inherits {
            Some(inh) => {
                let parent_bare = last_segment_name(inh).unwrap_or_default();
                let path = inherit_macro_self_ref_path(
                    &parent_bare,
                    inh,
                    inherit_macro_ident(&parent_bare),
                );
                (path, Some(build_ancestor_route(&parent_bare, inh)))
            }
            None => (TokenStream2::new(), None),
        };
        let recurse_next_ref = recurse_next.as_ref();
        // A root class's `as_ui_element` is a *second* required method beyond `dyn_ident` itself
        // (see `build_inherit_macros`'s own doc comment on `extra_required_names`) — every other
        // shape has none.
        let extra_required_names: Vec<Ident> = if is_root_mode {
            vec![format_ident!("as_ui_element")]
        } else {
            Vec::new()
        };
        // Issue #128: a forwardable `struct_only` implementor (not `no_ancestor_forward`) never
        // classifies against its own locally-known `overridable_names` (almost always empty — the
        // interface it implements owns that list, not this concrete struct) — instead its entry
        // macro redirects straight into that interface's own `__elwindui_class_interface_*!` bridge,
        // `@inherit_through_struct_only`, forwarding `$SubType`/`$OwnTrait`/`$OwnConcrete`/
        // `$overrides` through unexamined. `$BridgePath` (the entry macro's own 4th parameter, see
        // `build_inherit_macros`'s own doc comment) is used directly as the invocation prefix here,
        // rather than a path computed once at *this* class's own compile time — every caller of this
        // entry macro supplies it fresh, using its own knowledge of this ancestor, which is the only
        // way it resolves correctly no matter which crate ends up re-invoking this macro (see
        // `ancestor_bridge_path`'s own doc comment). The named accessor
        // (`as_<bare_name>(&self) -> &$OwnConcrete`) is still emitted directly here since it's
        // classify-independent (`build_inherit_macros`'s own `named_accessor`, bypassed along with
        // the rest of classify once `entry_override` is `Some`).
        let entry_override = args.struct_only.as_ref().and_then(|_| {
            if args.no_ancestor_forward {
                return None;
            }
            let named_accessor_ident_val = named_accessor_ident(&bare_name);
            // PR #164 final remediation round (finding C2): the `$RootConcrete` forwarded into the
            // target interface's own entry macro (via `@inherit_through_struct_only`) is *this*
            // class's own `inherits = ..` target when it has one (`next_concrete`, reusing the same
            // value ordinary recursion already computes for its own next hop) — never
            // `$OwnConcrete` blindly forwarded (that one is still correct, and still used, for
            // `named_accessor` above, which needs whatever the *caller* actually supplied — see
            // `extra_required_names`'s own parameter doc comment on `build_inherit_macros` for why
            // the two diverge). When this class itself has no further `inherits = ..` (e.g.
            // `Window`/`MenuBar`), there is no such target — this class's own concrete type is used
            // reflexively instead.
            let bridge_root_concrete = match &recurse_next {
                Some(route) => route.concrete.clone(),
                None => quote! { #impl_name #ty_generics },
            };
            Some(quote! {
                impl $SubType {
                    #[allow(dead_code)]
                    pub fn #named_accessor_ident_val(&self) -> &$OwnConcrete { self.base.#named_accessor_ident_val() }
                }
                $BridgePath!(@inherit_through_struct_only $SubType, $OwnTrait, $OwnConcrete, #bridge_root_concrete; $($overrides)*);
            })
        });
        Some(build_inherit_macros(
            &bare_name,
            &dyn_ident,
            true,
            args.no_ancestor_forward,
            &overridable_names,
            &extra_required_names,
            &recurse_macro_path,
            recurse_next_ref,
            entry_override,
        ))
    };

    // `#[sealed]`'s own check macro — generated for an ordinary, uninherited-from class, consulted
    // by anyone naming it as an `inherits = ..` target (see the `prelude` construction above).
    // Skipped for `sealed` itself: nothing can ever legally reach a `#[sealed]` class's own check
    // macro (there is no legal inheritor left to consult it), so generating it would only risk
    // colliding with some other same-crate class sharing this one's bare name for no benefit (the
    // scenario this sidesteps — e.g. a `#[sealed]` DSL-facing wrapper sharing a bare name with the
    // unsealed native leaf it composes, see `inherit_macros`'s own comment). An illegal attempt to
    // inherit a `#[sealed]` class anyway simply fails with "macro not found" (E0433) instead of
    // this macro's own friendlier message — an acceptable trade for never risking a same-crate
    // name collision. `no_ancestor_forward` classes (e.g. `NativeTabView`) are NOT skipped here —
    // unlike `sealed`, they remain perfectly legal to inherit from (`TabView` does), so their check
    // macro must exist for that inheritor's `prelude` to call.
    let sealed_ident = sealed_check_ident(&bare_name);
    let sealed_check_macro = if args.sealed || !owns_inherit_macros {
        TokenStream2::new()
    } else {
        quote! {
            #[doc(hidden)]
            #[macro_export]
            macro_rules! #sealed_ident {
                () => {};
            }
        }
    };

    // See `macro_reexport_mod_ident`'s own doc comment: a shared wrapper module (same gate as
    // `inherit_macros`/`sealed_check_macro` above, since all three are only ever present or absent
    // together) giving every macro this class just generated a second, path-addressable home at
    // `<this class's own module>::__elwindui_macros_of_<ClassName>::<macro name>` — reachable
    // through whatever re-export chain (including a merged/multi-crate one) makes this class's own
    // `inherits =`/`struct_only =` type path valid, without colliding with `#[macro_export]`'s own
    // unconditional crate-root placement even when this class is declared directly at that root.
    let macro_reexports = if args.sealed || !owns_inherit_macros {
        TokenStream2::new()
    } else {
        let entry_ident = inherit_macro_ident(&bare_name);
        let skip_ident = inherit_macro_skip_ident(&bare_name);
        let classify_ident = inherit_macro_classify_ident(&bare_name);
        let mod_ident = macro_reexport_mod_ident(&bare_name);
        let own_ext_alias_ident_val = own_ext_alias_ident();
        let policy_ident = own_ext_policy_ident(&bare_name);
        let bound_trait_ident = own_ext_bound_trait_ident(&bare_name);
        // Issue #128 remediation (review finding A1): an ordinary/root class also owns a
        // class-interface bridge for its own generated `{ClassName}Ext` — not just `trait_only`
        // interfaces — so a `struct_only` implementor of *this* class's own interface is equally
        // transparent for override purposes. `struct_only` itself never owns a new bridge (§3.2);
        // it only ever re-exports the one belonging to the interface it implements (below).
        let (
            own_bridge_macro,
            own_bridge_wrapper_mod,
            bridge_reexport,
            own_ext_alias,
            interface_reflexive_accessor,
            own_ext_bound_trait,
            own_ext_bound_trait_reexport,
            own_ext_policy_macro,
        ) = match &args.struct_only {
            None => {
                // PR #164 final remediation round (finding C2): whether this class's own bridge
                // needs the `has_matching_base = true` arm's `as_ui_element` forwarding member at
                // all — the concrete root type it forwards to is no longer baked in here (a
                // proc-macro has no way to name its own module path for a `#[macro_export]`'d
                // macro body invoked from arbitrary other crates/modules); it instead travels as
                // a macro parameter supplied fresh by the `struct_only` caller itself, which
                // already has it (its own `inherits = ..`) — see
                // `build_class_interface_bridge_macro`'s own `is_root_mode` parameter doc comment.
                let bridge_macro = build_class_interface_bridge_macro(
                    &bare_name,
                    &dyn_accessor_ident(&bare_name),
                    &overridable_names,
                    is_root_mode,
                );
                let bridge_ident = class_interface_bridge_ident(&bare_name);
                // Issue #128 remediation (review finding A1): an ordinary/root class's own bridge
                // is re-exported through `class_interface_wrapper_mod_ident` — the *same* naming
                // scheme `expand_trait_only` already uses for a `trait_only` interface's own bridge
                // (not `macro_reexport_mod_ident`, which is a different module reserved for this
                // class's entry/skip/classify trio; `class_interface_bridge_path`'s cross-crate
                // branch only ever looks in the former). Mirrors `expand_trait_only`'s
                // `inherit_macros_and_bridge` construction verbatim.
                let own_bridge_wrapper_mod_ident = class_interface_wrapper_mod_ident(&bare_name);
                let own_bridge_wrapper_mod = quote! {
                    #[doc(hidden)]
                    #[allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]
                    pub mod #own_bridge_wrapper_mod_ident {
                        pub use crate::#bridge_ident;
                    }
                };
                let own_ext_alias =
                    quote! { pub use super::#ext_ident as #own_ext_alias_ident_val; };
                // PR #164 final remediation round (finding A5): the `__ElwindUIOwnExtBound`
                // wrapper trait (proc-macro-time-resolvable supertrait-bound position) and the
                // `__elwindui_apply_own_ext_*!` policy macro (macro-expansion-time-resolvable
                // dispatch-routing position) — see `own_ext_bound_trait_ident`/`own_ext_policy_ident`
                // for the full reasoning on why both are needed and why neither alone suffices.
                // Ordinary/root's own trait genuinely shares this class's own generics, so both
                // unconditionally apply/echo whatever generic arguments a descendant's own `inherits
                // = ..` argument supplies.
                let own_ext_target_trait = quote! { #ext_ident #ty_generics };
                let own_ext_blanket_impl = own_ext_bound_blanket_impl(
                    &item.generics,
                    &bound_trait_ident,
                    &quote! { #ty_generics },
                    &own_ext_target_trait,
                );
                let own_ext_bound_trait = quote! {
                    #[doc(hidden)]
                    pub trait #bound_trait_ident #impl_generics : #ext_ident #ty_generics #where_clause {}
                    #[doc(hidden)]
                    #own_ext_blanket_impl
                };
                let own_ext_bound_trait_reexport_val =
                    quote! { pub use super::#bound_trait_ident; };
                let own_ext_policy_macro = quote! {
                    #[doc(hidden)]
                    #[macro_export]
                    #[allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]
                    macro_rules! #policy_ident {
                        // Immediate-hop form (`expand_impl`'s own prelude): a fixed continuation
                        // macro, already carrying every other piece of context it needs, receives
                        // only the resolved trailing generic-argument tokens.
                        ($cont:path; [$($args:tt)*]) => {
                            $cont!( [ $($args)* ] );
                        };
                        // PR #164 final remediation round (finding A6): recursive-hop form
                        // (`build_inherit_macros`'s own generated classify/skip macros, reused for
                        // *every* future descendant depth) — `$SubType`/the trailing override tokens
                        // are only known once some real descendant chain actually reaches this hop,
                        // so they travel alongside `$cont` here instead of being pre-bound into a
                        // per-invocation continuation macro (which a *second* future descendant
                        // reaching the same hop would collide on redefining).
                        ($cont:path, $SubType:ty; [$($args:tt)*]; $($unmatched:tt)*) => {
                            $cont!( $SubType, [ $($args)* ]; $($unmatched)* );
                        };
                    }
                };
                (
                    Some(bridge_macro),
                    Some(own_bridge_wrapper_mod),
                    None,
                    Some(own_ext_alias),
                    None,
                    Some(own_ext_bound_trait),
                    Some(own_ext_bound_trait_reexport_val),
                    Some(own_ext_policy_macro),
                )
            }
            Some(existing) if args.no_ancestor_forward => {
                (None, None, None, None, None, None, None, None)
            }
            Some(existing) => {
                // Issue #128: a forwardable `struct_only` implementor also re-exports the *real*
                // class-interface-bridge macro (defined and `#[macro_export]`'d by the interface it
                // implements, possibly in a different crate) into its own wrapper module, under the
                // same bridge ident, using a path computed reliably from its own `existing` (no
                // same-crate registry lookup involved) — this is what lets any descendant reach it
                // through the exact same, already-reliable wrapper-module route
                // (`ancestor_bridge_path`) it already uses for this ancestor's entry/skip macros,
                // rather than through a resolution that can silently fall back to a wrong prefix when
                // this ancestor's own same-crate registration hasn't happened yet at the point a
                // *different*, textually-earlier same-crate descendant expands (see
                // `ancestor_bridge_path`'s own doc comment).
                //
                // Issue #128 remediation (review finding A2): also re-exports `existing` itself
                // (the *actual* trait this concrete type implements) under the stable
                // `__ElwindUIOwnExt` alias — never a `{ConcreteName}Ext` naming-convention guess —
                // so nothing downstream ever needs to know or guess this concrete type's own bare
                // name relationship to the interface it implements.
                let class_name = class_name_from_ext_trait_path(existing).expect(
                    "#[class]: internal: struct_only path re-validation should not fail here",
                );
                let real_bridge_path =
                    class_interface_bridge_path(&class_name.to_string(), existing);
                let bridge_ident = class_interface_bridge_ident(&bare_name);
                // Issue #128 remediation (review finding A2): `existing`'s own actual visibility
                // (declared, possibly, in a different crate) is unknown to this proc-macro — capped
                // at this class's own declared `struct_vis` instead, which by this codebase's
                // established `struct_only` convention always matches it (see `StoredClassArgs::
                // struct_vis`'s own doc comment). A `pub use` of a narrower-than-`pub` target here
                // would otherwise be a hard privacy error (E0365), not a silent bug — see this
                // review round's own two `pub(crate) trait_only` regression fixtures
                // (`testsupport.rs`'s `BridgeBase`/`BridgeMulti`) that first exposed this.
                // `build_inherit_macros`'s own `named_accessor` (`has_concrete_type.then(..)`) makes
                // *every* concrete-typed ancestor's entry macro generate `self.base.as_<bare_name>()`
                // on whatever `$OwnConcrete` its caller supplies — for an ordinary/root interface
                // reached the *normal* way (a direct `inherits = Interface` descendant), `$OwnConcrete`
                // is `Interface` itself, whose own reflexive accessor (`self` returned) already
                // exists. Reached via *this* bridge instead, `@inherit_through_struct_only` forwards
                // `$OwnConcrete` as *this* `struct_only` class's own concrete type instead (the only
                // type that's actually real at this hop) — so *this* class needs the identical
                // reflexive accessor too, under the *interface's* bare name (not its own — that one's
                // already supplied by `entry_override`, above), or `self.base.as_<interface>()` has no
                // implementation to call. Never an issue for a `trait_only` interface (no concrete
                // type of its own at all, so `has_concrete_type` is `false` there and this accessor is
                // never generated/called in the first place) — this is purely an A1 (ordinary
                // non-root class as a `struct_only` target) consequence.
                //
                // Deliberately *not* extended to root-mode interfaces (`is_root_mode`, e.g. `UIElement`
                // itself): root-mode interfaces are the one case where the implemented interface
                // requires access to the *declaring root's own concrete type*, not just to `Self` — a
                // root class's own `as_ui_element(&self) -> &Self` is a required trait method whose
                // return type is hard-pinned to the declaring root struct's own concrete type
                // (`pub trait #ext_ident { fn as_ui_element(&self) -> &#impl_name; .. }`,
                // `expand_impl`'s root-mode branch), so this reflexive `interface_named_accessor`
                // (which only ever returns `&Self`) can't be the mechanism that satisfies it. PR #164
                // final remediation round (finding C2) is what makes a *different* concrete
                // implementor valid here instead: a forwardable `struct_only` implementor pairs
                // `struct_only = RootExt` with `inherits = Root`, composing the actual root storage in
                // `self.base` — the generated class-interface bridge's own `@impl_struct_only_members`
                // arm (`has_matching_base = true`) forwards the required `as_ui_element` accessor to
                // that composed base (`self.base.as_ui_element()`), rather than through this accessor.
                // The public accessor's own signature (a concrete, non-`dyn` return type) is
                // unchanged; the composition, not a redesign of `as_ui_element` itself, is what makes
                // this work. A missing or non-matching `inherits` on such a `struct_only` is rejected
                // by the bridge's own `has_matching_base = false` diagnostic arm, not a confusing
                // downstream `E0609`/`E0053`. Runtime coverage exists both same-crate
                // (`elwindui-core::ui::testsupport`'s `BridgeRootBase` -> `BridgeRootConcrete` ->
                // `BridgeRootDerived`) and cross-crate (`crates/elwindui/tests/class_bridge_cross_crate.rs`'s
                // `BridgeFixtureRoot` -> `BridgeFixtureRootConcrete` -> `BridgeFixtureRootDerived`).
                //
                // Skipped when `class_name == bare_name` (the established "shares the interface's own
                // bare name" convention — `Window`/`NativeControl`, per this module's own top doc
                // comment): this class's own unconditional `reflexive_named_accessor` (below,
                // `expand_impl`'s final `out`) already defines the exact same accessor in that case
                // (`pub fn as_<bare_name>(&self) -> &Self`) — redefining it here would be a hard
                // duplicate-definition error, confirmed empirically against every real
                // `NativeControlExt`/`MenuBar`-family implementor.
                let interface_named_accessor = (class_name.to_string() != bare_name)
                    .then(|| {
                        let interface_named_accessor_ident =
                            named_accessor_ident(&class_name.to_string());
                        quote! {
                            #[allow(dead_code)]
                            pub fn #interface_named_accessor_ident(&self) -> &#impl_name #ty_generics { self }
                        }
                    });
                let interface_reflexive_accessor = quote! {
                    impl #impl_generics #impl_name #ty_generics #where_clause {
                        #interface_named_accessor
                    }
                };
                // PR #164 final remediation round (finding A5): same reasoning as the ordinary/root
                // branch's own `own_ext_bound_trait`/`own_ext_policy_macro` — but here, `existing`
                // (never generic — `class_name_from_ext_trait_path`'s own validation) is the
                // supertrait bound target and generic arguments are *never* echoed back, since this
                // class's own generic parameters (if any) belong to its own concrete storage, not to
                // the independently-declared interface it implements.
                let own_ext_blanket_impl = own_ext_bound_blanket_impl(
                    &item.generics,
                    &bound_trait_ident,
                    &quote! { #ty_generics },
                    &quote! { #existing },
                );
                let own_ext_bound_trait = quote! {
                    #[doc(hidden)]
                    #struct_vis trait #bound_trait_ident #impl_generics : #existing #where_clause {}
                    #[doc(hidden)]
                    #own_ext_blanket_impl
                };
                let own_ext_bound_trait_reexport_val =
                    quote! { #struct_vis use super::#bound_trait_ident; };
                let own_ext_policy_macro = quote! {
                    #[doc(hidden)]
                    #[macro_export]
                    #[allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]
                    macro_rules! #policy_ident {
                        // Immediate-hop form — see the ordinary/root branch's own identical arm.
                        ($cont:path; [$($args:tt)*]) => {
                            $cont!( [] );
                        };
                        // Recursive-hop form (finding A6) — see the ordinary/root branch's own
                        // identical arm; discards `$args` here too, same as the immediate-hop form.
                        ($cont:path, $SubType:ty; [$($args:tt)*]; $($unmatched:tt)*) => {
                            $cont!( $SubType, []; $($unmatched)* );
                        };
                    }
                };
                (
                    None,
                    None,
                    Some(quote! { pub use #real_bridge_path as #bridge_ident; }),
                    Some(quote! { #struct_vis use #existing as #own_ext_alias_ident_val; }),
                    Some(interface_reflexive_accessor),
                    Some(own_ext_bound_trait),
                    Some(own_ext_bound_trait_reexport_val),
                    Some(own_ext_policy_macro),
                )
            }
        };
        // PR #164 final remediation round (finding A5): re-exported unconditionally, alongside
        // `__ElwindUIOwnExt`, in the same wrapper module — `None` (`no_ancestor_forward`) leaves
        // this a no-op splice, matching `own_ext_alias`'s own precedent.
        let policy_reexport = own_ext_policy_macro
            .is_some()
            .then(|| quote! { pub use crate::#policy_ident; });
        quote! {
            #own_bridge_macro
            #own_bridge_wrapper_mod
            #interface_reflexive_accessor
            #own_ext_bound_trait
            #own_ext_policy_macro
            #[doc(hidden)]
            #[allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]
            pub mod #mod_ident {
                pub use crate::#entry_ident;
                pub use crate::#skip_ident;
                pub use crate::#classify_ident;
                pub use crate::#sealed_ident;
                #bridge_reexport
                #own_ext_alias
                #policy_reexport
                #own_ext_bound_trait_reexport
            }
        }
    };

    let out = quote! {
        #(#prelude)*
        #sealed_check_macro
        #trait_decl
        #trait_impl
        #ctor_block
        #reflexive_named_accessor
        #inherit_macros
        #macro_reexports
    };
    if std::env::var("ELWINDUI_CLASS_DEBUG").is_ok() {
        eprintln!("=== #[class] impl {class_name} expansion ===\n{out}\n===");
    }
    out
}

#[cfg(test)]
mod rust_analyzer_shadow_tests {
    use super::*;

    // `expand_impl`'s `class_arg_store` lookup fails whenever no paired `struct ClassName` has
    // expanded first — exactly the scenario a bare `#[elwindui_macros::class]` impl with no prior
    // `store_class_args` call reproduces directly, without needing a real multi-invocation macro
    // expansion pass. Only a syntactic smoke test (the workspace's own ~25 real `#[class]`-managed
    // types are the semantic coverage for the shadow's shape, verified with `RUSTFLAGS="--cfg
    // rust_analyzer" cargo check --workspace` per docs/specs/macro_class_spec.md §15) — this
    // module has no proc-macro test harness capable of actually running the resulting tokens
    // through rustc.
    #[test]
    fn shadow_output_is_syntactically_valid_when_struct_lookup_fails() {
        let item: syn::ItemImpl = syn::parse_quote! {
            impl LonelyClass {
                #[inherent]
                fn helper_priv(&self) -> i32 { 42 }
                fn set_value(&self, v: i32) { let _ = v; }
                fn construct(padding: Option<f32>, name: String) -> Self { Self }
            }
        };
        let out = expand_impl(ClassArgs::default(), item, true);
        syn::parse2::<syn::File>(out)
            .expect("rust-analyzer shadow output must be valid Rust syntax");
    }
}

/// Guide §12's test matrix for `__self_weak`/`on_constructed`/mandatory-`construct`/banned-hand-`new`,
/// adapted to this module's actual entry points. Drives the real, top-level `expand` (not
/// `expand_impl` directly) for both the `struct` and `impl` invocations of each fixture — exactly the
/// two-invocation sequence a real `#[elwindui_macros::class]` use site produces, since the mandatory-
/// `construct`/`__self_weak` machinery under test lives in `expand_impl` but depends on
/// `class_arg_store`/`same_crate_classes` state only `expand`'s own `struct` branch populates. Each
/// fixture uses a unique type name (`class_arg_store`/`same_crate_classes` are process-global, and
/// `cargo test` runs these concurrently by default) — never reuse a name across fixtures below.
#[cfg(test)]
mod self_weak_on_constructed_tests {
    use super::*;

    /// Runs `expand` for `struct_item` then for `impl_item` (bare attr, matching real
    /// `#[elwindui_macros::class]` usage on the `impl` side) and returns `(struct output, impl
    /// output)`.
    fn expand_pair(
        struct_attr: TokenStream2,
        struct_item: TokenStream2,
        impl_item: TokenStream2,
    ) -> (TokenStream2, TokenStream2) {
        let struct_out = expand(struct_attr, struct_item);
        let impl_out = expand(TokenStream2::new(), impl_item);
        (struct_out, impl_out)
    }

    fn is_compile_error_containing(ts: &TokenStream2, needle: &str) -> bool {
        let s = ts.to_string();
        s.contains("compile_error") && s.contains(needle)
    }

    // A *successful* expansion always embeds a `compile_error!` literal too (inside the always-
    // generated `__elwindui_inherit_*_terminal!` check macro's own body, unconditionally present,
    // never itself triggered) — so a bare `s.contains("compile_error")` can't distinguish a real
    // top-level error from a normal, successful expansion. `expand_impl`'s own error paths instead
    // return *only* the `compile_error!` call (nothing else), discarding every other generated item
    // — so a real error is always the very first token in the output.
    fn is_real_compile_error(ts: &TokenStream2) -> bool {
        ts.to_string().trim_start().starts_with("compile_error")
    }

    // 12.16 (adapted): a class with no `construct` at all is a compile error, with no implicit
    // default `construct` synthesized.
    #[test]
    fn construct_missing_is_compile_error() {
        let (_struct_out, impl_out) = expand_pair(
            quote! {},
            quote! { pub struct SelfWeakTestMissingCtor { text: String } },
            quote! {
                impl SelfWeakTestMissingCtor {
                    fn text(&self) -> &str { &self.text }
                }
            },
        );
        assert!(
            is_compile_error_containing(&impl_out, "does not define `construct"),
            "expected a missing-`construct` compile_error, got: {impl_out}"
        );
    }

    // 12.17: more than one qualifying `construct` is a compile error.
    #[test]
    fn construct_duplicate_is_compile_error() {
        let (_struct_out, impl_out) = expand_pair(
            quote! {},
            quote! { pub struct SelfWeakTestDupCtor {} },
            quote! {
                impl SelfWeakTestDupCtor {
                    fn construct() -> Self { Self {} }
                    fn construct(_x: i32) -> Self { Self {} }
                }
            },
        );
        assert!(
            is_compile_error_containing(&impl_out, "multiple `construct`"),
            "expected a duplicate-`construct` compile_error, got: {impl_out}"
        );
    }

    // 12.18: a `construct` with the wrong receiver/return type is rejected.
    #[test]
    fn construct_wrong_return_type_is_compile_error() {
        let (_struct_out, impl_out) = expand_pair(
            quote! {},
            quote! { pub struct SelfWeakTestBadReturn {} },
            quote! {
                impl SelfWeakTestBadReturn {
                    fn construct() -> std::rc::Rc<Self> { std::rc::Rc::new(Self {}) }
                }
            },
        );
        assert!(
            is_compile_error_containing(&impl_out, "`construct` must return `Self`"),
            "expected a wrong-return-type compile_error, got: {impl_out}"
        );
    }

    // Confirmed design decision: hand-writing `new` is unconditionally rejected — `#[class]` always
    // derives it from `construct`.
    #[test]
    fn hand_written_new_is_compile_error() {
        let (_struct_out, impl_out) = expand_pair(
            quote! {},
            quote! { pub struct SelfWeakTestHandNew {} },
            quote! {
                impl SelfWeakTestHandNew {
                    fn construct() -> Self { Self {} }
                    fn new() -> std::rc::Rc<Self> { std::rc::Rc::new(Self::construct()) }
                }
            },
        );
        assert!(
            is_compile_error_containing(&impl_out, "do not define `new` by hand"),
            "expected a hand-written-`new` compile_error, got: {impl_out}"
        );
    }

    // 12.8 (parameter form): `__self_weak` is reserved and can't be used as a `construct` parameter
    // name.
    #[test]
    fn self_weak_reserved_as_param_is_compile_error() {
        let (_struct_out, impl_out) = expand_pair(
            quote! {},
            quote! { pub struct SelfWeakTestReservedParam {} },
            quote! {
                impl SelfWeakTestReservedParam {
                    fn construct(__self_weak: usize) -> Self { Self {} }
                }
            },
        );
        assert!(
            is_compile_error_containing(&impl_out, "reserved by the #[class] macro"),
            "expected a reserved-identifier compile_error, got: {impl_out}"
        );
    }

    // 12.8 (let-binding form): same reservation, this time as a local binding inside `construct`.
    #[test]
    fn self_weak_reserved_as_let_binding_is_compile_error() {
        let (_struct_out, impl_out) = expand_pair(
            quote! {},
            quote! { pub struct SelfWeakTestReservedLet {} },
            quote! {
                impl SelfWeakTestReservedLet {
                    fn construct() -> Self {
                        let __self_weak = 1;
                        let _ = __self_weak;
                        Self {}
                    }
                }
            },
        );
        assert!(
            is_compile_error_containing(&impl_out, "reserved by the #[class] macro"),
            "expected a reserved-identifier compile_error, got: {impl_out}"
        );
    }

    // 12.14: every listed invalid `on_constructed` signature is rejected with the guide's own
    // suggested message.
    #[test]
    fn on_constructed_bad_signatures_are_compile_errors() {
        let bad_signatures: &[TokenStream2] = &[
            quote! { fn on_constructed(&mut self) {} },
            quote! { fn on_constructed(&self) -> bool { true } },
            quote! { async fn on_constructed(&self) {} },
            quote! { fn on_constructed(&self) -> Result<(), ()> { Ok(()) } },
            quote! { fn on_constructed(&self, extra: i32) { let _ = extra; } },
        ];
        for (i, sig) in bad_signatures.iter().enumerate() {
            let struct_ident = format_ident!("SelfWeakTestBadHook{i}");
            let (_struct_out, impl_out) = expand_pair(
                quote! {},
                quote! { pub struct #struct_ident {} },
                quote! {
                    impl #struct_ident {
                        fn construct() -> Self { Self {} }
                        #sig
                    }
                },
            );
            assert!(
                is_compile_error_containing(
                    &impl_out,
                    "`on_constructed` must have the signature `fn on_constructed(&self)`"
                ),
                "signature #{i} ({sig}) should have been rejected, got: {impl_out}"
            );
        }
    }

    // 12.11: a class with no `on_constructed` still compiles — `new()`/the hook chain must not
    // require one.
    #[test]
    fn on_constructed_is_optional() {
        let (_struct_out, impl_out) = expand_pair(
            quote! {},
            quote! { pub struct SelfWeakTestNoHook { value: i32 } },
            quote! {
                impl SelfWeakTestNoHook {
                    fn construct() -> Self { Self { value: 10 } }
                }
            },
        );
        assert!(
            !is_real_compile_error(&impl_out),
            "a class with no on_constructed must not error: {impl_out}"
        );
        syn::parse2::<syn::File>(impl_out.clone())
            .unwrap_or_else(|e| panic!("expansion must be valid Rust syntax: {e}\n{impl_out}"));
    }

    // §3.1/§3.2: `construct` is renamed to the hidden `__class_construct`, gains a leading
    // `__class_self_weak: Weak<dyn SelfWeakTestRenamedExt>` parameter (this class's own Ext trait —
    // no root-type propagation, per the confirmed design), and the auto-generated `new()` is built on
    // `Rc::new_cyclic`, running the hook chain before returning.
    #[test]
    fn construct_is_renamed_and_new_uses_rc_new_cyclic() {
        let (_struct_out, impl_out) = expand_pair(
            quote! {},
            quote! { pub struct SelfWeakTestRenamed { value: i32 } },
            quote! {
                impl SelfWeakTestRenamed {
                    fn construct(value: i32) -> Self { Self { value } }
                }
            },
        );
        let s = impl_out.to_string();
        assert!(!is_real_compile_error(&impl_out), "unexpected error: {s}");
        assert!(
            s.contains("__class_construct"),
            "construct should be renamed to __class_construct: {s}"
        );
        assert!(
            s.contains("__class_self_weak")
                && s.contains("Weak")
                && s.contains("SelfWeakTestRenamedExt"),
            "hidden param should be typed against this class's own Ext trait: {s}"
        );
        assert!(
            s.contains("new_cyclic"),
            "auto-generated new() should use Rc::new_cyclic: {s}"
        );
        assert!(
            s.contains("__elwindui_run_on_constructed"),
            "auto-generated new() should run the hook chain: {s}"
        );
        syn::parse2::<syn::File>(impl_out.clone())
            .unwrap_or_else(|e| panic!("expansion must be valid Rust syntax: {e}\n{s}"));
    }

    // §3.3/§6.1/§6.2: an ancestor `construct()` call inside this class's own `construct` body is
    // rewritten to call `__class_construct` with the self-weak forwarded, while an unrelated
    // `Type::construct()` call is left completely untouched.
    #[test]
    fn ancestor_construct_call_is_rewritten_and_unrelated_calls_are_not() {
        let (_parent_struct_out, _parent_impl_out) = expand_pair(
            quote! {},
            quote! { pub struct SelfWeakTestParent { value: i32 } },
            quote! {
                impl SelfWeakTestParent {
                    fn construct() -> Self { Self { value: 0 } }
                }
            },
        );
        let (_child_struct_out, child_impl_out) = expand_pair(
            quote! { inherits = crate::self_weak_on_constructed_tests::SelfWeakTestParent },
            quote! { pub struct SelfWeakTestChild { extra: i32 } },
            quote! {
                impl SelfWeakTestChild {
                    fn construct() -> Self {
                        let unrelated = SelfWeakTestUnrelatedFactory::construct();
                        Self { base: SelfWeakTestParent::construct(), extra: unrelated }
                    }
                }
            },
        );
        let s = child_impl_out.to_string();
        assert!(
            !is_real_compile_error(&child_impl_out),
            "unexpected error: {s}"
        );
        assert!(
            s.contains("SelfWeakTestParent :: __class_construct")
                || s.contains("SelfWeakTestParent::__class_construct"),
            "ancestor construct() call should be rewritten to __class_construct: {s}"
        );
        assert!(
            s.contains("SelfWeakTestUnrelatedFactory :: construct")
                || s.contains("SelfWeakTestUnrelatedFactory::construct"),
            "an unrelated Type::construct() call must not be rewritten: {s}"
        );
        assert!(
            !s.contains("SelfWeakTestUnrelatedFactory :: __class_construct")
                && !s.contains("SelfWeakTestUnrelatedFactory::__class_construct"),
            "an unrelated Type::construct() call must not be rewritten: {s}"
        );
    }

    // 12.10/12.13: `on_constructed` is detected, kept out of the class's own `{ClassName}Ext` trait
    // (guide §1d — it's purely an internal lifecycle hook), and chained base-first via
    // `__elwindui_run_on_constructed`.
    #[test]
    fn on_constructed_is_chained_base_first_and_excluded_from_own_trait() {
        let (_parent_struct_out, parent_impl_out) = expand_pair(
            quote! {},
            quote! { pub struct SelfWeakTestHookParent { value: i32 } },
            quote! {
                impl SelfWeakTestHookParent {
                    fn construct() -> Self { Self { value: 0 } }
                    fn on_constructed(&self) {}
                }
            },
        );
        let parent_s = parent_impl_out.to_string();
        assert!(
            !is_real_compile_error(&parent_impl_out),
            "unexpected error: {parent_s}"
        );
        // The trait declaration (`pub trait SelfWeakTestHookParentExt ...`) must not itself declare
        // `on_constructed` as one of its own (dyn-dispatched) methods.
        let trait_decl_start = parent_s
            .find("pub trait SelfWeakTestHookParentExt")
            .expect("trait declaration must exist");
        let trait_decl_slice = &parent_s[trait_decl_start..];
        let trait_decl_end = trait_decl_slice
            .find("impl ")
            .unwrap_or(trait_decl_slice.len());
        assert!(
            !trait_decl_slice[..trait_decl_end].contains("on_constructed"),
            "on_constructed must not become a trait method: {}",
            &trait_decl_slice[..trait_decl_end]
        );
        assert!(
            parent_s.contains("fn on_constructed (& self)")
                || parent_s.contains("fn on_constructed(&self)"),
            "on_constructed's own body must still be emitted as a plain method: {parent_s}"
        );
    }

    // A root class (no `inherits`) gets its own `into_<name>_node` upcast as a default method on
    // its own `{Name}Ext` trait declaration — `Rc<Self> -> Rc<dyn {Name}Ext>`, gated `Self: Sized`
    // so the trait stays object-safe wherever it's already used as `dyn {Name}Ext`.
    #[test]
    fn into_node_upcast_generated_for_root_class() {
        let (_struct_out, impl_out) = expand_pair(
            quote! {},
            quote! { pub struct IntoNodeTestRoot { value: i32 } },
            quote! {
                impl IntoNodeTestRoot {
                    fn construct() -> Self { Self { value: 0 } }
                }
            },
        );
        let s = impl_out.to_string();
        assert!(!is_real_compile_error(&impl_out), "unexpected error: {s}");
        let trait_start = s
            .find("pub trait IntoNodeTestRootExt")
            .expect("trait declaration must exist");
        let trait_slice = &s[trait_start..];
        assert!(
            trait_slice.contains("into_into_node_test_root_node"),
            "root class's own trait should declare its into_<name>_node upcast: {trait_slice}"
        );
        assert!(
            trait_slice.contains("self : std :: rc :: Rc < Self >")
                || trait_slice.contains("self: std::rc::Rc<Self>"),
            "into_node upcast should take self: Rc<Self>: {trait_slice}"
        );
        assert!(
            trait_slice.contains("Self : Sized") || trait_slice.contains("Self: Sized"),
            "into_node upcast should be Self: Sized-gated to preserve object safety: {trait_slice}"
        );
        syn::parse2::<syn::File>(impl_out.clone())
            .unwrap_or_else(|e| panic!("expansion must be valid Rust syntax: {e}\n{s}"));
    }

    // An ordinary (non-root) class gets its own `into_<name>_node` upcast too, named after
    // *itself* — not the root's — one such method per ancestor level, each declared once on that
    // ancestor's own trait and inherited down through Rust's ordinary default-method/supertrait
    // mechanism (no per-hop generation needed in `build_inherit_macros`).
    #[test]
    fn into_node_upcast_generated_for_ordinary_class_named_after_itself() {
        let _ = expand_pair(
            quote! {},
            quote! { pub struct IntoNodeTestParent { value: i32 } },
            quote! {
                impl IntoNodeTestParent {
                    fn construct() -> Self { Self { value: 0 } }
                }
            },
        );
        let (_child_struct_out, child_impl_out) = expand_pair(
            quote! { inherits = crate::class::self_weak_on_constructed_tests::IntoNodeTestParent },
            quote! { pub struct IntoNodeTestChild { extra: i32 } },
            quote! {
                impl IntoNodeTestChild {
                    fn construct() -> Self { Self { extra: 0 } }
                }
            },
        );
        let s = child_impl_out.to_string();
        assert!(
            !is_real_compile_error(&child_impl_out),
            "unexpected error: {s}"
        );
        let trait_start = s
            .find("pub trait IntoNodeTestChildExt")
            .expect("trait declaration must exist");
        let trait_slice = &s[trait_start..];
        assert!(
            trait_slice.contains("into_into_node_test_child_node"),
            "child class's own trait should declare its own into_<name>_node upcast (not the \
             parent's): {trait_slice}"
        );
        assert!(
            !trait_slice.contains("into_into_node_test_parent_node"),
            "child's own trait must not redeclare the parent's upcast method (inherited via the \
             supertrait bound instead): {trait_slice}"
        );
    }
}

/// Issue #128 PR #164 review remediation, A4/T12: focused macro-level regression coverage for the
/// remediation itself, living directly in this crate rather than only in downstream fixture crates
/// (`elwindui-core`'s `testsupport.rs`, `elwindui-bridge-fixture-*`) — reuses
/// `self_weak_on_constructed_tests`'s own `expand_pair`/`is_real_compile_error` harness style
/// (duplicated locally rather than shared, matching that module's own precedent of not exposing test
/// helpers across `#[cfg(test)]` module boundaries).
#[cfg(test)]
mod class_interface_bridge_tests {
    use super::*;

    fn expand_pair(
        struct_attr: TokenStream2,
        struct_item: TokenStream2,
        impl_item: TokenStream2,
    ) -> (TokenStream2, TokenStream2) {
        let struct_out = expand(struct_attr, struct_item);
        let impl_out = expand(TokenStream2::new(), impl_item);
        (struct_out, impl_out)
    }

    fn is_real_compile_error(ts: &TokenStream2) -> bool {
        ts.to_string().trim_start().starts_with("compile_error")
    }

    // T12: a `trait_only` interface generates its own `__elwindui_class_interface_*!` bridge macro.
    #[test]
    fn bridge_macro_generated_for_trait_only_interface() {
        let out = expand(
            quote! { trait_only },
            quote! {
                pub trait BridgeTestTraitOnlyIface {
                    #[overridable]
                    fn value(&self) -> i32;
                }
            },
        );
        assert!(!is_real_compile_error(&out), "unexpected error: {out}");
        let s = out.to_string();
        assert!(
            s.contains("__elwindui_class_interface_BridgeTestTraitOnlyIface"),
            "trait_only interface must generate its own class-interface bridge macro: {s}"
        );
    }

    // T12/A1: an *ordinary* (non-root) class also generates its own bridge macro, not just
    // `trait_only` interfaces.
    #[test]
    fn bridge_macro_generated_for_ordinary_interface() {
        let (_struct_out, impl_out) = expand_pair(
            quote! {},
            quote! { pub struct BridgeTestOrdinaryIface { value: i32 } },
            quote! {
                impl BridgeTestOrdinaryIface {
                    #[overridable]
                    fn value(&self) -> i32 { self.value }
                    fn construct() -> Self { Self { value: 1 } }
                }
            },
        );
        assert!(
            !is_real_compile_error(&impl_out),
            "unexpected error: {impl_out}"
        );
        let s = impl_out.to_string();
        assert!(
            s.contains("__elwindui_class_interface_BridgeTestOrdinaryIface"),
            "an ordinary (non-root) class must generate its own class-interface bridge macro: {s}"
        );
    }

    // T3/T12/A1: a *root* class (no `inherits`, e.g. `UIElement` itself) also generates its own
    // bridge macro. Macro-expansion-level coverage only here — PR #164 final remediation round
    // (finding C2) superseded the earlier conclusion that a real runtime `struct_only` implementor
    // of a root interface is architecturally impossible: it *is* supported, by requiring the
    // `struct_only` implementor to also `inherits = <the same root class>` (composing the real root
    // storage) — see `crates/elwindui-core/src/ui/testsupport.rs`'s own `BridgeRootBase` ->
    // `BridgeRootConcrete` -> `BridgeRootDerived` fixture (T9, same-crate) and
    // `crates/elwindui/tests/class_bridge_cross_crate.rs`'s `BridgeFixtureRoot` ->
    // `BridgeFixtureRootConcrete` -> `BridgeFixtureRootDerived` fixture (T10, cross-crate) for
    // actual runtime coverage.
    #[test]
    fn bridge_macro_generated_for_root_interface() {
        let (_struct_out, impl_out) = expand_pair(
            quote! {},
            quote! { pub struct BridgeTestRootIface { value: i32 } },
            quote! {
                impl BridgeTestRootIface {
                    #[overridable]
                    fn value(&self) -> i32 { self.value }
                    fn construct() -> Self { Self { value: 1 } }
                }
            },
        );
        assert!(
            !is_real_compile_error(&impl_out),
            "unexpected error: {impl_out}"
        );
        let s = impl_out.to_string();
        assert!(
            s.contains("__elwindui_class_interface_BridgeTestRootIface"),
            "a root class must generate its own class-interface bridge macro too: {s}"
        );
        assert!(
            s.contains("struct_only implementing a root class interface must also use"),
            "a root class's own bridge must carry the `has_matching_base = []` diagnostic arm \
             for a struct_only implementor with a non-matching `inherits`: {s}"
        );
    }

    // T11 (finding C2): a `struct_only` implementor whose own `struct_only = ..Ext` target does
    // *not* match its `inherits = ..` argument (`struct_only_matches_direct_parent` computes
    // `false`) must route into the bridge's diagnostic arm (`has_matching_base = []`, the *empty*
    // bracket form — never a populated `[$RootConcrete:path]`) rather than silently reattaching
    // some other path. Whether the *target* interface is actually root-mode is irrelevant here:
    // this class's own generated invocation is purely syntactic (`struct_only`/`inherits` attribute
    // text only, no registry, no knowledge of the target's own shape) — see
    // `struct_only_matches_direct_parent`'s own doc comment.
    #[test]
    fn struct_only_non_matching_base_emits_empty_has_matching_base_arg() {
        let (_struct_out, impl_out) = expand_pair(
            quote! {
                struct_only = some_crate::BridgeTestNonMatchingRootExt,
                inherits = some_crate::SomeUnrelatedBase
            },
            quote! { pub struct BridgeTestNonMatchingRootConcrete {} },
            quote! {
                impl BridgeTestNonMatchingRootConcrete {
                    fn construct() -> Self { Self { base: some_crate::SomeUnrelatedBase::construct() } }
                }
            },
        );
        assert!(
            !is_real_compile_error(&impl_out),
            "unexpected error: {impl_out}"
        );
        let s = impl_out.to_string();
        assert!(
            s.contains("has_matching_base = []"),
            "a non-matching struct_only/inherits pair must emit the empty `has_matching_base = []` \
             form (never a populated bracket, which would wrongly claim a matching base): {s}"
        );
        assert!(
            !s.contains("has_matching_base = [some_crate"),
            "must not reattach an unrelated concrete path when the base does not match: {s}"
        );
    }

    // T12/A3: a generic `struct_only<T>` type preserves its own `impl_generics`/`where_clause` on
    // the outer `impl` header the bridge's members-only arm is spliced into (runtime-level coverage
    // already exists in `elwindui-core::ui::testsupport`'s `BridgeGenericConcrete`; this is the
    // focused macro-level companion).
    #[test]
    fn generic_struct_only_preserves_impl_generics_in_expansion() {
        let (_struct_out, impl_out) = expand_pair(
            quote! { struct_only = some_crate::SomeIfaceExt },
            quote! {
                pub struct BridgeTestGenericStructOnly<T> where T: Clone + 'static { source: T }
            },
            quote! {
                impl<T> BridgeTestGenericStructOnly<T> where T: Clone + 'static {
                    fn value(&self) -> i32 { 1 }
                    fn construct(source: T) -> Self { Self { source } }
                }
            },
        );
        assert!(
            !is_real_compile_error(&impl_out),
            "unexpected error: {impl_out}"
        );
        let s = impl_out.to_string();
        let impl_start = s
            .find("impl < T")
            .expect("outer impl header for the generic struct_only type must exist");
        let header = &s[impl_start..impl_start + 220.min(s.len() - impl_start)];
        assert!(
            header.contains("SomeIfaceExt for BridgeTestGenericStructOnly < T >")
                && header.contains("where T : Clone"),
            "the outer impl header must keep its own impl_generics/ty_generics/where_clause \
             (review finding A3), not lose them to the bridge's opaque macro tokens: {header}"
        );
    }

    // T21 (finding A6): a class's own generated *recursive* "forward past me" metadata (consulted
    // by future descendants reaching this hop, not the immediate `inherits = ..` edge A5 already
    // covers) calls the next ancestor's own `__elwindui_apply_own_ext_*!` policy macro at real
    // macro-expansion time, through a fixed, once-per-class continuation macro — never a
    // pre-resolved semantic trait baked in as a literal token (the former `ancestor_own_trait`
    // same-crate-registry heuristic, which could only ever get the immediate hop right). The
    // generic argument (`SomeArg`) travels through unresolved too — present in the policy macro's
    // own bracketed argument, not pre-applied to any trait path here.
    #[test]
    fn recursive_forwarding_metadata_uses_the_next_ancestors_own_policy_macro() {
        let (_struct_out, impl_out) = expand_pair(
            quote! { inherits = some_crate::SomeGenericParent<SomeArg> },
            quote! { pub struct BridgeTestRecursiveOrdinary {} },
            quote! {
                impl BridgeTestRecursiveOrdinary {
                    fn construct() -> Self {
                        Self { base: some_crate::SomeGenericParent::construct() }
                    }
                }
            },
        );
        assert!(
            !is_real_compile_error(&impl_out),
            "unexpected error: {impl_out}"
        );
        let s = impl_out.to_string();
        assert!(
            s.contains("__elwindui_continue_recursive_inherits_of_BridgeTestRecursiveOrdinary"),
            "recursive forwarding must generate a fixed, once-per-class continuation macro: {s}"
        );
        assert!(
            s.contains("__elwindui_apply_own_ext_SomeGenericParent"),
            "recursive forwarding must call the next ancestor's own policy macro, not a \
             pre-resolved trait: {s}"
        );
        let policy_call_start = s
            .find("__elwindui_apply_own_ext_SomeGenericParent !")
            .expect("policy macro invocation must exist");
        let window = &s[policy_call_start..(policy_call_start + 400).min(s.len())];
        assert!(
            window.contains("SomeArg"),
            "the concrete generic argument must travel through, unresolved, to the next \
             ancestor's own policy macro (never pre-applied to a trait path here): {window}"
        );
    }

    // T12/A2: a `struct_only` implementor whose own bare name differs from the interface it
    // implements gets a `__ElwindUIOwnExt` alias pointing at the *real* interface path, never a
    // `{ConcreteName}Ext` naming-convention guess.
    #[test]
    fn different_concrete_and_interface_name_own_trait_metadata() {
        let (_struct_out, impl_out) = expand_pair(
            quote! { struct_only = some_crate::OddlyNamedIfaceExt },
            quote! { pub struct BridgeTestDifferentNameConcrete {} },
            quote! {
                impl BridgeTestDifferentNameConcrete {
                    fn construct() -> Self { Self {} }
                }
            },
        );
        assert!(
            !is_real_compile_error(&impl_out),
            "unexpected error: {impl_out}"
        );
        let s = impl_out.to_string();
        assert!(
            s.contains("some_crate :: OddlyNamedIfaceExt as __ElwindUIOwnExt")
                || s.contains("some_crate::OddlyNamedIfaceExt as __ElwindUIOwnExt"),
            "the __ElwindUIOwnExt alias must point at the real `existing` interface path, not a \
             `{{ConcreteName}}Ext` naming-convention guess: {s}"
        );
    }

    // T8/T12: `no_ancestor_forward` bypasses the bridge entirely — no
    // `__elwindui_class_interface_*!` invocation anywhere in a `no_ancestor_forward` struct_only
    // class's own expansion.
    #[test]
    fn no_ancestor_forward_never_invokes_the_bridge() {
        let (_struct_out, impl_out) = expand_pair(
            quote! { struct_only = some_crate::HandWrittenTrait, no_ancestor_forward },
            quote! { pub struct BridgeTestNoForwardConcrete {} },
            quote! {
                impl BridgeTestNoForwardConcrete {
                    fn hand_value(&self) -> i32 { 1 }
                    fn construct() -> Self { Self {} }
                }
            },
        );
        assert!(
            !is_real_compile_error(&impl_out),
            "unexpected error: {impl_out}"
        );
        let s = impl_out.to_string();
        assert!(
            !s.contains("__elwindui_class_interface_"),
            "no_ancestor_forward must never reference the class-interface bridge at all: {s}"
        );
    }

    // T7: the unknown-`#[overrides]`-target diagnostic. A leaf class (no further ancestor) always
    // embeds its own `__elwindui_inherit_*_terminal!` check macro, whose literal body is this exact
    // `compile_error!` — reachable directly from a single `expand()` call without needing to
    // actually re-expand it through a second macro-invocation pass (which this crate's own test
    // harness cannot do — see `rust_analyzer_shadow_tests`'s own doc comment on why no proc-macro
    // integration-test harness exists here).
    #[test]
    fn unknown_override_target_terminal_diagnostic_is_present_in_expansion() {
        let (_struct_out, impl_out) = expand_pair(
            quote! {},
            quote! { pub struct BridgeTestTerminalDiagnostic { value: i32 } },
            quote! {
                impl BridgeTestTerminalDiagnostic {
                    fn construct() -> Self { Self { value: 1 } }
                }
            },
        );
        assert!(
            !is_real_compile_error(&impl_out),
            "unexpected error: {impl_out}"
        );
        let s = impl_out.to_string();
        assert!(
            s.contains("no ancestor declared these methods #[overridable]"),
            "a leaf class must still embed the unknown-override terminal diagnostic: {s}"
        );
    }
}
