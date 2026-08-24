# ControlTemplate runtime and codegen design

Normative contract: [`../../specs/control_template_spec.md`](../../specs/control_template_spec.md)

Tracking: [#83](https://github.com/puchinya/elwindui/issues/83)

## 1. Runtime values

`ControlTemplate<C>`は`Rc<dyn Fn(ControlTemplateContext<C>) -> Rc<dyn UIElementExt>>`をprivateに保持する。
target型は`C: ControlExt + 'static`で静的に制約し、factory entrypoint以降で型消去やdowncastを行わない。

`ControlTemplateContext<C>`はbuild中だけstrong `Rc<C>`を渡す。macro-authored template instanceは
targetを`Weak<C>`として保持し、Controlがtemplate rootをstrong所有してもcycleにならない。

## 2. Template root ownership

`Control`はprivateな`template_root: RefCell<Option<Rc<dyn UIElementExt>>>`を持つ。
rootの更新は一つのinternal methodに集約し、次をatomicな操作として扱う。

1. old rootを`VisualCollection`から除去してVisual parentを解除する。
2. new rootを`VisualCollection`へ追加する。
3. storageを更新し、measureをinvalidateする。

template rootはControlのVisual childであり、logical `parent`は設定しない。
初期版ではmount中に一度設定されるが、replace helper自体はdetach/attach invariantを守る。

`Control` は公開 collection content を所有しない。`Layout` が collection children、
`ContentControl` が単一の logical content を所有する。`Control` の内部 scalar
`#[content(visual_root)]` surface は `set_visual_root(root)` から同じ
`__set_template_root(root)` replacement helper へ委譲される。したがって
`#[component(inherits Control)]` の default body に authored visual root がある場合も、codegen は
型名を特別扱いせず scalar content setter を呼ぶだけでよい。`template_root` は依然として
private であり、ユーザーへ公開する children collection や別の汎用 content slot は追加しない。

## 3. Generated mount path

`#[component(template = key)]`はComponent metadataへtyped key nameを保存する。
generated `mount`/`__build_view`は既存のmount-time Environment解決を維持して次の二分岐を生成する。

```text
resolve own Environment fields
resolve K from __mount_environment
  Some(template) -> template.build(context)
  None           -> existing specialized default body build
set template root
mount descendants / install selected branch wiring
run target wiring and on_mount
```

default body用node storageは既存通り`OnceCell`等でstructに確保してよいが、custom branchではsetせず、
default node construction・event wiring・subscriptionを実行しない。
shape-composed `Control` の default body では、構築済みの単一 authored visual root を
private template-root pathへ接続する。`ContentControl`は内部のbody-presentation metadataで
同じtemplate-root modeへopt-inし、body rootを`__prepare_template_presentation()`後に
`__set_template_root()`へ接続する。componentのuse-site bare childはこのown-body処理とは別に、
継承された`content` setterへlowerされる。collection compositionは `Layout` のみが担当する。
Keyへはsubscribeしないためtemplate choiceはmount lifetime中固定される。

### 3.1 Body-presentation metadata

`#[class]`は内部のbody-presentation capabilityを`__elwindui_props_{Name}!`へ出力する。
`@component_body_presentation { template_block }, { content_block }` queryは、opt-inした
`ContentControl`とそのcross-crate descendantsだけtemplate blockを選び、それ以外はcontent
blockを選ぶ。class/type名を検査するcodegen分岐や公開DSL構文は追加しない。これはcomponent自身の
authored bodyにだけ使い、caller側のbare childは既存の`@children`/scalar・collection loweringを
使い続ける。

## 4. control_template authoring

`#[control_template(target = T)]` frontendはbodyを既存View ASTへlowerし、root要素へcompositionするprivate
template-instance型を生成する。public authoring typeはzero-sized namespaceとして残り、`template()`が
`ControlTemplate<T>`を返す。

private instanceは`Weak<T>` templated parent、body node storage、binding/subscription storageを持つ。
初期buildではcontextのstrong targetからWeakを作り、instanceを同じEnvironmentでmountする。
`templated_parent.foo`はWeak upgrade後のtyped getter callとしてemitする。targetがtemplate rootを所有し、
target drop時にはtemplate instanceとsubscriptionも先に解放されるため、live callback中のupgrade成功をinvariantとする。
subscription callbackはtemplate instanceをWeak captureする。

別のreactive graphは作らず、既存`ObservableExt`/PropertyChanged、dependency collection、
`emit_resync`をowner名`templated_parent`へ適用する。

## 5. ContentControl presentation

`ContentControl`はlogical `content`に加えてpresentation modeを持つ。

- direct mode: raw `ContentControl`互換としてcontentを自身のVisual childにもする。
- template mode: contentのlogical parentだけを自身に設定し、active `ContentPresenter`へ変更を通知する。

template-enabled mountはroot build前にtemplate modeへ切り替える。direct modeで既にVisual配置されたcontentが
あればControlから外すがlogical parentは維持する。

`ContentPresenter`はbackend非依存の`Control`派生builtinであり、presented contentをprivateに保持する。
binding時にContentControlのcontent-change通知へpresenterをWeak captureしたcallbackを登録し、current contentを自身の
`VisualCollection`へ追加する。content changeではold Visual childをremoveし、new childをaddするだけで、
logical parentには触れない。source/presenter双方はWeak callbackとcancel可能な`Subscription`でcycleを避ける。

static template validationにより一つのtemplate instanceにactive presenterは最大一つとなる。

## 6. Validation boundaries

same-crate registryで判定可能なtemplate key、target category、body shapeはfrontendで`syn::Error`にする。
cross-crate targetは次のgenerated Rust constraintsで検証する。

- `T: ControlExt + 'static`
- `K: EnvironmentKey<Value = Option<ControlTemplate<T>>>`
- `templated_parent` getter method resolution
- `ContentPresenter`利用時の`T: ContentControlExt`

`#[id]`、複数/dynamic `ContentPresenter`はAST traversalでrejectする。
既存のcomponent inheritanceを表す`is_template_composition`は
`is_inherited_view_composition`へrenameし、ControlTemplate selectionとは独立させる。

## 7. Lifecycle and performance

custom factoryはmountごとに一回だけ呼ぶ。Environment template keyをsubscribeせず、runtime差し替え用の
allocationやdiffを持たない。default branchはfactory wrapperを経由せず既存generated constructionを直接実行する。

template instanceのon_mountはtarget Controlのon_mountより前に実行する。general detach hookは本Issueで追加せず、
root、subscription、presenter bindingはControl/treeの既存lifetimeとWeak ownershipに従って解放する。
